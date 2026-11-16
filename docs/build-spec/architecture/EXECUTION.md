# Execution & Rebalancing Specification

> Reference: Product doc §4.5 Execution and Rebalancing

---

## Execution Philosophy

Rebalancing costs money (gas, spread, slippage). Every rebalance that doesn't happen saves users money. The execution layer is designed so that **most days require no trading at all**.

The order of preference:
1. **Flow-based routing** — route deposits/withdrawals to fix imbalances (zero trading cost)
2. **Issuer mint/redeem** — use primary market at NAV (no spread)
3. **AMMs with caps** — Soroswap, Aquarius, SDEX for small residuals (spread applies)

---

## Rebalance Trigger Logic

### Step 1: Flow-Based Netting (Daily, Continuous)
```
On every deposit:
  → Deposit amount goes to the most underweight sleeve(s)
  → No trading required

On every withdrawal:
  → Withdrawal amount comes from the most overweight sleeve(s)
  → No trading required
```

This alone eliminates most rebalance events. Users naturally bring the basket to target weights over time.

### Step 2: Band Monitoring (Keeper, Periodic Check)
```
Every period (configurable, default: hourly):
  For each sleeve in vault:
    if sleeve.weight < sleeve.min_band OR sleeve.weight > sleeve.max_band:
      → Trigger band-breach rebalance proposal
```

Band breach rebalance:
1. Keeper computes minimum trades to bring all sleeves within bands
2. Keeper submits rebalance proposal to policy contract
3. Policy contract validates: within policy bounds, slippage limits, liquidity floor preserved
4. If valid: execution proceeds
5. If invalid: rejected, keeper logs failure, alert sent

### Step 3: Policy Contract Validation (Every Rebalance)
Policy contract checks the proposal against:
- [ ] Weight bands (post-trade weights within min/max for all sleeves)
- [ ] Issuer caps (no issuer exceeds its cap post-trade)
- [ ] Liquidity floor (liquidity sleeve stays ≥ minimum)
- [x] Asset exclusions (no excluded asset is bought) — enforced as an allow-list: only assets declared in the signed sleeves (non-zero band) may be bought (audit M1)
- [x] Proposal names the current policy hash; proposals built for an amended policy are void (audit M1)
- [ ] Slippage cap (estimated slippage ≤ cap per route)
- [ ] Eligible assets only (all assets in registry and eligible)

If ANY check fails: **rebalance rejected in its entirety**.

---

## Implementation status (audit W1a `c732a97`, W1b `b1faf5d`)
- `vault.rebalance(proposal)`: only the designated **keeper** (set by the admin) submits; a paused vault refuses; the signed policy's `validate_rebalance` must accept the proposal.
- **Soroswap legs:** the vault authorizes exactly one `token.transfer(vault → pair, amount_in)` for the router. Spent and received are **measured on the vault's own balances**, so a lying or compromised router cannot over-spend or under-deliver: the vault itself enforces `spent ≤ amount_in` and `received ≥ min_amount_out`. Holdings are booked from those measured deltas only. Events: `REBALANCE_STEP`, `REBALANCE_COMPLETE`.
- **Routers:** set with `propose_router` and active only after **48 h** (`activate_router`), because a router can move vault funds.
- **Post-trade check (W1b):** the proposal's `weights_after` is only a claim. After the swaps, the vault values every sleeve as **actually held** (measured balance, capped by the booked balance, at its own asset's median price; cash at face value) and the policy's `validate_holdings` must accept the result: every signed band, single-asset cap, issuer caps and the cash liquidity floor, compared exactly. Otherwise the whole rebalance reverts with `AllocationOutOfPolicy`, swaps included. A sleeve without a valid price stops it with `OracleUnavailable`.
- **Targets (W1b):** once a policy is bound, `set_sleeves` accepts only sleeves declared in it with exactly the signed target weight (`SleeveTargetMismatch`). Targets are read **live** from the policy (`get_sleeves`, empty-vault risk sheet), so an activated amendment applies without re-running `set_sleeves`. If an amendment drops a sleeve the vault still holds, the next rebalance must sell it to zero (the post-trade check rejects undeclared holdings).
- **Blend adapter (decision D-H, `000f40a`):** route `Blend`. A `Lending` sleeve whose address the oracle knows as a Blend position (official-factory pools only) is held as **plain-supply bTokens** (never collateral: the vault never borrows, so it cannot be liquidated). Supply authorizes exactly one `transfer(vault → pool, amount_in)`; withdraw converts bTokens to underlying at the live `b_rate`, rounded down so Blend burns ≤ `amount_in`. Spent and received are measured on the vault's token balance and its bToken position; the other leg must be the position's own underlying; Soroswap never routes a Blend position. Liquidity risk is real: Blend refuses a withdraw that would take utilization to 100 % (`InvalidUtilRate` 1207) and the whole rebalance reverts — cash for redemptions must come from the cash sleeve, as the liquidity floor requires.
- **Not yet:** Aquarius, SDEX and issuer routes (refused with `RouteUnavailable`); DeFindex.

## Execution Order Within a Rebalance

