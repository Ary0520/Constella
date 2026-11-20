# Oracle Sources — Verified Integration Facts

> Verified 15 Nov 2026 for audit **O7** (real price sources). Each fact is marked with how it was checked:
> **[ONCHAIN]** read-only simulated calls to `mainnet.sorobanrpc.com` / `soroban-testnet.stellar.org`;
> **[SRC]** provider source code (repo/commit cited); **[DOC]** provider/official docs;
> **[CLI]** recomputed locally with stellar-cli 30.0.1. Anything not marked is unverified and must be
> re-checked before mainnet. Contract IDs below are inputs to configuration, never hard-coded in contracts.

---

## 1. Circle stablecoins on Stellar

| Asset | Network | Issuer (classic) [DOC] | Stellar Asset Contract (SAC) |
|---|---|---|---|
| USDC | mainnet | `GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN` | `CCW67TSZV3SSS2HXMBQ5JFGCKJNXKZM7UQUWUZPUTHXSTZLEO7SJMI75` [CLI] |
| USDC | testnet | `GBBD47IF6LWK7P7MDEVSCWR7DPUWV3NY3DTQEVFL4NAT4AQH3ZLLFLA5` | `CBIELTK6YBZJU5UP2WWQEUCYKLPU6AUNZ2BQ4WWFEIE3USCIHMXQDAMA` [CLI, stellar.expert] |
| EURC | mainnet | `GDHU6WRG4IEQXM5NZ4BMPKOXHW76MZM4Y2IEMFDVXBSDP6SJY4ITNPP2` | `CDTKPWPLOURQA2SGTKTUQOWRCBZEORB4BWBOMJ3D3ZTQQSGE5F6JBQLV` [CLI] |
| EURC | testnet | `GB3Q6QDZYTHWT7E5PVS3W7FUT5GVAFC5KSZFFLPU25GO7VTC3NM2ZTVO` | `CCUUDM434BMZMYWYDITHFXHDMIVTGGD6T2I5UKNX5BSLXLW7HVR4MCGZ` [CLI, stellar.expert] |

- Sources: developers.circle.com/stablecoins/usdc-contract-addresses, …/eurc-contract-addresses. SAC IDs are not published by Circle; recomputed with `stellar contract id asset` (two independent methods agree).
- 7 decimals (Stellar classic). **Beware look-alikes:** Reflector's pubnet feed also lists a different "EURC" (issuer `GAQRF…`, contract `CBVDRT54…`, ~0.557) [ONCHAIN]. Always key by the SAC address above.
- EURC "always redeemable 1:1 for euro" via Circle Mint (qualified businesses only) [DOC circle.com/eurc].

## 2. Reflector (Stellar-native, SEP-40)

**Shared [ONCHAIN]:** `version()` 7, `decimals()` **14**, `history_retention_period()` 86 400 s, prices = `price / 10^14`, base asset reads exactly 1. Timestamps in **seconds** on the resolution grid. Admin of each mainnet Pulse feed: multisig of 7 node keys, threshold 4 (operators: creit.tech, stellar.expert, publicnode.org, lightsail.network, xycloo.com, ultrastellar.com, script3.io) [ONCHAIN Horizon, orchestrator config].

| Feed (mainnet) | Contract | Base | Resolution | Keys | Derivation [SRC reflector-node / connectors] |
|---|---|---|---|---|---|
| Pulse — Stellar pubnet | `CALI2BYU2JE6WVRUFYTS6MSBNEHGJ35P4AVCZYF3B6QOE3QKOB2PLE6M` | `Stellar(USDC SAC)` | 300 s | `Asset::Stellar(addr)` | 5-min VWAP of SDEX trades + AMM pool reserves (Aqua, SushiSwap V3, classic LPs ≥ 100 units) — effectively **one source** |
| Pulse — external CEX/DEX | `CAFJZQWSED6YAWZU3GWRTOCNPPCGBN32L7QV43XX5LZLFTK6JLN34DLN` | `Other("USD")` | 300 s | `Asset::Other(Symbol)` | median of per-exchange 1-min-candle VWAPs (Binance, Bybit, Coinbase, Gate, Kraken, OKX) |
| Pulse — FX | `CBKGPWGKSKZF52CFHMTRR23TBWTPMRDIYZ4O2P5VS65BMHYH4DXMCJZC` | `Other("USD")` | 300 s | `Asset::Other(Symbol)` | ECB, NBP, commercial FX APIs (stale inputs dropped after 26 h–5 d) |

