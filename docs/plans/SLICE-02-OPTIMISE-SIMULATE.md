# Slice 2 Micro-Spec: Thesis Engine Optimise & Simulate (Steps 3 & 4)

> **Module:** `services/ai-engine`  
> **Status:** Specification & Plan Phase (Pre-Implementation)  
> **Reference:** [`docs/build-spec/ai-engine/THESIS-ENGINE.md`](../build-spec/ai-engine/THESIS-ENGINE.md), [`docs/build-spec/architecture/VAULT-DESIGN.md`](../build-spec/architecture/VAULT-DESIGN.md), [`docs/build-spec/AGENT-RULES.md`](../build-spec/AGENT-RULES.md) (Rule 1, 3, 10, 11)

---

## 1. Objectives of Slice 2

Implement and verify Steps 3 and 4 of the Constella Thesis Engine:
1. **Step 3: Optimise (`optimizer.py`)**  
   Generate an optimal target allocation vector across candidate assets using Hierarchical Risk Parity (HRP via PyPortfolioOpt) while enforcing all signed policy bounds:
   - Weights sum to 1.0 (10,000 bps)
   - Liquidity floor: Cash sleeve (`USDC`/`EURC`) >= `policy.min_liquidity_sleeve`
   - Currency exposure: Matches target USD/EUR ratio
   - Concentration caps: No single issuer > `max_issuer_weight` (default 30%); no single asset > `max_single_asset_weight` (default 25%)
   - Fallback heuristic when historical covariance returns are absent or singular.

2. **Step 4: Simulate (`simulator.py`)**  
   Produce a multi-dimensional risk, yield, redemption, and stress simulation report before user funds are committed:
   - Weighted average yield estimation
   - 4-tier redemption ladder (< 1h, 24h, 1 week, illiquid)
   - 5% stablecoin depeg stress test
   - 30-day issuer freeze stress test
   - Mandatory legal disclaimer (Rule 10)

---

## 2. Input / Output Contracts

### Input to Step 3
- `policy: PolicyDraft` (from Step 1)
- `candidates: list[AssetCandidate]` (from Step 2)
- `historical_returns: pd.DataFrame | None = None` (optional historical return series)

### Output of Step 3 (`OptimizationResult`)
```python
class OptimizationResult(BaseModel):
    weights: dict[str, float]  # asset_id -> allocation [0.0, 1.0], sum == 1.0
    weights_bps: dict[str, int]  # asset_id -> basis points [0, 10000], sum == 10000
    sleeve_weights: dict[str, float]  # sleeve_type -> total weight
    currency_weights: dict[str, float]  # currency -> total weight
    issuer_weights: dict[str, float]  # issuer -> total weight
    algorithm_used: str  # "HRP" | "CONSTRAINED_HEURISTIC"
```

### Output of Step 4 (`SimulationReport`)
```python
class RedemptionLadder(BaseModel):
    instant_pct: float  # < 1 hour (tier 0)
    within_24h_pct: float  # 24 hours (tier 1)
    within_1w_pct: float  # 1 week (tier 2)
    illiquid_pct: float  # tier 3

class StressScenario(BaseModel):
    scenario_name: str
    nav_impact_pct: float  # e.g. -0.005 (-0.5%)
    trapped_liquidity_pct: float  # e.g. 0.25 (25%)
    description: str

class SimulationReport(BaseModel):
    weighted_yield_pct: float
    redemption_ladder: RedemptionLadder
    depeg_stress: StressScenario
    issuer_freeze_stress: StressScenario
    worst_case_exit_hours: int
    historical_nav_series: list[float]
    disclaimer: str
```

---

## 3. Mandatory Test Matrix (TDD Verification)

| Test ID | Test Scenario | Expected Outcome |
|---|---|---|
| **T05** | HRP / Constrained Weights Sum & Cap Enforcement | Weights sum strictly to 1.0 (10,000 bps). No issuer exceeds 30%; no asset exceeds 25%; cash sleeve meets >= 10%. |
| **T06** | Currency Matching Constraint | Target currency ratio (e.g. 70% USD / 20% EUR) is preserved within ±5% tolerance. |
| **T07** | Redemption Ladder Horizon Breakdown | Basket with 10% USDC, 45% BENJI, 20% Spiko EUR, 25% Spiko USD partitions correctly: instant=10%, 24h=90%, 1w=0%. |
| **T08** | Depeg & Issuer Freeze Stress Simulations | 5% depeg on 10% USDC reflects -0.5% NAV impact. 30-day freeze on Spiko (45% total across USD/EUR) captures 45% trapped liquidity without crashing remaining sleeves. |
| **T09** | Mandatory Simulation Disclaimer (Rule 10) | Exact legal disclaimer text is present in the report. |

---

## 4. Acceptance Criteria to Gate Slice 2

- [x] `test_optimise_and_simulate.py` written and verified failing before implementation.
- [x] `optimizer.py` and `simulator.py` pass all T05–T09 test assertions.
- [x] PyPortfolioOpt HRP integrated with fallback handling.
- [x] 100% test pass rate with clean outputs.
- [x] AI pauses and presents verification proofs before touching Slice 3.
