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

**LLM instructions:**
- Use schema-constrained output (function calling mode)
- Do NOT hallucinate asset names; leave `exclusions` for domain-level exclusions only
- If the thesis is ambiguous, generate the most conservative reasonable interpretation AND flag ambiguities for user confirmation
- Output confidence score for each parsed field

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

**Fallback:** If HRP fails to converge → equal-weight within constraints.

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
| LLM | GPT-4o or Claude via API (schema-constrained function calling) |
| Orchestration | LangChain |
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
