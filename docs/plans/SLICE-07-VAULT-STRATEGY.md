# Slice 7 Micro-Spec: Strategy Vault Contract (`contracts/vault-strategy`)

## 1. Context & Objectives
- **Target:** `contracts/vault-strategy`
- **Build Spec References:**
  - `docs/build-spec/architecture/VAULT-DESIGN.md` (Sleeve architecture, liquidity floor, fee accrual, Collateral Passport)
  - `docs/build-spec/contracts/CONTRACTS-SPEC.md` (§ Strategy Vault Contract, lines 114–195)
  - `docs/build-spec/contracts/SEPS.md` (§ SEP-56 Tokenized Vault & § SEP-41 Fungible Token Standard)
- **Role:** Core multi-asset SEP-56 / SEP-41 yield-bearing vault contract on Stellar Soroban.

---

## 2. Invariants & Rules
1. **Rule 1 (Closed Universe):** Vault constituents must be active assets registered in `AssetRegistry`.
2. **Rule 3 (Dual NAV & Pricing):**
   - Deposits (`deposit`, `mint`, `convert_to_shares`) are evaluated using `get_nav_deposit()` (median of valid oracle feeds) to ensure fair entry and prevent dilution.
   - Redemptions (`redeem`, `withdraw`, `convert_to_assets`) are evaluated using `get_nav_conservative()` (minimum valid oracle feed) to protect remaining shareholders from overpaying departing capital.
   - Invariant: `conservative_nav <= deposit_nav`.
3. **Rule 5 (Policy Immutability):** Vault rebalancing, sleeve boundaries, and trading operations are bound to a verified 32-byte cryptographic policy hash stored in `constella-policy`.
4. **Liquidity Sleeve Protection:**
   - Free numéraire (USDC/EURC) in `CASH_STABLE` sleeve must satisfy the policy liquidity floor (5%–15% default).
   - Instant redemptions draw directly from the `CASH_STABLE` sleeve. If requested assets exceed available cash or breach the floor, redemptions are queued or rejected at limit.
5. **Fee Accrual via Continuous Share Dilution:**
   - Management fee (e.g. 15 bps/year) is accrued based on elapsed ledger time by minting proportional new shares directly to the protocol treasury. Underlying asset reserves are never extracted.
6. **Collateral Passport:**
   - Exposes standardized view functions for lending protocols (e.g. Blend, Liqvid):
     - `get_nav_conservative()`
     - `get_nav_deposit()`
     - `get_redemption_capacity()` (instant, 24h, 1w)
     - `get_risk_sheet()` (recommended LTV, haircut bps)
     - `get_listing_kit()` (complete pool parameter specification)
7. **Emergency Circuit Breakers:**
   - `pause()` can be invoked by `pause_authority` or `admin` to halt deposits, mints, and transfers immediately.
   - `unpause()` strictly requires `admin.require_auth()`.
8. **WASM Symbol Decoupling:**
   - Keep cross-contract dependencies (`constella-registry`, `constella-policy`, `constella-nav-oracle`) out of `[dependencies]` to avoid duplicate `#[no_mangle]` symbol collisions in `cdylib`. Use local client definitions with `#[contractclient]`.

---

## 3. Data Structures & Storage Layout

```rust
#[derive(Clone, Copy, PartialEq, Eq)]
#[contracttype]
pub enum SleeveType {
    CashStable = 0,
    UsdGovFunds = 1,
    EurSovereign = 2,
    YieldDollar = 3,
    Lending = 4,
    Equity = 5,
    Xlm = 6,
}

#[derive(Clone)]
#[contracttype]
pub struct SleeveState {
    pub asset_id: String,
    pub contract_address: Address,
    pub target_weight_bps: u32,
    pub band_bps: u32,
    pub current_balance: i128,
    pub sleeve_type: SleeveType,
    pub liquidity_tier: u8,
}

#[derive(Clone)]
#[contracttype]
pub struct VaultConfig {
    pub admin: Address,
    pub pause_authority: Address,
    pub numeraire: Address,
    pub policy_contract: Address,
    pub oracle_contract: Address,
    pub registry_contract: Address,
    pub treasury: Address,
    pub management_fee_bps: u32,
}

#[derive(Clone)]
#[contracttype]
pub struct RedemptionCapacity {
    pub instant: i128,
    pub in_24h: i128,
    pub in_1w: i128,
    pub longer: i128,
    pub computed_at: u64,
}

#[derive(Clone)]
#[contracttype]
pub struct RiskSheet {
    pub recommended_ltv_bps: u32,
    pub recommended_haircut_bps: u32,
    pub weighted_avg_redemption_hours: u32,
    pub computed_at: u64,
}

#[derive(Clone)]
#[contracttype]
pub struct ListingKit {
    pub share_token: Address,
    pub numeraire: Address,
    pub oracle_contract: Address,
    pub recommended_ltv_bps: u32,
    pub recommended_haircut_bps: u32,
    pub liquidation_threshold_bps: u32,
}
```

---

## 4. Test Matrix (T40 - T46)

| Test ID | Objective | Scenario | Expected Outcome |
|---|---|---|---|
| **T40** | SEP-56 Deposit & Mint | User deposits numéraire USDC into vault | Mints shares based on deposit NAV; receiver balance and `total_supply` updated |
| **T41** | Instant Redemption | User burns shares for numéraire | Evaluates at conservative NAV; transfers assets from liquidity sleeve |
| **T42** | Fee Dilution Calculation | Time advances on-chain | New shares minted to treasury proportionally; total assets unchanged |
| **T43** | Collateral Passport Views | Lender queries risk and redemption stats | Returns valid `RedemptionCapacity`, `RiskSheet`, `ListingKit`, and dual NAVs |
| **T44** | Emergency Pause & Unpause | Pause authority pauses vault; admin unpauses | Operations blocked during pause; restored upon admin unpause |
| **T45** | SEP-41 Token Standard | Standard transfer, allowance, approve, transfer_from | Operates identically to standard Soroban token |
| **T46** | Liquidity Sleeve Floor Enforcement | User attempts redemption exceeding liquid cash | Redemptions exceeding instant capacity are capped or queued |
