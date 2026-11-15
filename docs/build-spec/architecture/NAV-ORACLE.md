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

### Who can move a price (audit O7a, 15 Nov 2026)
- Each pushed source has its own **feeder** key, and only that key can call `update_price` for the source. The admin cannot write prices.
- **Delayed changes:** any change to the sources of an asset that already has a valid price waits **48 hours**, publicly (`SOURCE_CHANGE_PROPOSED`). This covers adding a source and changing an existing one (feeder rotation, looser staleness or depth limits). Anyone may activate the change after the delay; the admin may cancel it.
- Bootstrapping an asset that has no price yet is immediate, because nothing can consume a price that does not exist.
- Registration never sets a price: activating a change to an existing source keeps its price, timestamp and depth.
- `deactivate_source` is immediate (it can only halt pricing).
- Verified provider facts for the pull sources (Reflector, RedStone) are in `references/ORACLE-SOURCES.md`.

### Constella feeder service (audit O7d)
- `services/feeder` pushes Constella-derived prices with each source's own feeder key.
- **`ECB_EURC`:** EURC in USDC = ECB EUR/USD reference rate (Circle par redemption), timestamped at the ECB concertation (14:15 Europe/Berlin). Staleness limit 5 days (weekends and holidays).
- **Gates:** sanity band, 500 bps jump guard (refuse and alert), ordering, idempotence.
- **Independence:** this is a fundamental value, independent of market feeds, so a stablecoin depeg trips the cross-source deviation breaker.

### Reflector pull sources (audit O7b, `1e2c5ff`)
- `register_reflector_source(source, ReflectorConfig { contract, asset, quote, rounds })`: the source's price is read live from a Reflector Pulse feed. It goes through the same immediate-or-48 h rule as pushed sources and cannot be pushed into.
- **Rounds:** the median of the newest `rounds` records (3–20). Fewer real records (gaps) mean the source is invalid. Staleness is measured on the **oldest** round used.
- **Conversion** (rounding down):
  - `InBase`, for a feed whose base is our numéraire (e.g. Reflector pubnet, base USDC): `median × 10^ours / 10^feed_decimals`.
  - `ViaUsdc(key)`, for USD-based feeds (Reflector CEX/DEX, FX): `median(asset) × 10^ours / median(USDC)`, using the same feed's USDC key. USD is never assumed to equal USDC, and the cross rate is as stale as its stalest leg.
  - Overflow invalidates the source; it never wraps.
- **Validated at registration against the live feed:** decimals, quoted asset, base or USDC key, round bounds. A depth floor must be 0, because Reflector carries no depth.
- **Thin markets:** Reflector's pubnet feed is an SDEX/AMM VWAP. Use it only alongside an independent source; the cross-source deviation breaker then fails closed on manipulation (YieldBlox lesson, `references/ORACLE-SOURCES.md` §5).
- **Operations:** Reflector assets expire unless renewed with XRF, and an expired asset reads as invalid. Monitor and fund every key used.

### RedStone pull sources (audit O7c, `d8ed508`)
- `register_redstone_source(source, RedStoneConfig { adapter, feed_id, usdc_feed_id })`: the price is read live through `read_price_data_for_feed`.
- **Conversion:** `price(asset) × 10^ours / price(USDC)` using the same adapter's USDC feed (8 decimals cancel), rounding down. There is no in-base mode, because RedStone is USD-based.
- **Timestamp:** the **older signing time** (`package_timestamp` ms → s). The source is invalid if a leg is future-dated or older than the source's own staleness limit; RedStone itself keeps serving data for 30 h.
- **Registration:** both feeds must be readable now. RedStone's EURC/USD id is `"EUROC"`.
- **Observed adapter behaviour:** signatures are verified **per feed**. A feed without 3 valid signer signatures is silently not stored while the rest of the write succeeds, so a successful `write_prices` does not mean every feed updated. The stale feed then reads as invalid here.

### Per-asset evaluation (audit O6, 15 Nov 2026)
- Every rule below is evaluated **per asset**, among that asset's own sources. A sleeve may hold several assets at different prices (USDC and EURC in cash; BENJI and USDY in USD gov funds), and comparing them would be meaningless.
- The **sleeve** is the configuration and pause domain: deviation threshold, minimum valid sources, and the pause flag / `NAV_PAUSE` event.
- An asset belongs to exactly one sleeve, and a source ID cannot be re-registered for another asset or sleeve (`AssetSleeveMismatch`).
- Per-asset reads: `asset_price(asset)` (median, deposit/fair) and `price(asset)` (minimum, collateral).
- Sleeve-level `get_deposit_nav` / `get_collateral_nav` return a price only for a single-asset sleeve, and return `None` otherwise. Reads never pause a sleeve because of other assets.

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

## Production Price Flow (target design — not yet built, audit O7)

```
 Reflector (Stellar oracle network) ──┐
 RedStone  (Stellar oracle network) ──┼──► NAV Oracle contract ──► Strategy Vault ──► share prices
 Blend contract (supply exchange rate)┤    (2+ sources, staleness,                  │
 Issuer NAV APIs ──► feeder service ──┘     deviation, min/median)                  ▼
   (Franklin, Spiko, Ondo)  per-source key,                              Blend / Liqvid read the
                            issuer signature verified where available    conservative price (SEP-40)
```

- **On-chain providers** (Reflector, RedStone, Blend) are read directly by the oracle contract; no one pushes these prices.
- **Issuer NAVs** originate off-chain and are carried in by a Constella feeder service signing with a dedicated key per source — never the admin key. Where an issuer signs its NAV, the contract verifies that signature.
- **Current state (15 Nov 2026):** the safety rules are implemented and tested, but every source is written via admin-only `update_price`. Tracked as audit O7.

## SEP-40 Compatible Interface

The oracle implements the exact SEP-40 interface (audit W5). It is verified against `sep-40-oracle` 1.4.0, which Blend v2 uses, and against Reflector's conventions:

```rust
enum Asset { Stellar(Address), Other(Symbol) }
struct PriceData { price: i128, timestamp: u64 }   // exactly these two fields
fn base() -> Asset; fn assets() -> Vec<Asset>; fn decimals() -> u32; fn resolution() -> u32 /* seconds */
fn lastprice(asset: Asset) -> Option<PriceData>    // live conservative price; None if no valid price
fn price(asset: Asset, timestamp: u64) -> Option<PriceData>       // history, floored to the grid
fn prices(asset: Asset, records: u32) -> Option<Vec<PriceData>>   // newest first, 1..=20
```

**Blend v2** calls only `lastprice(Asset::Stellar(token))` and `decimals()`. It rejects `None`, prices ≤ 0, and prices older than 24 h. Constella returns `None` earlier, as soon as a source passes its own staleness limit, so Blend halts instead of lending against stale data.

**Vault share tokens are not quoted yet** (audit L1); see `SEPS.md`.

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
- [x] SEP-40 interface implemented and tested (W5; Blend v2 `load_price` mirrored in tests; real Blend WASM on the local network still to do)
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