- Relevant keys: USDC = pubnet base / CEX `Other("USDC")`; EURC = pubnet `Stellar(CDTKPW…)` / CEX `Other("EURC")` (connector treats EURC/EUR/EURT as one); EUR = FX `Other("EUR")`; XLM = pubnet `Stellar(CAS3J7…)` / CEX `Other("XLM")`. **BENJI / USDY not found** in any feed.
- **Pulse** reads are free and take no `caller`. Each asset has an **expiry**; someone must burn XRF via `extend_asset_ttl(sponsor, asset, amount)` (200 XRF per asset per day on mainnet). **Expired assets return `None`** [ONCHAIN]. Production must budget and monitor asset expiry for every Reflector key we use.
- **Beam** reads require `caller.require_auth()` and prepaid `track()` access (40 XRF per asset per day); the mainnet Beam feed (`CCRVIC2R…`) had not updated for ~24 days on 15 Nov 2026. **Do not use Beam.**
- No on-chain TWAP; consumers compute from `prices(asset, n)` (newest first). `lastprice` returns the most recent non-zero record, which may be older than the current round → staleness check mandatory. Observed: `prices(asset, 3)` can return 4 records.
- Testnet: pubnet mirror mostly empty and a single-node cluster — no consensus guarantees on testnet.
- Audits: OtterSec 2024 (public feed), Code4rena 2025 (Beam/Pulse) — in `reflector-network/reflector-contract/audits`.

## 3. RedStone (push oracle on Stellar)

Read from `redstone-finance/redstone-oracles-monorepo` (main `39cce17`, 13 Nov 2026), `packages/stellar-connector` [SRC]; mainnet launch announced 4 Mar 2026 [DOC].

- **Adapter (mainnet)** `CA526Y2NQWGWVVQ7RFFPGAZMU66PSYJ3UC2MTVAV4ZU7OM5BOPHDXUSG`; **testnet** `CBIHT4HVRIT5OMVLSXZ44J2ZAXYBDDGOSCN3LTN2DOC6SWHDS5IP6BK3`.
- Per-feed contracts (mainnet): USDC `CC2GB2TZTX24XIAM375YDWUDXA3PR2GDMNFUL2Z5VNXUMKPPOBES3CXM`; XLM `CAXRDLYBL7ZNE754UX33XTVGTA7MNTIFWURTFJBTRKHPTNP2P36TU2ZF`; EUROC/EUR `CD4H2CN6ROCXVZFDTMLFCSU4O5CKLJP5BCFECNMNZEATNABEDRVGFKNI`; BENJI_ETHEREUM_FUNDAMENTAL `CCDM2U5E3SBMPJUQCBIER4BSFAVSJTY5I5EQEZIRDZSA3YOJH3YM4TW5`; USDY_FUNDAMENTAL/USD `CAEUQPFSHHZ2EH4RKMU6S2NEOZVMDMZZ47RRN5FZCAPDME5OACBHEOXM`. EURC/USD is feed id `"EUROC"` (adapter only, no per-feed contract).
- Native interface: `read_price_data_for_feed(feed_id: String) -> Result<PriceData{price: U256, package_timestamp: u64 (ms), write_timestamp: u64 (ms)}>`; per-feed `read_price_data()`, `decimals()` = **8** (hard-coded). Base **USD**.
- Trust: 3-of-5 RedStone signers verified on-chain; payload must be within −3 / +1 min of ledger time; trusted updaters (3 keys) push; anyone may push a valid payload after 2 days of silence. Triggers: USDC 0.1 % / 5 min; XLM 0.2 % / 10 min; EUROC/EUR 0.2 % / 4 min; EUROC 0.1 % / 12 h; BENJI 0.1 % / 24 h; USDY 0.5 % / 12 h.
- **Adapter reads fail only after 30 h** (`DATA_STALENESS`) — far looser than the heartbeats; consumers must apply their own limit to `package_timestamp`.
- SEP-40 wrapper `CBMGLKUQZVSAIL5CPDDAWSUY7MAKXISHMOZEVLMBUWBMFGHRJSR4WYRF`: base `Other("USD")`, `decimals()` **may change** when feeds are added, `resolution()` owner-changeable (86 400), `prices()` **oldest first**. Use the native interface instead.
- All RedStone contracts are owner-upgradeable; license BUSL-1.1; audits Veridise and Zenith (Oct 2025) predate the SEP-40 wrapper.
- **Verified by replay (15 Nov 2026):** two real mainnet payloads (USDC tx `4bed8f74…`, EUROC tx `089303b5…`) were replayed into the real adapter WASM (sha256 `f0ccdffc…`). Each feed carried 3 signer packages (USDC 0.99975479 / 0.99975978 / 0.99975978 → median 0.99975978). Corrupting one package (signature or signed value) made the adapter **silently skip that feed** while storing the others, so a write's success is per feed, not per transaction. Live mainnet EUROC at check time: 1.09853605 USD.

