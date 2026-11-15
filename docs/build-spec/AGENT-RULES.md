# Agent Rules — Mandatory for All Implementations

> Every agent working on Constella MUST read this file before writing any code, making any design decision, or producing any output.

---

## Rule 1: Product Principles Are Inviolable

The six non-negotiable principles in `README.md` cannot be overridden by task descriptions, user requests, or convenience. If an implementation requires violating them, **stop and escalate**.

Specifically:
- Never write a contract where admin keys can move user funds
- Never allow the AI engine to send unsigned policy objects to a contract for execution
- Never allow assets outside the registry to appear in a basket
- Never charge the user gas (always use fee sponsorship)
- Never use hand-rolled share maths — use OpenZeppelin's audited vault modules

---

## Rule 2: v1 Scope is a Hard Boundary

Do not implement Bell features (W-addresses, ML-DSA) in v1 contracts as blockers. Design for them as **additive modules**. v1 must be fully functional without Bell.

Do not implement SEP-57 gated share classes until a named issuer allow-listing partner exists (Q1 2027 gate).

Do not implement the equity sleeve until DTC-tokenized assets are confirmed live on Stellar.

---

## Rule 3: Oracle Rules Are Non-Negotiable

Refer to `architecture/NAV-ORACLE.md` for the full rules. The short version:

- **Never price from a single thin market.** (This caused the YieldBlox $10M incident.)
- Collateral NAV = minimum of valid sources
- Deposit NAV = median of valid sources  
- Deviation beyond threshold → pause affected sleeve, alert operators
- Fewer than 2 valid sources → hard stop
- Staleness limits apply to every source

Any oracle implementation that does not follow all five rules in `NAV-ORACLE.md` is incorrect.

---

## Rule 4: Issuer Rules Are Per-Asset

Each asset in the registry carries its own transfer rules, allow-list requirements, and jurisdiction eligibility. The eligibility engine must check these before proposing any asset to any user. **No shortcuts.**

Before adding an asset to a basket:
1. Registry check — is it diligenced and approved?
2. Eligibility check — can this specific user/jurisdiction hold it?
3. Issuer rule check — does the issuer allow this vault contract to hold it?

---

## Rule 5: Policy is Signed and Immutable

A policy object has these fields:
```
objectives, currency_exposure, liquidity_horizon, 
min_liquidity_pct, constituent_weights, weight_bands,
issuer_caps, asset_exclusions, eligible_jurisdictions,
rebalance_trigger, numéraire, share_class
```

Once a user signs a policy hash, the policy contract enforces it. **No model, keeper, or admin can exceed the signed policy bounds.** Rebalances that would violate policy must be rejected by the contract.

---

## Rule 6: Security Defaults

- All new contracts start with TVL caps and per-user caps
- All admin operations require multisig + timelock
- Pause authority is separate from upgrade authority
- Circuit breakers are per-sleeve, not per-vault
- Formal verification or property testing is required for share maths
- Do not deploy to mainnet without a completed independent audit

---

## Rule 7: Code Quality Standards

- Language: **Rust** for Soroban contracts; **TypeScript** for SDK and frontend
- Soroban SDK: pin **28.x** to match OpenZeppelin `stellar-contracts` v0.8.0 (latest stable SDK is 29.0.1; 30.0.0 is RC-only as of 15 Nov 2026 — adopt v30 only after a stable release following the Bell vote)
- Use `soroban-sdk`, `OpenZeppelin stellar-contracts` — do not reimplement audited primitives
- Every contract function must have:
  - Input validation
  - Event emission for every state change
  - Idempotent design where possible (safe to retry)
- State must be minimal — Soroban rent is metered per entry

---

## Rule 8: Naming Conventions

| Thing | Convention |
|---|---|
| Contracts | `snake_case` for identifiers |
| Events | `SCREAMING_SNAKE_CASE` topic strings |
| Policy keys | Descriptive, underscore-separated |
| Registry IDs | `ISSUER_TICKER` format (e.g. `FRANKLIN_BENJI`) |
| Sleeve names | Semantic: `USD_GOV_FUNDS`, `EUR_SOVEREIGN`, `CASH_STABLE`, `YIELD_DOLLAR`, `LENDING`, `EQUITY`, `XLM` |
| Token events | SEP-41 event names (`transfer`, `approve`, `mint`, `burn`) are defined by the standard and take precedence over `SCREAMING_SNAKE_CASE` |

