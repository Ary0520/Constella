# Contracts Specification

> All contracts are written in **Rust**, compiled to **WASM**, deployed on **Soroban**.  
> Base: OpenZeppelin stellar-contracts **v0.8.0** (released 20 October 2026; requires `soroban-sdk` **28.0.0**).  
> Reference: Product doc §4.3, §4.6, §4.9; `architecture/VAULT-DESIGN.md`

---

## Contract Repository Layout

```
constella-contracts/
├── registry/           # Asset registry — the closed universe
├── policy/             # Policy storage and validation
├── vault-strategy/     # Shared strategy vault (SEP-56)
├── vault-factory/      # Deploys private vault instances [Q1 2027]
├── vault-private/      # Per-institution vault (same WASM as strategy) [Q1 2027]
├── nav-oracle/         # NAV computation and circuit breakers
├── keeper/             # Rebalance proposer and TTL extender
├── adapters/
│   ├── blend/          # Blend supply position adapter [Phase 2]
│   ├── defindex/       # DeFindex yield aggregator adapter [Phase 2]
│   ├── benji/          # Franklin Templeton BENJI adapter [Phase 3]
│   ├── spiko/          # Spiko USD + EUR adapter [Phase 3]
│   ├── usdy/           # Ondo USDY adapter [Phase 3]
│   └── ylds/           # Figure YLDS adapter [Phase 3]
└── identity/           # SEP-57 identity verifier hook [Phase 3]
```

---

## Registry Contract

**Purpose:** The on-chain closed universe. The thesis engine and policy contract both query this. An asset that is not in the registry CANNOT appear in any basket.

**Key functions:**
```rust
fn get_asset(env: Env, asset_id: String) -> Option<AssetEntry>
fn list_assets(env: Env, filters: AssetFilter) -> Vec<AssetEntry>
fn add_asset(env: Env, entry: AssetEntry)      // admin multisig only
fn deactivate_asset(env: Env, asset_id: String) // admin multisig only
fn is_registered(env: Env, contract_address: Address) -> bool
```

**AssetEntry struct:**
```rust
pub struct AssetEntry {
    contract_address: Address,
    asset_id: String,            // e.g. "FRANKLIN_BENJI"
    issuer: Symbol,              // e.g. "CIRCLE" for USDC and EURC; required (audit P2)
    sleeve_type: SleeveType,     // cash | usd_gov_fund | eur_sovereign | yield_usd | lending | equity
    liquidity_tier: u8,          // 0=instant, 1=24h, 2=1week, 3=illiquid
    permissionless: bool,
    oracle_sources: Vec<String>,
    eligible_jurisdictions: Vec<String>,   // empty = all
    blocked_jurisdictions: Vec<String>,
    investor_classes_required: Vec<String>, // empty = all
    is_active: bool,
    added_at: u64,
}
```

**Phase 1 registry (permissionless only):**
- `USDC` — Circle USDC on Stellar testnet
- `EURC` — Circle EURC on Stellar testnet
- `BLEND_USDC_SUPPLY` — Blend USDC supply position (via DeFindex)

**Admin:** Registry admin multisig (2-of-N, same as protocol admin)

---

## Policy Contract

**Purpose:** Stores signed policy hashes. Validates that proposed rebalance operations are consistent with the policy. The sole on-chain authority for what a vault is allowed to do.

**Key functions:**
```rust
// Requires vault.require_auth() AND owner.require_auth(); one-time per vault.
fn register_policy(env: Env, vault: Address, owner: Address, policy_hash: BytesN<32>,
                   registry_address: Address, min_liquidity_sleeve_bps: u32, max_single_asset_bps: u32,
                   max_issuer_bps: u32, max_slippage_bps: u32, rebalance_band_bps: u32,
                   sleeves: Vec<SleeveTarget>)
// Requires the stored owner's fresh signature; may amend every cap and sleeve.
fn update_policy(env: Env, vault: Address, new_hash: BytesN<32>, min_liquidity_sleeve_bps: u32,
                 max_single_asset_bps: u32, max_issuer_bps: u32, max_slippage_bps: u32,
                 rebalance_band_bps: u32, new_sleeves: Vec<SleeveTarget>)
fn validate_rebalance(env: Env, vault: Address, proposal: RebalanceProposal) -> ValidationResult
fn get_policy_hash(env: Env, vault: Address) -> BytesN<32>
fn get_policy_config(env: Env, vault: Address) -> Option<PolicyConfig> // includes owner + terms_hash
```

