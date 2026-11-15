# NAV & Oracle Design Specification

> Reference: Product doc §4.4 NAV and Oracle Design, §2.4 Collateral Passport  
> **CRITICAL — read before any oracle or pricing implementation.**

---

## Why This Exists: The YieldBlox Incident

On 22 February 2026, the YieldBlox lending pool lost over $10M. This was not a smart contract bug. It was an oracle misconfiguration:
- The pool accepted a price derived from a nearly empty on-chain market
- One trade moved the SDEX price from ~$1 to ~$107
- The oracle fed this manipulated price to the pool
- The attacker borrowed against artificially inflated collateral

**Our NAV design exists to make this impossible.**

The fix is not complex: never price from a single thin source, enforce multi-source validation with deviation limits, and hard-stop when sources are insufficient.

---

## The Five Oracle Rules (All Are Mandatory)

### Rule 1: Fund tokens use issuer NAV as primary
- For tokenized money-market funds (BENJI, USDY, Spiko, etc.): issuer's published NAV is the **primary source**
- Cross-check against on-chain trading price where liquid on-chain markets exist
- If on-chain price deviates by >threshold from issuer NAV: **flag and investigate, do not use the on-chain price alone**

### Rule 2: Market-traded assets need minimum liquidity depth
- For any price from an on-chain market (SDEX, Soroswap, Aquarius):
  - A **minimum liquidity depth threshold** must exist for the price to be counted
  - If depth is below threshold: source is **ignored entirely** — not averaged, not discounted, ignored
  - Threshold is per-asset, documented in the registry entry

### Rule 3: Two NAV values, two purposes
| NAV type | Method | Used for |
|---|---|---|
| **Collateral NAV** | Minimum of all valid sources | Lender collateral sizing, Collateral Passport, liquidation |
| **Deposit/Share NAV** | Median of all valid sources | Share minting on deposit, share pricing for UI |

This means collateral NAV is always conservative (protects lenders). Deposit NAV is fair (protects depositors from being diluted).

### Rule 4: Deviation circuit breaker
- If two or more valid sources exist and their values differ by **>2–3%** (threshold per asset, set in registry):
  - **Deposits paused** for the affected sleeve
  - **Redemptions paused** for the affected sleeve  
  - Operators alerted immediately
  - Logs the deviation event on-chain
- Trading and rebalancing for the affected sleeve also paused
- Unaffected sleeves continue normally
- **Where it is detected (audit O4, 15 Nov 2026):** the deviation check runs inside every price write (`update_price`, `register_source`), so the pause and its `NAV_PAUSE` event persist in that successful transaction. Reads re-check as defence in depth, but a pause first seen inside a caller that reverts would be rolled back — so writes are the authoritative trigger. Staleness and insufficient sources (Rule 5) are evaluated live on every read and are not latched.

### Rule 5: Staleness limits and hard stop
- Every source has a **staleness limit** (time since last update, defined per-source in registry)
- Sources beyond staleness limit are treated as invalid (same as unavailable)
- **Timestamp integrity (audit O2, 15 Nov 2026):** a price stamped later than the current ledger time is rejected on write and treated as invalid if found in storage (otherwise it would never go stale); a feed update older than the stored one is rejected (prevents replaying an old-but-fresh price over newer data)
- **No sleeve may be configured below 2 sources** — `configure_sleeve` rejects `min_valid_sources < 2` (`MIN_VALID_SOURCES_FLOOR`, audit O3)
- If **fewer than 2 valid sources** remain for a sleeve:
  - **Hard stop**: deposits, redemptions, and rebalancing for that sleeve are halted
  - Operators alerted
  - The sleeve stays halted until valid sources are restored
  - Remaining sleeves continue normally

---

## Oracle Sources by Asset Type

