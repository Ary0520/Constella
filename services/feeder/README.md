# Constella price feeder

Pushes prices that Constella derives itself to the NAV oracle's **pushed sources** (audit O7). Each source has
its own feeder key, and only that key can write the source's price; the oracle admin cannot (O7a).

## Source 1: `ECB_EURC`, EURC priced in USDC from the ECB reference rate

- **Method:** a *fundamental* price. Circle redeems EURC 1:1 for EUR and USDC 1:1 for USD, so EURC in USDC is USD per EUR.
  - The ECB rate is the primary, official, public source (`eurofxref-daily.xml`, no API key).
  - This is independent of market feeds (Reflector/RedStone). If either stablecoin depegs, the market sources diverge from this value and the oracle's deviation breaker halts the sleeve.
- **Timestamp:** the ECB concertation the rate refers to, 14:15 Europe/Berlin (CET/CEST aware).
- **Cadence:** one new rate per TARGET business day, with none on weekends or TARGET holidays. Register the source with `staleness_limit_seconds = 432000` (5 days), which covers Easter (Thu → Tue).
- **Units:** oracle units have 7 decimals, rounding **down**, so 1.1052 becomes `11052000`. Liquidity depth is `0` (no market depth), so register the source with `min_liquidity_depth_usd = 0`.

## Safety gates (nothing is submitted when a gate refuses)

| Gate | Rule |
|---|---|
| Sanity band | EUR/USD must be within 0.70–1.80; it has traded 0.82–1.60 since 1999 |
| Jump guard | A move of more than 500 bps versus the on-chain price is refused (exit 2) and needs a human; the source then goes stale and fails closed |
| Ordering | Data older than on-chain is refused; identical data is skipped (idempotent); future-dated data is refused |

## Running

```bash
pip install -e .            # Python 3.11+, stellar-sdk 17.0.1 (pinned)
export CONSTELLA_NETWORK=testnet          # testnet | mainnet | local
export CONSTELLA_ORACLE_ID=C...           # deployed oracle
export FEEDER_SOURCE_ID=ECB_EURC
export FEEDER_SECRET=S...                 # this source's feeder key ONLY; from a secret store, never committed or logged
constella-feeder                          # one run: SUBMIT / SKIP (exit 0), REFUSED (exit 2), ERROR (exit 1)
```

**Schedule:** run it every 15–30 minutes, e.g. cron or a systemd timer. Re-runs are idempotent, so the frequency only affects how quickly a new ECB rate lands on-chain.

**Keys:**
- Use a dedicated key per source, funded only for fees.
- Rotating it is an admin `register_source` with the new `feeder`, and it takes effect after the 48 h source-change delay. The old key keeps working until then.
- Alert on exit code 2 (REFUSED) and on repeated exit 1.

## Testnet (real, not simulated)

On testnet, EURC gets two genuinely independent real sources:
1. **Reflector testnet CEX/DEX** (`CCYOZJCO…RN63`), as an O7b pull source with `Other("EURC")` via `Other("USDC")`.
2. **This feeder** (`ECB_EURC`), pushing the live ECB rate with its own testnet key.

See `docs/build-spec/references/TESTNET-READINESS.md`.

## Tests

- `pytest`: unit tests on a real ECB document (`tests/fixtures/ecb_eurofxref_daily_20261115.xml`, fetched 15 Nov 2026).
- `tests/localnet_e2e.sh` (from the repo root, after `stellar contract build`): starts a real local Stellar network (`stellar/quickstart`, protocol 30, the same as testnet), deploys Constella's release oracle WASM and real USDC/EURC asset contracts, registers `ECB_EURC`, then runs the feeder. It checks SUBMIT (price and timestamp on-chain), SKIP on re-run, REFUSED on a +17.6 % jump, and that the admin cannot write a price.
