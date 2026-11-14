# The Collateral Passport Specification

> The signature architecture turning Constella vault share tokens from "held yield" into "accepted collateral" across Stellar lending protocols.

---

## 1. Overview & Strategic Mandate

At Meridian 2026, Stellar Development Foundation leadership articulated a clear mandate: **tokenized assets must move from being merely "useful" (earning idle yield in a wallet) to "used" (active as collateral in lending, margin, and repo).**

Today, lending protocols (Blend, Liqvid) hesitate to list complex multi-asset tokens because:
- NAV pricing is difficult to verify without trusting a centralized black box.
- Underlying constituents have staggered redemption times (some instant, some 24 hours, some 7 days).
- In liquidations, selling a multi-asset token into thin AMMs causes catastrophic slippage.
- Permissioned assets require the liquidator itself to be whitelisted by the underlying issuer.

**The Collateral Passport solves every single one of these problems on-chain.**

---

## 2. The Five Pillars of the Collateral Passport

```
┌────────────────────────────────────────────────────────────────────────┐
│                        THE COLLATERAL PASSPORT                         │
└────────────────────────────────────────────────────────────────────────┘
     │                    │                    │                    │
     ▼                    ▼                    ▼                    ▼
1. Conservative NAV  2. Instant-Redeem    3. Dynamic Risk      4. Compliant
   Oracle Engine        Capacity View        Sheet & Haircut      Liquidation Hook
   Min of valid         Real-time liquid     Recommended LTV      Identity-gated
   sources (SEP-40)     exit capacity        based on duration    liquidator whitelist
```

---

### Pillar 1: Conservative Multi-Source NAV
- **Method:** Evaluates all valid, non-stale, non-deviant price feeds and returns the **minimum** (floor) value.
- **Why Minimum:** Lenders must size risk based on conservative liquidation value, not optimistic median prices.
- **Circuit Breaker:** If independent feeds diverge by > 2–3%, the passport raises an on-chain flag and halts fresh borrowing against the collateral.
- **Interface:** Implements the standard `SEP-40` oracle interface.

---

### Pillar 2: Instant-Redeem Capacity View
- **What it does:** A view function returning exactly how much capital (in numéraire USDC/EURC) can be redeemed **right now** versus in 24 hours versus in 1 week.
- **Value to Lenders:** Allows money market risk models to cap the maximum borrowable size against this collateral to ensure liquidations can settle without market freeze.

```rust
pub struct RedemptionCapacity {
    pub instant_capacity: i128,   // Liquid cash sleeve + instant AMM depth
    pub within_24h: i128,          // T-Bill funds settling T+1 (BENJI, Spiko)
    pub within_1w: i128,           // Longer settlement assets
    pub illiquid_tier: i128,       // Locked or queued
    pub timestamp: u64,
}
```

---

### Pillar 3: Dynamic Risk Sheet & Haircut Recommendations
- **What it does:** Computes recommended Loan-to-Value (LTV) and liquidation haircuts directly from the weighted duration and liquidity of the underlying constituents.
- **Benchmark:** Aligned with the Meridian 2026 repo reference model (e.g., 2% haircut at SOFR + 10 bps for prime sovereign collateral).
- **Standardized Recommendations:**
  - High-Cash Basket (> 50% USDC/EURC): **90% LTV (10% Haircut)**
  - Pure T-Bill Basket (BENJI, Spiko): **85% LTV (15% Haircut)**
  - Yield & Lending Basket (Blend, wYLDS): **70%–75% LTV (25%–30% Haircut)**

---

### Pillar 4: Compliant Liquidation Path
- **The Challenge:** In gated baskets (SEP-57), random liquidation bots cannot legally hold underlying tokens like BENJI or JQQQ without KYC.
- **The Solution:**
  - The vault exposes a whitelisted liquidator hook (`is_eligible_liquidator`).
  - Institutional market makers and approved liquidator bots (who have verified their credentials with the SEP-57 identity registry) can seize and unwind the position.
  - Aligned with the architectural framework validated by Liqvid (SCF #45).

---

### Pillar 5: The Lender Listing Kit
- A self-contained deployment package for Blend pool creators and DAO risk committees:
  1. Verified contract addresses (Share token, NAV oracle, Policy contract).
  2. Recommended Blend pool parameters (Borrow rate slope, collateral factor, reserve factor).
  3. Pre-configured oracle adaptor code.
  4. Integration tests verifying mock liquidation scenarios.

---

## 3. On-Chain Soroban Interface

```rust
pub trait CollateralPassportTrait {
    /// Returns the conservative collateral NAV (minimum valid source) in numéraire
    fn get_nav_conservative(env: Env) -> (i128, u32);

    /// Returns the deposit NAV (median of valid sources) used for minting
    fn get_nav_deposit(env: Env) -> (i128, u32);

    /// View function detailing instant vs queued redemption liquidity
    fn get_redemption_capacity(env: Env) -> RedemptionCapacity;

    /// Returns calculated risk metrics, recommended LTV, and haircut
    fn get_risk_sheet(env: Env) -> RiskSheet;

    /// Compliance check confirming if a liquidator address is authorized to seize collateral
    fn is_eligible_liquidator(env: Env, liquidator: Address) -> bool;

    /// Health and staleness status of all registered oracle feeds
    fn get_oracle_status(env: Env) -> Vec<OracleSourceStatus>;
}
```

---

## 4. Lender Integration Workflow

```
   Constella Vault Deployed
              │
              ▼
    Collateral Passport Active
 (Emits conservative NAV via SEP-40)
              │
              ▼
   Blend / Liqvid Pool Operator
   Reviews Listing Kit & Risk Sheet
              │
              ▼
  Pool Approves Basket Token
  Sets LTV based on Risk Sheet (e.g. 85%)
              │
              ▼
    Borrowers Deposit Basket Token
   Borrow USDC / EURC against their yield
```
