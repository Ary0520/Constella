# Constella keeper

Keeps each vault at its **signed** targets and settles queued frozen-sleeve claims.
The keeper proposes; it never decides what is allowed.

Per vault, per tick:
1. Skip unless this key is the vault's keeper (`get_keeper`) and the vault is not paused.
2. **Plan** (`planner.py`, pure): values every sleeve exactly like the vault (`sleeve_value`:
   cash = units, others = units × oracle `asset_price` / 10^decimals, holdings = min(booked, real)).
   Nothing happens while every sleeve is within its band. Trades go through the vault currency
   (sells first; buys only with cash the vault will surely have). Each leg's `min_amount_out` is the
   **smallest value the policy accepts** (policy step 9: oracle prices, token decimals, max slippage) —
   and if the Soroswap quote is below it, the plan is refused rather than trading against an
   off-oracle pool. Blend legs use the Blend route (no AMM quote).
3. Pre-validate with `policy.validate_rebalance` (read) and log any rejection code.
4. Submit `rebalance`: simulated first — the vault re-checks its **actual** holdings against the
   policy after the trades — and nothing is sent if the simulation fails.
5. Storage rent (A1): contracts renew what a transaction touches; each tick the keeper renews whatever is
   idle - the vault / policy / oracle / registry instances, their WASM code, the vault's policy records and
   open claims - with one permissionless `ExtendFootprintTTL` when fewer than `KEEPER_TTL_MIN_DAYS` (20)
   remain, to `KEEPER_TTL_EXTEND_DAYS` (30). Idle user share balances are restored automatically by the next
   transaction that needs them (at a fee).
6. Settle claims: `CLAIM_RECORDED` events (from the RPC's oldest retained ledger, then a saved
   cursor) → `settle_claim` (permissionless, pays only the holder) once the asset is priced again;
   otherwise retried next tick. The keeper must run at least once per RPC retention window.

## Run

```
CONSTELLA_RPC_URL=… CONSTELLA_NETWORK_PASSPHRASE=… KEEPER_SECRET=S… KEEPER_VAULTS=C…,C… constella-keeper
```
Optional: `KEEPER_INTERVAL_SECONDS` (300), `KEEPER_ONCE=1`, `KEEPER_DRY_RUN=1`, `KEEPER_STATE_FILE`,
`KEEPER_DEADLINE_SECONDS` (300, from ledger close time). The keeper key needs XLM for fees only;
it cannot move funds outside policy-validated rebalances.

## Tests

- `pytest` — planner (policy-exact minimums across decimals, off-oracle refusal, cash-limited buys, Blend route, fail-closed).
- `tests/localnet_e2e.sh` — real local network (protocol 30), release WASMs + real mainnet Soroswap WASMs:
  deposit → rebalance to 14.94 % EURC; in band → nothing; pool pushed off oracle → refused, nothing
  sent; wrong key → skip; frozen EURC → user redeems (cash + claim), price back → keeper settles the
  claim (paid at the oracle price); the keeper renews 10 idle entries (instances, code, policy records) to 40 days; paused → skip.
