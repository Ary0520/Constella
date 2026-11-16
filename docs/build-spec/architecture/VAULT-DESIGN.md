# Vault Design Specification

> Reference: Product doc §4.3 Vault Design, §2.2 What the User Gets

---

## Core Vault Concept

Each Constella vault:
- Holds **multiple RWA constituents** (tokens from different issuers)
- Accepts deposits and pays redemptions in a single **numéraire** (USDC or EURC)
- Issues **share tokens** representing proportional ownership
- Prices shares as **NAV in the numéraire**
- Self-rebalances within a **signed policy**

This makes the vault SEP-56 compatible even though it internally holds many assets.

---

## Vault Types

### Strategy Vault (Default)
- **Shared** across many holders
- Cheap — shared state, shared gas
- Ideal for: individual users, partner-embedded baskets
- One WASM deployed; multiple vault instances share the code
- Upgraded atomically via CAP-85 (Adapter, Protocol 28)

### Private Vault (Q1 2027)
- Deployed **per institution** via a factory contract
- Own policy, own admin accounts, own share class
- Required for: treasuries needing custom policy limits, multisig control, audit exports
- Supports ML-DSA admin keys after Bell (Protocol 30)
- More expensive (own state, own rent)

**v1 delivers Strategy Vault only.** Private Vault Factory is Q1 2027.

---

## Share Classes

### Open Share Class
- For baskets composed **entirely of permissionless assets**
- No KYC required from holders
- Freely transferable between any accounts
- Minted by depositing numéraire; burned by redemption

### Gated Share Class (SEP-57 / T-REX) — Q1 2027
- Required whenever **any constituent is permissioned** (e.g. BENJI, USDY, JQQQ)
- Every holder must pass the SEP-57 identity verifier
- Transfers obey issuer rules (allow-list, clawback, freeze compatible)
- The vault is listed on the issuer's allow-list as a valid holder contract
- If an issuer freezes an asset: the **sleeve** is frozen, not the entire basket
  - Remaining sleeves continue to operate
  - Redemptions from frozen sleeve are queued
  - **Implementation status (16 Nov 2026):** a frozen or unpriced sleeve that holds nothing is skipped when pricing shares (audit V3). For a *funded* sleeve without a valid price (audit **V6**, `d1ecad4`):
    - `redeem` and deposits stay halted — there is no fair share price, and an entry at a stale price would dilute holders.
    - `redeem_frozen(shares, …)` (explicit opt-in) pays `shares / supply` of every **priced** sleeve's conservative value in cash now, and records `shares / supply` of each frozen sleeve's **units** as a claim for the receiver. The units leave the sleeve's books, so the remaining holders' per-share value is unchanged. The usual rules apply: operator auth + allowance, pause, fee accrual, cash actually held, and the liquidity floor measured over the priced part.
    - `settle_claim(holder, asset)` (anyone may call; pays only the holder) settles in cash at units × **conservative** price once the asset is priced again, returning the units to the books. If the issuer clawed tokens back meanwhile, payment is capped pro rata to the reserved units still held. Settlement needs cash: the keeper raises it through policy-validated rebalances.
    - A sleeve with outstanding claims cannot be removed (the reserved units are still held).

---

## Numéraire

| Property | Rule |
|---|---|
| One per vault | A vault settles all deposits and redemptions in one asset |
| Choices | USDC or EURC |
| Why one? | Keeps SEP-56 compatible; simplifies NAV calculation |
| Currency matching | A euro-currency basket uses EURC as numéraire |
| Must be registered | The numéraire must be an active **Cash** entry in the registry, or the vault cannot be deployed (audit M4) |
| Labels | Every sleeve's label is its registry asset ID (e.g. the cash sleeve of a EURC vault is `EURC`); free-text labels are rejected (audit M4) |

---

## Liquidity Sleeve

**Purpose:** Ensure redemptions and liquidations can settle immediately.

| Property | Value |
|---|---|
| Default minimum | 5–15% of AUM in cash-like assets |
| Eligible assets | USDC, EURC (permissionless, instant settlement) |
| Redemption flow | Small redemptions: draw from liquidity sleeve first |
| Large exits | Queued redemption; where issuer allows, redemption in kind |
| Collateral liquidations | Liquidator draws from liquidity sleeve; policy contract validates |

**Share price source (audit V5, 15 Nov 2026):** the share price is always computed from the vault's own holdings; no oracle feed can override it. The vault *publishes* its NAV for lenders; it never reads its NAV from an oracle.

**Valuation rule (audit V4, 15 Nov 2026):** every valuation — deposit NAV, conservative NAV, `total_assets`, redemption capacity, risk sheet — uses one function. Cash counts at face value; an empty sleeve counts as 0; a funded non-cash sleeve must have a valid oracle price or the call fails with `OracleUnavailable`. There is never a par (1.0) fallback.

**Rule:** The policy contract MUST enforce the liquidity floor at all times. Any rebalance that would bring the liquidity sleeve below the floor is rejected.

