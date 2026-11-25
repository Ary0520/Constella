# AI Thesis Engine — Specification

> Reference: Product doc §4.7; Previous run `07-ai-engine/thesis-engine-spec.md`

---

## Overview

The thesis engine is the front-door innovation of Constella. It converts a plain-English statement from a user into a typed, constraint-bound policy object that can be signed and executed on-chain.

**Critical rule:** The model has **zero on-chain authority**. It proposes. The user signs. The contract enforces.

---

## Pipeline (Six Steps)

```
User text → [1. Parse] → PolicyDraft
                      → [2. Retrieve] → Candidate assets (from registry ONLY)
                                     → [3. Optimise] → Weights + bands
                                                     → [4. Simulate] → Report
                                                                     → [5. Explain] → Diff
                                                                                    → [6. Sign] → Policy hash on-chain
```

---

### Step 1: Parse
**Input:** Plain-English thesis text from the user  
**Output:** Typed `PolicyDraft` object (schema-constrained function calling)

```python
class PolicyDraft(BaseModel):
    objectives: List[str]           # e.g., ["preserve capital", "match euro payroll"]
    currency_exposure: Dict[str, float]  # e.g., {"USD": 0.80, "EUR": 0.20}
    liquidity_horizon_hours: int    # e.g., 24 (exit within 24h)
    max_issuer_weight: float        # e.g., 0.30 (no issuer > 30%)
    max_single_asset_weight: float  # e.g., 0.25
    risk_tolerance: str             # "conservative" | "moderate" | "growth"
    exclusions: List[str]           # e.g., ["equity", "illiquid"]
    min_liquidity_sleeve: float     # e.g., 0.10
    target_yield_orientation: str   # "maximize" | "secondary" | "none"
    investor_class: str             # "retail" | "professional" | "institutional"
    jurisdiction: str               # e.g., "IN", "EU", "non-US"
    confidence: Dict[str, float]    # confidence score per field
    ambiguities: List[str]          # fields with low confidence, flagged for user
```

**User caps (audit A2, implemented):** caps the user states ("no more than 40% per issuer", "no single asset above 15%") are carried into the policy as written; a cap looser than the documented default is applied **and** flagged for confirmation; conflicting caps resolve to the strictest; impossible or "minimum per issuer" caps keep the default and are flagged. The same rules apply to the LLM parser's output (validated after the model).