| Asset Type | Primary Source | Secondary Sources | Fallback |
|---|---|---|---|
| Tokenized money-market funds (BENJI, USDY, Spiko) | Issuer published NAV API | Reflector, RedStone (if feed exists) | None — halt if primary unavailable |
| Euro sovereign bonds (Spiko EUR, GOVY) | Issuer NAV | Reflector | — |
| Yield-bearing dollar (YLDS) | Figure's NAV feed | — | — |
| USDC | Reflector · RedStone | On-chain (SDEX/Soroswap if deep) | — |
| EURC | Reflector · RedStone | On-chain (if deep) | — |
| Lending tokens (Blend) | Blend contract rate | — | — |
| XLM | Reflector · RedStone | CoinGecko (if implemented) | — |

**Sources used:**
- **Reflector** — Stellar-native oracle, decentralized price feeds
- **RedStone** — Launched on Stellar March 2026; push oracle, cryptographically signed
- **Issuer NAV APIs** — Direct HTTP feeds with API keys; treat as trusted but verify

---

## SEP-40 Compatible Interface

The vault publishes its NAV through a SEP-40 compatible interface so lenders can integrate using the standard Stellar oracle pattern:

```rust
// SEP-40 compatible
fn price(asset: Address) -> Option<PriceData>
fn prices(assets: Vec<Address>, records: u32) -> Option<Vec<Option<PriceData>>>
fn lastprice(asset: Address) -> Option<PriceData>
fn resolution() -> u32  // in seconds
fn base() -> Address    // numéraire asset
fn decimals() -> u32
fn assets() -> Vec<Address>
```

**Lender recommended config (in listing_kit):**
- Use `collateral_nav()` for position sizing — not `price()`
- Minimum 2 sources required; refuse single-source configurations
- Alert threshold: 2% deviation between sources
- Staleness limit: 4 hours for fund tokens, 1 hour for stablecoins

---

## Deviation Threshold Configuration

Configured per-sleeve in the registry:

```json
{
  "sleeve": "USD_GOV_FUNDS",
  "deviation_threshold_bps": 200,
  "staleness_limit_seconds": 14400,
  "min_liquidity_depth_usd": 100000,
  "sources": ["FRANKLIN_BENJI_NAV", "REFLECTOR_BENJI"]
}
```

---

## Circuit Breaker Events (On-Chain)

Every circuit breaker event MUST emit an on-chain event:

```
Topic: ["NAV_PAUSE", sleeve_id]
Data: {
  reason: "DEVIATION" | "STALENESS" | "INSUFFICIENT_SOURCES",
  sleeve: sleeve_id,
  sources_valid: u32,
  sources_total: u32,
  deviation_bps: i128,  // if applicable
  timestamp: u64
}
```

---

## Implementation Checklist

Before any NAV oracle code ships to testnet:

- [ ] All 5 oracle rules implemented and enforced
- [ ] Unit tests for each circuit breaker scenario
- [ ] Property tests: collateral_nav ≤ deposit_nav for all valid inputs
- [ ] Deviation threshold configurable per-sleeve (not hardcoded)
- [ ] Staleness limit configurable per-source (not hardcoded)
- [ ] Circuit breaker emits on-chain events
- [ ] Hard stop (< 2 valid sources) tested end-to-end
- [ ] SEP-40 interface implemented and tested
- [ ] Collateral Passport view functions return valid data
- [ ] Lender listing kit documents our oracle configuration
- [ ] Independent review of oracle design before mainnet

---

## Anti-Patterns (Never Do These)

| Anti-pattern | Why |
|---|---|
| Pricing from a single on-chain market | YieldBlox incident — one trade can move a thin market 100x |
| Averaging all sources without liquidity check | A thin manipulated source contaminates the average |
| Using SDEX price for fund tokens | SDEX market for fund tokens is thin and manipulable |
| Hardcoding deviation thresholds | Different assets have different normal volatility |
| Ignoring staleness | A 24-hour-old price is not a price |
| Pausing the whole vault on one sleeve issue | Unnecessarily harms users in unaffected sleeves |
| Letting keepers override the oracle pause | The whole point is that oracle failures are hard stops |