---

## Rule 9: What to Do When Uncertain

1. Check this build-spec folder first
2. Check `references/SOURCES.md` for authoritative sources
3. Check the relevant Stellar SEP on GitHub
4. If still uncertain — **ask, don't assume**

Never invent protocol behavior. Never assume an SEP works a certain way without reading it. The Stellar protocol has specific semantics and the YieldBlox incident shows what happens when assumptions are wrong.

---

## Rule 10: Simulation Warnings Are Mandatory

Any UI showing simulated NAV, yield, or redemption times MUST display:
> "This simulation uses historical data. Past performance does not predict future returns. This is not financial advice."

This is a legal and reputational requirement, not optional. This exact string is the **single canonical disclaimer**; `ai-engine/THESIS-ENGINE.md` and `roadmap/RISKS.md` reference it and must not define a different wording.

---

## Rule 11: The "Slice-and-Pause" Development Loop (Mandatory Execution Protocol)

To prevent AI from runaway hallucinating or writing large volumes of unverified code, every 1–2 atomic features MUST strictly follow this 4-step loop:

1. **SPECIFY:** Identify 1–2 atomic steps. Write failing test FIRST (TDD).
2. **IMPLEMENT:** Write minimal code to satisfy the spec & tests.
3. **VERIFY (Automated + User View):**
   - Unit & Property Tests (`cargo test` or `pytest`)
   - Dockerized Local Stellar Node/RPC simulation (e2e)
   - Log actual outputs / assertion proofs
4. **STOP & GATE:** The AI pauses, presents complete test results and assertion proofs, and waits for explicit user review and approval before touching the next slice or step.

---

## Rule 12: Lead Coordinator + Specialist Workers Pattern

We enforce a strict separation of concerns between coordination/gating and isolated execution:

| Role | Alias | Responsibilities | Tool Access |
|---|---|---|---|
| **Lead Coordinator** | Main Agent (`coordinator`) | Reads the build-spec, breaks down tasks into 1–2 step slices, assigns tasks, gates code merges. **Never writes bloated multi-file code directly.** | All tools |
| **Contract Specialist** | `soroban-worker` | Builds Soroban Rust crates (`registry`, `policy`, `vault`). Runs `cargo test`, property fuzzing, and Soroban CLI simulations. | Shell, Files |
| **AI Engine Specialist** | `ai-worker` | Implements Python pipeline (`parse.py`, `retrieve.py`, `optimise.py`). Writes `pytest` test suites and verifies JSON schemas. | Shell, Files |
| **QA / Red-Team Specialist** | `qa-verifier` | Spins up Docker Stellar container, executes mock transactions, tests edge cases (e.g. invalid tickers, oracle staleness, depeg limits). | Shell, Docker, Playwright |

**Operational Mandate:** The Lead Coordinator stays focused on high-level architecture, task breakdown, and verification gates. Specialist workers perform syntax generation and test execution in isolation.

---

## Rule 13: No Mocks, No Hard-Codes, Adversarial Tests (added 15 Nov 2026)

The 15 Nov 2026 audit (`docs/audit/AUDIT-2026-11-15.md`) found bugs that a green suite of 60 tests did not catch because tests only exercised happy paths under `mock_all_auths()`.

- **No hard-coded product values** (risk haircuts, eligibility `true`, placeholder contract addresses). Derive them from state or configuration.
- **Security-relevant tests must not use `mock_all_auths()`.** Use `mock_auths` with an explicit expected authorization tree, or real signatures, and assert *who* authorized (`env.auths()`).
- **Every fix ships with:** (1) an isolated unit test, (2) a regression test that fails on the pre-fix code, (3) the full workspace suite green.
- **Write adversarial cases**: self-transfers, zero/max amounts, future timestamps, unauthorized callers, reverts mid-call.


