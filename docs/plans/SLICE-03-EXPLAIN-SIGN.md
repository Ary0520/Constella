# Slice 3 Micro-Spec: Thesis Engine Explain & Sign (Steps 5 & 6)

> **Module:** `services/ai-engine`  
> **Status:** Specification & Plan Phase (Pre-Implementation)  
> **Reference:** [`docs/build-spec/ai-engine/THESIS-ENGINE.md`](../build-spec/ai-engine/THESIS-ENGINE.md), [`docs/build-spec/AGENT-RULES.md`](../build-spec/AGENT-RULES.md) (Rule 1, 5, 11), [`docs/build-spec/contracts/CONTRACTS-SPEC.md`](../build-spec/contracts/CONTRACTS-SPEC.md)

---

## 1. Objectives of Slice 3

Implement and verify Steps 5 and 6 of the Constella Thesis Engine, completing the 6-stage AI pipeline:
1. **Step 5: Explain (`explainer.py`)**  
   Generate an actionable plain-language rationale, a transparent diff against the user's original thesis, ambiguity highlights, and concentration warnings.
2. **Step 6: Sign (`signer.py`)**  
   Assemble the canonical on-chain `CanonicalPolicy` object, perform deterministic RFC 8785 canonical JSON serialization, and generate the 32-byte SHA-256 policy hash (`BytesN<32>`) that anchors on-chain execution.

---

## 2. Input / Output Contracts

### Input to Step 5
- `thesis_text: str`
- `policy: PolicyDraft`
- `opt_result: OptimizationResult`
- `candidates: list[AssetCandidate]`

### Output of Step 5 (`ExplanationReport`)
```python
class DiffItem(BaseModel):
    category: str  # e.g., "currency", "exclusion", "liquidity"
    requested: str
    proposed: str
    explanation: str

class ExplanationReport(BaseModel):
    asset_rationales: dict[str, str]  # asset_id -> plain-language rationale
    thesis_diff: list[DiffItem]  # list of diff items
    issuer_concentration_warnings: list[str]  # warnings if any issuer >= 25%
    ambiguity_clarifications: list[str]
```

### Input to Step 6
- `policy: PolicyDraft`
- `opt_result: OptimizationResult`
- `candidates: list[AssetCandidate]`
- `basket_name: str` (e.g., `"cRUNWAY"`)
- `admin_address: str`
- `multisig_signers: list[str]`
- `multisig_threshold: int`

### Output of Step 6 (`SignedPolicyEnvelope`)
```python
class CanonicalSleeve(BaseModel):
    asset_id: str
    contract_address: str
    target_weight_bps: int
    band_bps: int
    sleeve_type: str

class CanonicalConstraints(BaseModel):
    max_issuer_weight_bps: int
    max_single_asset_weight_bps: int
    min_liquidity_sleeve_bps: int
    max_slippage_bps: int
    rebalance_band_bps: int

class CanonicalEligibility(BaseModel):
    jurisdiction: str
    investor_class: str
    share_class: str

class CanonicalAdmin(BaseModel):
    admin_address: str
    multisig_threshold: int
    multisig_signers: list[str]

class CanonicalPolicy(BaseModel):
    schema_version: str = "1.0"
    created_at: str
    basket_name: str
    numeraire: str
    sleeves: list[CanonicalSleeve]
    constraints: CanonicalConstraints
    eligibility: CanonicalEligibility
    admin: CanonicalAdmin

class SignedPolicyEnvelope(BaseModel):
    policy: CanonicalPolicy
    canonical_json: str
    policy_hash_hex: str
    policy_hash_bytes: bytes
```

---

## 3. Mandatory Test Matrix (TDD Verification)

| Test ID | Test Scenario | Expected Outcome |
|---|---|---|
| **T14** | Asset rationales & diff generation | Every constituent receives a plain-language explanation. Mismatches (e.g. 70% USD vs 72% proposed, excluded assets) generate clear `DiffItem`s. |
| **T15** | Issuer concentration warning trigger | Any issuer with allocation >= 25% triggers an explicit concentration risk warning even if below the 30% cap. |
| **T16** | Canonical JSON Determinism | Canonical JSON is strictly sorted by key with compact separators (`","`, `":"`); two identically valued policies always yield the exact same SHA-256 hash. |
| **T17** | SHA-256 Policy Hash Format | `policy_hash_bytes` is exactly 32 bytes (`len == 32`), and `policy_hash_hex` is a valid 64-character hex string compatible with Soroban `BytesN<32>`. |
| **T18** | End-to-End Pipeline Sanity (Steps 1–6) | Natural language text runs through Parse → Retrieve → Optimise → Simulate → Explain → Sign in a single coherent flow. |

---

## 4. Acceptance Criteria to Gate Slice 3

- [ ] `test_explain_and_sign.py` written and verified failing before implementation.
- [ ] `explainer.py` and `signer.py` authored and pass T14–T18.
- [ ] Deterministic canonical JSON and SHA-256 policy hashing verified.
- [ ] Complete pipeline test passing end-to-end (all 25+ tests green).
- [ ] High-frequency atomic git commits and pushes executed.
- [ ] AI pauses and presents verification proofs before proceeding to Slice 4.