```
1. Issuer redemptions (for assets being reduced)
   └─ Call issuer mint/redeem API/contract
   └─ Receive numéraire (USDC/EURC)

2. AMM/SDEX trades (small residuals, if needed)
   └─ Soroswap → Aquarius → SDEX (in that order of preference)
   └─ Each route: check liquidity depth; skip if too thin
   └─ Enforce slippage cap per route

3. Issuer subscriptions (for assets being increased)
   └─ Call issuer mint/redeem API/contract
   └─ Receive new asset tokens

4. Verify: post-trade weights are within bands
5. Emit: REBALANCE_COMPLETE event with trade details
```

**Atomic where possible:** Soroban allows one contract call per transaction. Multi-step rebalances are:
- Sequenced (not all in one transaction)
- Idempotent (safe to retry on failure)
- State-tracked (each step is committed before next begins)
- Safe-resume-on-failure (keeper can resume from last committed step)

---

## Issuer Mint/Redeem Adapters

Each issuer has an adapter contract that abstracts their specific redemption mechanism:

```rust
trait IssuerAdapter {
    fn subscribe(amount: i128, numéraire: Address) -> Result<(), Error>;
    fn redeem(shares: i128) -> Result<(), Error>;
    fn pending_redemption() -> Option<(i128, u64)>; // (amount, expected_settle_time)
    fn instant_capacity() -> i128; // how much can be redeemed instantly
}
```

**Adapters to build (in priority order):**
1. USDC/EURC (direct — no adapter needed)
2. Blend supply/withdraw adapter ✅ (D-H, `000f40a`)
3. DeFindex adapter
4. Spiko (if API available)
5. Franklin BENJI (if allow-listed)
6. Ondo USDY (if allow-listed)
7. Figure YLDS (if allow-listed)

---

## AMM Routes and Slippage Caps

| Venue | Priority | Slippage Cap | Notes |
|---|---|---|---|
| Soroswap | 1 | 0.5% | Most liquid Stellar AMM |
| Aquarius | 2 | 0.5% | Secondary |
| SDEX | 3 | 0.3% | Classic DEX; thin for most assets |

**Rules:**
- Slippage is measured in value against the oracle's fair price for each asset (audit P1), so cross-currency legs (e.g. USDC ↔ EURC) are judged on real loss, not token counts
- If a route would exceed the slippage cap: skip that route, try next
- If all routes exceed cap: **abort that trade leg**, leave sleeve slightly out-of-band, alert operators
- Never force a trade into a thin market
- DEX TVL context: Stellar DEX total ~$16M, Soroswap ~$1.2M — these caps are realistic and necessary

---

## Keeper Network

**What is a keeper?**
A keeper is an off-chain bot that:
1. Monitors vault weight drift
2. Proposes rebalances to the policy contract
3. Executes approved rebalances
4. Monitors oracle validity and circuit breakers
5. Extends Soroban state TTLs (rent management)

**Keeper implementation:**
- TypeScript process (runs continuously)
- Submits transactions signed by a dedicated keeper keypair
- Keeper keypair has no admin authority — it can only propose rebalances
- Policy contract rejects any unauthorized operation
- Multiple keeper instances can run (idempotent design means no double-execution)

**Keeper is NOT trusted with:**
- Policy changes
- Admin operations
- User funds
- Oracle source configuration

---

## Parallel Execution

Benefit from Protocol 23 (Whisk) parallel execution:
- Split sleeves across separate contracts where possible
- Independent sleeves can be rebalanced in parallel
- Avoids serialization bottlenecks for large vaults

---

## Rebalance Events

Every rebalance step emits an on-chain event:

```
Topic: ["REBALANCE_STEP", vault_id, step_type]
Data: {
  sleeve: sleeve_id,
  asset: asset_address,
  action: "BUY" | "SELL" | "MINT" | "REDEEM",
  amount: i128,
  price: i128,
  route: "ISSUER" | "SOROSWAP" | "AQUARIUS" | "SDEX",
  slippage_bps: i128,
  timestamp: u64
}
```

```
Topic: ["REBALANCE_COMPLETE", vault_id]
Data: {
  weights_before: Map<sleeve_id, u32>,
  weights_after: Map<sleeve_id, u32>,
  total_trades: u32,
  timestamp: u64
}
```

---

## Implementation Checklist

- [ ] Flow-based deposit routing implemented
- [ ] Flow-based withdrawal routing implemented
- [ ] Band monitoring in keeper (configurable period)
- [ ] Policy contract validation of all rebalance proposals
- [ ] All 7 policy checks implemented in policy contract
- [ ] Issuer adapter interface defined
- [ ] USDC/EURC adapter (trivial — direct)
- [x] Blend supply/withdraw adapter (D-H, `000f40a`)
- [ ] DeFindex adapter
- [ ] AMM routing with slippage caps
- [ ] Route fallback logic (Soroswap → Aquarius → SDEX → abort)
- [ ] Idempotent step execution with state tracking
- [ ] Safe-resume-on-failure logic
- [ ] Rebalance events emitted
- [ ] TTL extension keeper
- [ ] Keeper keypair has no admin authority (enforced in policy contract)