**Signing model (audit P3, 15 Nov 2026):**
- The **policy owner** (user passkey smart account or treasury multisig) signs `register_policy` / `update_policy`. Soroban authorization covers every argument, so one signature binds the canonical-JSON `policy_hash` to the exact on-chain caps and sleeves.
- The **vault** consents by invoking registration itself through `vault.bind_policy(...)`, which also requires the vault admin (who selects the owner). Neither the admin nor the owner can bind alone.
- The **protocol admin cannot register, replace, or amend** a policy. Registration is one-time per vault (`PolicyAlreadyRegistered`), preventing squatting.
- `terms_hash = SHA-256(XDR(registry_address, min_liquidity_sleeve_bps, max_single_asset_bps, max_issuer_bps, max_slippage_bps, rebalance_band_bps, sleeves))` is stored on every write so anyone can verify the enforced terms.

**Validation checks (ALL must pass — any failure rejects the entire proposal):**
1. `policy_hash` matches the vault's registered hash
2. All proposed weights are within configured bands
3. No issuer cap exceeded after rebalance
4. No single asset cap exceeded
5. Liquidity sleeve minimum maintained
6. All assets in proposal are in the registry and active
7. Proposal deadline not exceeded (prevents replays)
8. Slippage per operation within limit — measured in **value**, not token units (audit P1): each leg is `amount × asset_price / 10^token_decimals` using the NAV oracle's per-asset median (numéraire = 1.0); require `min_value_out ≥ value_in × (1 − max_slippage_bps/10000)`. No valid price → proposal invalid (`PriceUnavailable`). The policy contract's oracle is set once in `initialize(admin, oracle)` and has no setter.

**RebalanceProposal struct:**
```rust
pub struct RebalanceProposal {
    vault: Address,
    operations: Vec<TradeOperation>,
    weights_after: Vec<(String, u32)>, // (asset_id, weight_bps)
    deadline: u64,
    proposer: Address,
}

pub struct TradeOperation {
    asset_in: Address,
    asset_out: Address,
    amount_in: i128,
    min_amount_out: i128,  // slippage cap enforcement
    route: TradeRoute,     // ISSUER | SOROSWAP | AQUARIUS | SDEX
}
```

---

## Strategy Vault Contract

**Purpose:** Shared vault for multiple holders. Core of Constella for most users.

**Base:** OpenZeppelin stellar-contracts v0.8.0 SEP-56 vault module.

**Key functions:**
```rust
// SEP-56 required
fn deposit(env: Env, assets: i128, receiver: Address) -> i128   // returns shares
fn redeem(env: Env, shares: i128, receiver: Address, owner: Address) -> i128  // returns assets
fn total_assets(env: Env) -> i128
fn total_supply(env: Env) -> i128
fn convert_to_shares(env: Env, assets: i128) -> i128
fn convert_to_assets(env: Env, shares: i128) -> i128
fn asset(env: Env) -> Address   // the numéraire

// Collateral Passport (view-only)
fn get_nav_conservative(env: Env) -> Option<PriceData>
fn get_nav_deposit(env: Env) -> Option<PriceData>
fn get_redemption_capacity(env: Env) -> RedemptionCapacity
fn get_risk_sheet(env: Env) -> RiskSheet
fn is_eligible_liquidator(env: Env, liquidator: Address) -> bool
fn get_oracle_status(env: Env) -> Vec<OracleSource>
fn get_listing_kit(env: Env) -> ListingKit

// Admin
fn pause(env: Env)             // pause authority (single key — fast)
fn unpause(env: Env)           // admin multisig required
fn upgrade(env: Env, new_wasm: BytesN<32>)  // admin multisig + 48h timelock
```