**LLM-assisted Parse (decision D-J, implemented `8dd73df`, `c9ef464`; `src/thesis_llm.py`, `src/llm.py`):**
The model helps with language only; it proposes and never decides.
- OpenRouter chat/completions, `response_format` json_schema `strict`, `provider: {require_parameters: true, data_collection: "deny", zdr: true}`; pinned models `anthropic/claude-haiku-5.5` → `openai/gpt-5-mini` → `google/gemini-2.5-flash`. No `temperature` (current models reject it, and with `require_parameters` it excludes every endpoint).
- Schema has **no nullable unions** (some providers' strict mode refuses them): `numbers: [{field, pct, quote}]`, enums with `"unstated"`.
- **Jurisdiction and investor class come from onboarding only** — never from the text or the model.
- **Grounding:** each number needs a quote found verbatim in the thesis that contains it; otherwise dropped and flagged.
- **Merge with the rule-based parser toward the safer value:** lower cap, higher cash floor, more conservative risk / yield orientation, union of exclusions; duplicate readings → lowest cap / highest floor; conflicting currency readings ignored; every disagreement is flagged for the review screen.
- Thesis sent delimited as untrusted data; > 4000 characters or any error / budget refusal → rule-based parser (flagged).
- **SpendGuard:** before sending, the worst case (prompt + `max_tokens` at the dearest pinned model, live prices from `/models`) must fit the run budget and leave the account above a floor; the actual `usage.cost` is booked after (worst case if absent); truncated output is an error.

---

**Real yield numbers (audit A3, implemented):** `src/yields.py` reads rates from the chain, never assumes them. Blend supply APR = Blend v2's own interest model on the pool's live reserve data (verified against the real pool's accrual to <= 1 bp); vault realised APR = the vault's own daily conservative NAV history (`get_nav_history`, at most one point a day, 90 kept). Assets without a live source stay 0 = unknown and are never quoted. Rates are simple APRs, variable, never promised.

**LLM-assisted Explain (D-J):** the model writes a ≤ 120-word summary and 2-4 risks from a structured facts object only (allocation, vault currency, profile, floor, cap, warnings, items needing confirmation). It is discarded if it contains any number not in the facts or any promise ("guaranteed", "risk-free", "insured", ...). The deterministic `ExplanationReport` remains the authoritative explanation.

**Tests:** offline `tests/test_thesis_llm.py` (82 total in the suite; guard mutation 21/21); live `tests/test_llm_live.py`, opt-in (`CONSTELLA_LIVE_LLM=1`, key in env, shared $0.03 guard) — 3/3 on Claude Haiku 5.5, $0.00088 per run (17 Nov 2026).

---

### Step 2: Retrieve
**Input:** `PolicyDraft` object  
**Output:** Candidate asset list (from registry ONLY — no exceptions)

```python
# Audit A3 / decision D-B (implemented, `3eaaa02`)
registry = AssetRegistry.from_chain(rpc_url, passphrase, registry_id, any_account)  # registry.list_assets()
def retrieve_candidates(policy: PolicyDraft, registry: AssetRegistry) -> List[AssetCandidate]:
    # 1-2. Eligibility = issuer rules recorded ON CHAIN (blocked / eligible
    #      jurisdictions, required investor classes, active flag) AND
    #      Constella's class policy: retail = permissionless only (until
    #      Phase 4); sanctioned jurisdictions get nothing; vague answers fail closed.
    eligible = eligibility.eligible_assets(registry.list_assets(), policy.jurisdiction, policy.investor_class)
    # 3. Exclusions; 4. currency alignment (vector ranking: future)
    ...
```

**Closed universe enforcement (A3):** the on-chain registry is the only source of assets, contract addresses and issuer rules; the engine keeps display text only, skips on-chain assets it has no description for, and never invents yields (unknown until a live source exists). The vector search, when added, only searches over assets in the registry. Only registry contract addresses are valid outputs. If a user asks for "Amazon stock," and it's not in the registry, it cannot appear in the proposal — the diff screen will explain why.

---

### Step 3: Optimise
**Input:** Candidate assets + policy constraints  
**Output:** Proposed weights (target allocation per asset)

**Algorithm:** Hierarchical Risk Parity (HRP) as default for stability  
**Library:** PyPortfolioOpt (MIT license)

**Constraints enforced by optimizer:**
- Total weights sum to 1.0
- Per-issuer cap (from policy)
- Per-asset cap (from policy)
- Liquidity sleeve minimum (from policy)
- Currency exposure targets (from policy)
- No illiquid assets in liquid baskets
- Each sleeve within its declared liquidity tier

**Fail closed (audit A1 / decision D-A, implemented `fce2fed`):** there is **no** fallback that relaxes a rule. Hard rules — the profile (Cash Reserve, the default and only Phase-1 profile: vault currency ≥ 85 %, lending ≤ 15 %), the liquidity floor, and the user's issuer / single-asset caps on every asset **except the vault currency** — are never relaxed. The user's currency split is a soft target: if it conflicts, `InfeasibleAllocation` lists the reasons and offers the nearest valid allocation without it, which the user must accept. An exact LP proves feasibility, SLSQP / HRP refinements are kept only if still valid, bps rounding is cap-safe largest-remainder, and an integer-exact final check raises rather than returning an invalid allocation. The explainer discloses the vault-currency issuer exposure (e.g. "92 % depends on CIRCLE through USDC") plainly.

**Thesis-driven rules (step 4, decision D-P, 25 Nov 2026)** - these replace the single Cash Reserve profile for every thesis; the fail-closed machinery above is unchanged:
- vault currency = the cash asset of the thesis's main currency, restricted by the API to what the chain can price (the NAV oracle base, USDC; the factory refuses others);
- cash >= the thesis's liquidity floor (default 10 %); a currency the thesis did not ask for gets 0 %;
- each lending pool <= 15 % (one audited pool, D-A); excluded sleeves get 0 %;
- no tier-3 (illiquid) asset unless the horizon allows a month; assets slower than the horizon <= 25 % in total;
- the currency split is a two-sided ±5 % band and the refinement pulls each currency to its requested share;
- the parser reads fraction words ("a third", "half") and completes partial splits with the vault currency.

**Covariance inputs:**
- Historical NAV returns (where available; short histories for newer tokenized funds)
- Duration/liquidity tier proxies where history is insufficient

---

### Step 4: Simulate
**Input:** Proposed weights + historical NAV data + policy parameters  
**Output:** Simulation report shown to user **before any money moves**

| Component | Method |
|---|---|
| **Historical NAV replay** | Replay constituent NAVs over available history |
| **Yield estimate** | Weighted average of current yield disclosures per constituent |
| **Redemption ladder** | How much redeemable in <1h / 24h / 1 week / longer, by liquidity tier |
| **Depeg stress** | 5% depeg of the largest stable constituent; show basket impact |
| **Issuer outage** | Largest-weight issuer frozen for 30 days; show basket impact |
| **Worst-case exit time** | Slowest plausible full exit scenario |

**UI requirements (non-negotiable):**
> Every simulation screen MUST display the canonical disclaimer from `AGENT-RULES.md` Rule 10: *"This simulation uses historical data. Past performance does not predict future returns. This is not financial advice."*

- Label clearly: "Simulation" not "Backtest"
- Simulation is free and unlimited; no money moves until user explicitly initiates
- Show the worst case prominently, not just the base case

---

### Step 5: Explain
**Input:** Proposed basket + user's original thesis  
**Output:** Plain-language explanation + diff

- **Rationale:** Why each asset was included, in plain language
- **Diff against thesis:** Where the proposed basket matches and where it doesn't
  - e.g., *"You asked for 1/3 euro. We proposed 20% euro because the euro T-bill sleeve is permissioned and requires professional investor status. If you have that status, we can increase it."*
- **Ambiguity flags:** Any parsed fields with low confidence are flagged for user confirmation
- **Issuer concentration warning:** If any issuer exceeds 25% (even if below the user's configured cap), display a warning
- **Missing thesis elements:** If the thesis mentioned something not achievable, explain clearly why it's not in the proposal

---

### Step 6: Sign
**Input:** User approves the proposed basket  
**Output:** Signed policy hash on-chain; vault instantiated

```
1. Engine generates the final Policy object (not a draft — final)
2. Policy is serialized to canonical JSON (deterministic field ordering)
3. Policy hash = SHA-256(canonical JSON)
4. User (or multisig) — the policy *owner* — signs the policy hash together with every enforced term
   (Soroban authorization of `register_policy`, invoked by the vault through `vault.bind_policy`)
5. Policy contract on-chain stores the hash, the terms, and terms_hash = SHA-256(XDR(terms))
6. Vault is bound to this policy; only the owner can amend it later (re-sign `update_policy`), and an amendment takes effect only after a notice period of at least 48h so holders can exit first
7. The on-chain vault enforces only operations consistent with the signed policy
```

**Models can change. The signed policy cannot be exceeded.**

---

## Policy Object Schema (Final, On-Chain Canonical Form)

```json
{
  "schema_version": "1.0",
  "created_at": "2026-11-14T00:00:00Z",
  "basket_name": "cRUNWAY",
  "numeraire": "USDC",
  "sleeves": [
    {
      "asset_id": "FRANKLIN_BENJI",
      "contract_address": "CXXXXXX...",
      "target_weight_bps": 4500,
      "band_bps": 500,
      "sleeve_type": "usd_gov_fund"
    },
    {
      "asset_id": "SPIKO_EUR",
      "contract_address": "CXXXXXX...",
      "target_weight_bps": 2000,
      "band_bps": 500,
      "sleeve_type": "eur_sovereign"
    },
    {
      "asset_id": "USDC",
      "contract_address": "CXXXXXX...",
      "target_weight_bps": 1000,
      "band_bps": 200,
      "sleeve_type": "cash"
    }
  ],
  "constraints": {
    "max_issuer_weight_bps": 3000,
    "max_single_asset_weight_bps": 2500,
    "min_liquidity_sleeve_bps": 1000,
    "max_slippage_bps": 50,
    "rebalance_band_bps": 500
  },
  "eligibility": {
    "jurisdiction": "non-US",
    "investor_class": "professional",
    "share_class": "open"
  },
  "admin": {
    "admin_address": "GXXXX...",
    "multisig_threshold": 2,
    "multisig_signers": ["GXXXX...", "GYYY..."]
  }
}
```

---

## Asset Registry Schema (Vector Search Index)

Each asset in the registry has a text document for semantic search:

```
{
  asset_id: "FRANKLIN_BENJI",
  display_name: "Franklin Templeton OnChain U.S. Government Money Fund",
  description: "Tokenized US government money market fund; invests in US Treasury bills and repos; daily liquidity; targets SOFR-adjacent yield",
  tags: ["usd", "treasury", "government", "money-market", "fund", "yield", "daily-liquidity"],
  sleeve_type: "usd_gov_fund",
  liquidity_tier: 1,            // 24h
  current_yield_pct: 4.9,       // approximate, from issuer
  jurisdiction_note: "Available to non-US investors; KYC required",
  oracle_sources: ["BENJI_NAV_API", "REFLECTOR"]
}
```

---

## Technology Stack

| Component | Technology |
|---|---|
| LLM | OpenRouter (D-J): Claude Haiku 5.5 with gpt-5-mini / gemini-2.5-flash fallbacks, json_schema strict, ZDR + no data collection |
| Orchestration | Direct HTTP (stdlib), no LangChain — two calls do not need a framework |
| Vector search | pgvector (PostgreSQL extension) or Chroma |
| Portfolio optimisation | PyPortfolioOpt (HRP) |
| Policy schema | Pydantic (Python) |
| Backend | FastAPI or similar |
| NAV history store | PostgreSQL + TimescaleDB |

---

## Failure Modes and Safety

| Failure | Consequence | Safety |
|---|---|---|
| LLM misparses thesis | Poor asset selection | Closed universe → worst case is mediocre match; diff screen shows mismatch |
| LLM hallucinates an asset | Asset not in registry | Closed universe enforcement at Step 2 blocks it |
| LLM proposes weights that violate caps | Policy creation fails | Optimizer enforces constraints at Step 3; policy validation on-chain |
| Simulation is misleading | User has wrong expectations | Mandatory disclaimer; "simulation not forecast" in UI |
| User signs a bad policy | Vault executes a bad policy | Diff screen and explanation serve as final human review before signing |
| Model is replaced/changed | New model behaves differently | Signed policy is immutable; new model can propose policy *amendments* which user must re-sign |
