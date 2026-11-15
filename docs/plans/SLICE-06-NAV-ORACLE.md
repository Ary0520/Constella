# Slice 6 Micro-Spec: NAV Oracle Contract (SEP-40 Multi-Source & Deviation Circuit Breakers)

> **Module:** `contracts/nav-oracle`  
> **Status:** Implemented. Open findings (O1–O4, W5) tracked in `docs/audit/AUDIT-2026-11-15.md`.  
> **Reference:** [`docs/build-spec/architecture/NAV-ORACLE.md`](../build-spec/architecture/NAV-ORACLE.md), [`docs/build-spec/contracts/CONTRACTS-SPEC.md`](../build-spec/contracts/CONTRACTS-SPEC.md#L197-L240), [`docs/build-spec/contracts/SEPS.md`](../build-spec/contracts/SEPS.md#L87-L111), [`docs/build-spec/AGENT-RULES.md`](../build-spec/AGENT-RULES.md) (Rule 1, 3, 6, 8, 11, 12)

---

## 1. Objectives of Slice 6

Implement and verify the on-chain NAV Oracle contract in Soroban Rust:
1. **The 5 Inviolable Oracle Laws (Rule 3):**
   - **Rule 1:** Fund tokens prioritize primary issuer NAV.
   - **Rule 2:** Minimum liquidity depth floor ($100k+) for on-chain DEX/AMM prices; sub-depth feeds are ignored entirely.
   - **Rule 3 (Dual NAV):**
     - `collateral_nav` = **Minimum** of valid sources (conservative lender collateral sizing).
     - `deposit_nav` = **Median** of valid sources (equitable share minting/redemption).
   - **Rule 4 (Circuit Breaker):** Automated sleeve pause if valid sources diverge by > 2%–3% (`deviation_threshold_bps`). Emits `NAV_PAUSE`.
   - **Rule 5 (Hard Stop & Staleness):** Feeds exceeding `staleness_limit_seconds` are invalid; if fewer than 2 valid sources remain, trigger hard stop on the sleeve.
2. **SEP-40 Interface Compliance:** Full compatibility with Stellar's oracle standard (`price`, `lastprice`, `base`, `decimals`, `resolution`, `assets`) for direct lending integration on Blend and Liqvid.
3. **Per-Sleeve Isolation:** An issue or pause on one sleeve never halts or corrupts unaffected sleeves.

---

## 2. Smart Contract Interface (Soroban Rust)

```rust
#[contractclient(name = "NavOracleClient")]
pub trait NavOracleTrait {
    /// Initialize oracle with admin, base asset (numéraire), decimals, and resolution
    fn initialize(env: Env, admin: Address, base_asset: Address, decimals: u32, resolution: u32);

    /// Configure sleeve deviation threshold (bps) and staleness limits
    fn configure_sleeve(
        env: Env,
        sleeve_id: Symbol,
        deviation_threshold_bps: u32,
        min_valid_sources: u32,
    );

    /// Register a new oracle price feed source (admin only)
    fn register_source(env: Env, source: OracleSource);

    /// Update a price feed with current price, depth, and timestamp
    fn update_price(
        env: Env,
        source_id: Symbol,
        price: i128,
        liquidity_depth_usd: i128,
        timestamp: u64,
    );

    /// Rule 3: Conservative NAV (minimum of valid sources) for collateral sizing
    fn get_collateral_nav(env: Env, sleeve_id: Symbol) -> Option<PriceData>;

    /// Rule 3: Deposit NAV (median of valid sources) for share pricing
    fn get_deposit_nav(env: Env, sleeve_id: Symbol) -> Option<PriceData>;

    /// SEP-40: Standard price query
    fn price(env: Env, asset: Address) -> Option<PriceData>;

    /// SEP-40: Standard last price query
    fn lastprice(env: Env, asset: Address) -> Option<PriceData>;

    /// SEP-40 metadata queries
    fn base(env: Env) -> Address;
    fn decimals(env: Env) -> u32;
    fn resolution(env: Env) -> u32;
    fn assets(env: Env) -> Vec<Address>;

    /// Circuit breaker status and management
    fn is_sleeve_paused(env: Env, sleeve_id: Symbol) -> bool;
    fn trigger_circuit_breaker(env: Env, caller: Address, sleeve_id: Symbol, reason: Symbol); // pause authority or admin (audit O1)
    fn set_pause_authority(env: Env, new_authority: Address); // admin only
    fn get_pause_authority(env: Env) -> Address;
    fn clear_circuit_breaker(env: Env, sleeve_id: Symbol);

    /// Admin query
    fn get_admin(env: Env) -> Address;
}
```

### Core Data Structs
```rust
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PriceData {
    pub price: i128,       // price * 10^decimals
    pub timestamp: u64,    // ledger timestamp
    pub source: Symbol,    // e.g. "REFLECTOR", "REDSTONE", "BENJI_NAV"
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OracleSource {
    pub source_id: Symbol,
    pub asset: Address,
    pub sleeve: Symbol,
    pub price: i128,
    pub timestamp: u64,
    pub staleness_limit_seconds: u64,
    pub min_liquidity_depth_usd: i128,
    pub current_liquidity_depth_usd: i128,
    pub is_active: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SleeveOracleConfig {
    pub sleeve_id: Symbol,
    pub deviation_threshold_bps: u32, // e.g. 200 = 2%
    pub min_valid_sources: u32,       // default 2
    pub is_paused: bool,
    pub pause_reason: Symbol,
}
```

---

## 3. Mandatory Test Matrix (TDD Verification)

| Test ID | Test Scenario | Expected Outcome |
|---|---|---|
| **T33** | Multi-Source Pricing & Dual NAV Evaluation | With 3 valid feeds ($1.00, $1.01, $0.99), `collateral_nav` returns $0.99 (min) while `deposit_nav` returns $1.00 (median). Invariant: `collateral_nav <= deposit_nav`. |
| **T34** | Thin Liquidity Depth Exclusion (Rule 2) | Feed with liquidity depth ($50k) below floor ($100k) is ignored completely, preventing YieldBlox-style exploits. |
| **T35** | Staleness Rejection (Rule 5) | Feed older than `staleness_limit_seconds` is excluded as invalid. |
| **T36** | Hard Stop on Insufficient Sources (Rule 5) | When fewer than `min_valid_sources` (2) remain valid, `get_collateral_nav` halts and returns `None` / `InsufficientSources`. |
| **T37** | Deviation Circuit Breaker (Rule 4) | When 2 valid feeds diverge by >2.5% (above 2.0% threshold), the affected sleeve automatically pauses, emits `NAV_PAUSE`, and rejects pricing. |
| **T38** | Per-Sleeve Isolation | Pausing `USD_GOV_FUNDS` leaves `CASH_STABLE` operating normally without interference. |
| **T39** | SEP-40 Interface Conformance | `price`, `lastprice`, `base`, `decimals`, `resolution`, `assets` return compliant outputs for lenders. |

---

## 4. Acceptance Criteria to Gate Slice 6

- [x] Cargo workspace configured with `nav-oracle` member.
- [x] Contract authored with Soroban SDK and compiles cleanly.
- [x] `cargo test` in `contracts/nav-oracle` passes 100% of T33–T39 test cases.
- [x] Optimized WASM build generated via `stellar contract build`.
- [x] High-frequency atomic commits and pushes to GitHub.
- [x] AI pauses and presents test assertion proofs before touching Slice 7.
