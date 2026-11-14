# Slice 1 Micro-Spec: Thesis Engine Parse & Retrieve (Steps 1 & 2)

> **Module:** `services/ai-engine`  
> **Status:** Specification & Test Harness Phase (Pre-Implementation)  
> **Reference:** [`docs/build-spec/ai-engine/THESIS-ENGINE.md`](../build-spec/ai-engine/THESIS-ENGINE.md), [`docs/build-spec/ai-engine/REGISTRY.md`](../build-spec/ai-engine/REGISTRY.md)

---

## 1. Objectives of Slice 1

Implement and verify the first two foundational steps of the Constella Thesis Engine:
1. **Step 1: Parse**  
   Convert an arbitrary plain-English investment thesis into a typed, validated `PolicyDraft` schema with explicit confidence metrics and ambiguity warnings.
2. **Step 2: Retrieve (Closed Universe Enforcement)**  
   Filter candidate assets exclusively from the verified Stellar Asset Registry based on user jurisdiction, investor accreditation class, and declared exclusions. **Zero hallucinated tickers permitted.**

---

## 2. Input / Output Contracts

### Input
- `thesis_text: str` (e.g., *"Park 18 months of runway safe, 70% dollar T-bill funds, 20% euro T-bills for payroll, 10% instant liquidity."*)
- `user_jurisdiction: str` (e.g., `"US"`, `"EU"`, `"IN"`, `"non-US"`)
- `investor_class: str` (`"retail"`, `"professional"`, `"institutional"`)

### Output of Step 1 (`PolicyDraft`)
```python
class PolicyDraft(BaseModel):
    objectives: list[str]
    currency_exposure: dict[str, float]  # e.g. {"USD": 0.80, "EUR": 0.20}
    liquidity_horizon_hours: int
    max_issuer_weight: float
    max_single_asset_weight: float
    risk_tolerance: str                  # "conservative" | "moderate" | "growth"
    exclusions: list[str]
    min_liquidity_sleeve: float          # default 0.05 to 0.15
    target_yield_orientation: str
    investor_class: str
    jurisdiction: str
    confidence: dict[str, float]
    ambiguities: list[str]
```

### Output of Step 2 (`list[AssetCandidate]`)
A list of approved asset entries strictly originating from the diligence registry matching the user's constraints.

---

## 3. Mandatory Test Matrix (TDD Verification)

| Test ID | Test Scenario | Expected Outcome |
|---|---|---|
| **T01** | Standard treasury runway thesis | Successfully parses into USD 70%, EUR 20%, cash 10%; returns valid candidates (USDC, EURC, Spiko/BENJI). |
| **T02** | User requests non-existent/hallucinated asset (e.g. "Put 50% in TSLA stock") | Closed Universe Rule blocks the asset; TSLA never appears in candidate list; ambiguity/mismatch flagged. |
| **T03** | Retail investor in US attempts to access gated professional asset (e.g. JQQQ or Tradable) | Eligibility filter strips unauthorized assets; candidate list contains only permissionless/eligible assets. |
| **T04** | Ambiguous or incomplete thesis (e.g. "I want yield") | Generates conservative fallback defaults; flags low confidence on missing parameters in `ambiguities`. |

---

## 4. Acceptance Criteria to Gate Slice 1

Before moving to Step 3 (Optimise) or Step 4 (Simulate):
- [ ] Test harness passes 100% of T01–T04 test cases.
- [ ] Schema validation rejects out-of-bound weights or invalid enum values.
- [ ] Zero network/external dependencies required for offline deterministic testing.
- [ ] AI pauses and displays complete test run evidence to the user.