---

## Sleeve Architecture

A vault logically partitions its holdings into named sleeves:

```
Vault (USDC numéraire)
├── Sleeve: CASH_STABLE        → USDC, EURC          (5–15% by default)
├── Sleeve: USD_GOV_FUNDS      → BENJI, USDY, Spiko  (up to 70%)
├── Sleeve: EUR_SOVEREIGN      → Spiko EUR, GOVY      (up to 20%)
├── Sleeve: YIELD_DOLLAR       → YLDS, wYLDS           (up to 20%)
├── Sleeve: LENDING            → Blend/DeFindex        (up to 15%)
├── Sleeve: EQUITY             → JQQQ, DTC stocks     (future, gated)
└── Sleeve: XLM                → Native XLM           (optional, hard-capped at 5%)
```

Sleeves have:
- A **target weight** (% of AUM)
- A **band** (min/max %) — rebalance triggered when weight leaves band
- An **issuer cap** (max % from any single issuer)
- A **liquidity tier** (determines redemption priority)

---

## Deposit Flow

```
User deposits USDC
      │
      ▼
Eligibility check (registry + SEP-57 if gated)
      │
      ▼
NAV calculation (median of valid sources)
      │
      ▼
Shares minted = deposit_amount / NAV_per_share
      │
      ▼
Deposit directed to underweight sleeves (flow-based rebalancing)
      │
      ▼
Share token transferred to user wallet
```

---

## Redemption Flow

```
User requests redemption of N shares
      │
      ▼
Shares burned
      │
      ▼
Redemption amount = N × NAV_per_share (collateral NAV — conservative)
      │
      ▼
Can liquidity sleeve cover it?
  ├── YES → Instant: USDC transferred from liquidity sleeve
  └── NO  → Queued: redemption joins the queue
              │
              ▼
         Issuer redemption initiated for required amount
              │
              ▼
         USDC transferred when settled (time per issuer terms)
```

**Redemption ladder published:** The UI shows how much is available in minutes, 1 day, 1 week.

---

## Fee Accrual

| Fee | Method | On-chain? |
|---|---|---|
| Performance fee (D-D) | 10% of profit above the per-share high-water mark (conservative NAV), max 0.20%/yr of assets, at most daily; share dilution | YES — enforced and visible on-chain |
| Subscription fee | Off-chain billing | No |
| Issuer expense ratios | Pass-through; no Constella markup | Shown in UI before deposit |

**Share dilution method:**
- Accrue fees by minting new shares to the protocol treasury wallet
- NAV per share decreases slightly each period
- No separate fee extraction transaction needed
- Fully transparent — anyone can compute fee accrual from on-chain data

---

## Upgrade Strategy

**Timelock (audit W3, 15 Nov 2026):** upgrades are two-step on-chain — `propose_upgrade(hash)` then, no earlier than 48h later, `execute_upgrade()` — so the public notice below is enforced by the contract, not just process. `cancel_upgrade()` drops a proposal.

Using Adapter (Protocol 28, CAP-85):
- All vault instances share one WASM binary
- Upgrade is atomic across the fleet — one transaction upgrades all shared-code vaults
- State migration uses CAP-85's standard data-migration path
- No downtime for users during upgrade (ledger state is migrated atomically)

**Before any upgrade:**
1. Multisig + timelock approval
2. Testnet deployment and verification
3. Public announcement with 48h notice minimum
4. Circuit breakers armed

---

## State Design (Soroban Rent Considerations)

Soroban charges rent per storage entry. Design rules:
- **Minimal per-user state** — only share balance; positions tracked at vault level, not user level
- **Shared vault state** — one entry per sleeve (not per holder per sleeve)
- **Keepers extend TTLs** in batches using Protocol 26's precise TTL control (CAP-78)
- **Registry entries** — persistent storage; keepers monitor and extend
- **Policy objects** — persistent storage; one per vault

---

## Collateral Passport (On-Chain View Functions)

Each vault exposes these view functions for lenders:

```rust
// Conservative NAV for collateral sizing
fn collateral_nav() -> (amount: i128, numéraire: Address, timestamp: u64, sources: Vec<Source>)

// How much can be redeemed right now (for position sizing)
fn instant_redeem_capacity() -> (amount: i128, numéraire: Address)

// Recommended LTV and haircut for this basket
fn risk_sheet() -> (recommended_ltv: u32, haircut_bps: u32, basis: String)

// Pool configuration data for lenders
fn listing_kit() -> ListingKit
```

**Lender integration:** Lenders use `collateral_nav()` and `instant_redeem_capacity()` for position management. The oracle recommended config in `listing_kit()` matches our NAV oracle rules (see `NAV-ORACLE.md`).

---

## XLM Sleeve (Optional)

- Hard-capped at 5% of any basket
- Optional — not in default baskets
- Purpose: network activity participation, fee float
- Permissionless
- Not eligible as a "liquidity sleeve" asset (less stable than USDC/EURC)