**VaultState struct:**
```rust
pub struct VaultState {
    numeraire: Address,
    total_shares: i128,
    policy_hash: BytesN<32>,
    policy_version: u32,
    sleeves: Vec<SleeveState>,
    admin: Address,
    pause_authority: Address,
    is_paused: bool,
    last_fee_accrual: u64,
    management_fee_bps: u32,
}

pub struct SleeveState {
    asset_id: String,
    contract_address: Address,
    target_weight_bps: u32,
    band_bps: u32,
    current_balance: i128,
    sleeve_type: SleeveType,
    liquidity_tier: u8,
}
```

**RedemptionCapacity struct:**
```rust
pub struct RedemptionCapacity {
    instant: i128,   // from liquidity sleeve, in numéraire
    in_24h: i128,
    in_1w: i128,
    longer: i128,
    computed_at: u64,
}
```

**RiskSheet struct:**
```rust
pub struct RiskSheet {
    recommended_ltv_bps: u32,     // derived from composition, see references/COLLATERAL-PASSPORT.md Pillar 3
    recommended_haircut_bps: u32, // e.g. 1000 = 10% for a >50% cash basket; never hard-coded
    weighted_avg_redemption_hours: u32,
    computed_at: u64,
}
```

**Share class:** Open (Phase 1); Gated/SEP-57 (Phase 3+, requires identity verifier)

---

## NAV Oracle Contract

**Purpose:** Computes conservative and deposit NAV from multiple sources. Enforces all oracle safety rules from `architecture/NAV-ORACLE.md`.

**Key functions:**
```rust
// SEP-40 (Draft v0.1.0) — exact interface; Blend v2 calls lastprice(Asset::Stellar(addr))
// enum Asset { Stellar(Address), Other(Symbol) }
// struct PriceData { price: i128, timestamp: u64 }
fn base(env: Env) -> Asset                                         // numéraire
fn assets(env: Env) -> Vec<Asset>
fn decimals(env: Env) -> u32
fn resolution(env: Env) -> u32                                     // seconds
fn price(env: Env, asset: Asset, timestamp: u64) -> Option<PriceData>
fn prices(env: Env, asset: Asset, records: u32) -> Option<Vec<PriceData>>
fn lastprice(env: Env, asset: Asset) -> Option<PriceData>          // conservative NAV; None while sleeve paused/halted

// Constella extensions (non-SEP-40)
fn get_collateral_nav(env: Env, sleeve_id: Symbol) -> Option<PriceData>  // minimum of valid sources
fn asset_price(env: Env, asset: Address) -> Option<PriceData>            // median of the asset's own valid sources; base = 1.0
fn get_deposit_nav(env: Env, sleeve_id: Symbol) -> Option<PriceData>     // median of valid sources

// Oracle management
fn get_sources(env: Env) -> Vec<OracleSource>
fn is_halted(env: Env) -> bool
fn is_sleeve_paused(env: Env, sleeve_id: String) -> bool
fn update_source(env: Env, source_id: String, price: i128, timestamp: u64)  // oracle feeds only
// caller must sign AND be the pause authority or the admin (audit O1)
fn trigger_circuit_breaker(env: Env, caller: Address, sleeve_id: Symbol, reason: Symbol)
fn clear_circuit_breaker(env: Env, sleeve_id: Symbol)  // admin (multisig) only
fn set_pause_authority(env: Env, new_authority: Address)  // admin only; defaults to admin
fn get_pause_authority(env: Env) -> Address
```

**PriceData struct:**
```rust
pub struct PriceData {
    price: i128,       // price × 10^decimals
    timestamp: u64,    // ledger timestamp
    source: String,    // e.g. "REFLECTOR" | "REDSTONE" | "BENJI_NAV_API"
}
```

**Events emitted:**
```
Topic: ["NAV_PAUSE", sleeve_id]
Data: { reason, sleeve, sources_valid, sources_total, deviation_bps, timestamp }

Topic: ["NAV_RESUME", sleeve_id]
Data: { sleeve, sources_valid, timestamp }
```

---

## Keeper Contract