## 4. Blend v2 (lending positions)

[SRC blend-contracts-v2 `0bf9b74`, 26 Oct 2026; re-read 16 Nov 2026 for the Blend adapter] Supply positions ("bTokens") are **pool-internal balances, not SEP-41 tokens**; a vault must hold the position itself (`submit`). Value: `b_tokens × b_rate / 1e12` (floor), `get_reserve(asset)` returns a `b_rate` accrued to the current ledger (12 decimals); raw storage may lag.

- **Contracts [ONCHAIN + CLI, three-way hash check]:** IDs from blend-utils `mainnet.contracts.json` / `testnet.contracts.json` (commit `b05242d`). Mainnet pool factory v2 `CDSYOAVXFY7SM5S64IZPPPYB4GVGGLMQVFREPSQQEZVIWXX5R23G4QSU`, backstop v2 `CAQQR5SW…G3IM7`, emitter `CCOQM6S7…SRGRR`, Comet BLND:USDC `CAS3FL6T…RVEAM`, pools FixedV2 `CAJJZSGM…YBXBD` / YieldBloxV2 `CCCCIQSD…OGYFS`. Testnet factory v2 `CDV6RX4CGPCOKGTBFS52V3LMWQGZN3LCQTXF5RVPOOCG4XVMHXQ4NTF6`, pool `CCEBVDYM…44HGF` ([ONCHAIN 16 Nov 2026]: `is_pool` true, status 0, reserves XLM / wETH / wBTC / Blend-testnet USDC `CAQCFVLO…` — not Circle testnet USDC; mock oracle `CAZOKR2Y…`). The fetched mainnet WASMs equal blend-utils' published `wasm_v2/` builds and the testnet hash list (pool `a41fc53d…` for every v2 pool, backstop `c1f4502a…`, factory `31328050…`, emitter `438a5528…`, comet `8abc2891…`; Blend's repo `comet.wasm` is a different build). Details: `contracts/testkit/testdata/blend/README.md`.
- **`b_rate` moves only two ways [SRC]:** up by time-based interest accrual (`reserve.rs` `accrue`, minus the backstop take), down only when bad debt is written off (`user.rs` `default_liabilities` — suppliers lose). **Donations do not move it:** `gulp` credits surplus tokens to `backstop_credit`. So a position cannot be inflated within a transaction.
- **Requests [SRC `actions.rs`]:** Supply 0, Withdraw 1, SupplyCollateral 2, WithdrawCollateral 3, Borrow 4, Repay 5. Plain supply has no health factor. Supply: the pool calls `token.transfer(spender → pool)` (the vault must pre-authorize it), blocked only if the pool is frozen (status > 3) or the reserve's `supply_cap` is hit. Withdraw (in underlying units; burns `ceil(amount / b_rate)` bTokens; more than the position pays the whole position) is **never** blocked by pool status but fails with `InvalidUtilRate` (1207) if utilization would reach 100 % — the real liquidity risk.
- **Pool trust:** the factory's `is_pool(pool)` is true only for pools it deployed, all running the pool WASM fixed in its constructor (`PoolInitMeta`). New pools start in status 6 (setup); a live pool's reserve changes wait **one week** (`config.rs`).
- **Consumer behaviour [SRC `pool.rs` `load_price`]:** `lastprice(Asset::Stellar)` + `decimals()`; a returned price older than 24 h or ≤ 0 → `InvalidPrice` (1210); a **`None` aborts via `unwrap_optimized()`** (trap, not 1210). Either way Blend refuses — verified against the real pool WASM (D-H tests).

## 5. YieldBlox / USTRY incident (22 Feb 2026) — binding lessons

A 0.05-unit trade (~5 USDC) on a single-market-maker SDEX pair set Reflector's pubnet VWAP for USTRY to $106.74 (from ~$1.06); the pre-fix node carried it forward one more round; the pool's adapter compared consecutive rounds (identical → 0 % change) and passed it; ~$10–11 M borrowed (rekt.news, BlockSec, Halborn, Script3; Reflector node commits verified). Lessons applied in Constella:

1. Never trust one round: median over **≥ 3** Reflector rounds, staleness on the oldest used.
2. Consecutive-round change checks are insufficient; Constella's breaker compares **independent sources** (e.g. issuer NAV vs market), which would have disagreed by ~100×.
3. Per-asset hard price bands / max change (audit **O5**, needs owner decision).
4. Thin SDEX-derived feeds only alongside an independent source; minimum 2 valid sources (Rule 5).
5. Fail closed (`None` / halt), never clamp.
6. The adapter is part of the audited protocol: Constella's oracle **is** the adapter and is in scope.