**Purpose:** Proposes rebalance transactions when sleeve weights leave bands. Extends Soroban state TTLs. The keeper has NO admin authority — it can only propose rebalances (which the policy contract validates).

**Key functions:**
```rust
fn propose_rebalance(env: Env, vault: Address, proposal: RebalanceProposal)
fn execute_rebalance(env: Env, vault: Address, proposal_id: BytesN<32>)
fn extend_ttl_batch(env: Env, entries: Vec<Address>)
fn is_registered_keeper(env: Env, address: Address) -> bool
fn add_keeper(env: Env, keeper: Address)     // admin only
fn remove_keeper(env: Env, keeper: Address)  // admin only
```

**Phase 1:** Single Constella-operated keeper (manual triggers)
**Phase 4:** Multi-keeper network (3+ keepers, on-chain coordination)

---

## Issuer Adapter Interface

Each issuer adapter implements the `IssuerAdapter` trait:

```rust
pub trait IssuerAdapter {
    fn request_redemption(env: Env, amount: i128) -> RedemptionRequest;
    fn get_redemption_status(env: Env, id: BytesN<32>) -> RedemptionStatus;
    fn claim_redemption(env: Env, id: BytesN<32>) -> i128;
    fn subscribe(env: Env, amount: i128) -> SubscriptionRequest;
    fn get_pending_amount(env: Env) -> i128;
    fn get_instant_capacity(env: Env) -> i128;
}
```

**Adapter build priority:**
| Phase | Adapter | Note |
|---|---|---|
| Phase 1 | None — USDC/EURC need no adapter | Direct |
| Phase 2 | Blend, DeFindex | Permissionless |
| Phase 3 | Spiko, USDY, BENJI | After allow-listing |
| Phase 4 | YLDS, JQQQ | After allow-listing |

---

## Security Properties

| Property | Implementation |
|---|---|
| Admin cannot move user funds | `redeem()` checks `owner == caller` or allowance; no admin bypass |
| Policy cannot be exceeded | Policy contract validates every proposal; rejects if any check fails |
| Oracle cannot be gamed by thin markets | Min-liquidity-depth check in NAV oracle (Rule 2) |
| Upgrade requires multisig + timelock | `upgrade()` gated by admin multisig and 48h timelock |
| Pause is fast | Single-sig pause authority; multi-sig required to unpause |
| Circuit breakers are per-sleeve | One bad oracle/issuer does not halt the whole vault |
| Fee accrual is transparent | On-chain, visible at all times as share dilution |
| Keeper is not trusted | Keeper keypair has only proposal authority; policy contract validates |

---

## Upgrade and Deployment

All strategy vault instances share one WASM binary. Fleet upgrades use CAP-85 "externally managed contract executables" (Protocol 28). There is **no** `--upgrade-fleet` CLI flag; follow the CAP-85 and stellar-cli documentation for the exact procedure and verify on testnet first.

State migrations use the standard CAP-85 data-migration path.

**Before any upgrade:**
1. Admin multisig + 48h timelock approval
2. Testnet deployment and full verification
3. Public announcement 48h minimum before upgrade
4. Per-sleeve circuit breakers armed during upgrade window

---

## Events Reference

All state-changing functions must emit events. Format:

```
Topic: ["EVENT_NAME", ...discriminators]
Data: { relevant fields, timestamp }
```

| Event | Trigger |
|---|---|
| `DEPOSIT` | User deposits numéraire |
| `REDEEM` | User redeems shares |
| `REBALANCE_STEP` | Each trade in a rebalance |
| `REBALANCE_COMPLETE` | Full rebalance completed |
| `NAV_PAUSE` | Circuit breaker triggered |
| `NAV_RESUME` | Circuit breaker cleared |
| `FEE_ACCRUAL` | Management fee accrued |
| `POLICY_REGISTERED` | New policy hash stored |
| `POLICY_UPDATED` | Policy hash updated |
| `VAULT_PAUSED` | Vault paused |
| `VAULT_UNPAUSED` | Vault unpaused |
| `UPGRADE` | WASM upgraded |
