# Slice 5 Micro-Spec: Policy Storage & Rebalance Guardrails Contract (Soroban Rust)

> **Module:** `contracts/policy`  
> **Status:** Implemented (code + tests on main). Open findings tracked in `docs/audit/AUDIT-2026-11-15.md`.  
> **Reference:** [`docs/build-spec/contracts/CONTRACTS-SPEC.md`](../build-spec/contracts/CONTRACTS-SPEC.md#L71-L112), [`docs/build-spec/architecture/EXECUTION.md`](../build-spec/architecture/EXECUTION.md#L33-L85), [`docs/build-spec/AGENT-RULES.md`](../build-spec/AGENT-RULES.md) (Rule 1, 5, 6, 8, 11, 12)

---

## 1. Objectives of Slice 5

Implement and verify the on-chain Policy Storage & Validation contract in Soroban Rust:
1. **Rule 5 (Immutable Signed Policy Enforcement):** Store user-signed 32-byte policy hashes (`BytesN<32>`). No keeper, admin, or model can exceed signed bounds.
2. **Rebalance Guardrail Validation:** Provide an on-chain `validate_rebalance` endpoint that acts as the single execution gate for portfolio keepers.
3. **Multi-Constraint Verification:** Verify that any proposed trade sequence strictly satisfies:
   - Weights sum to 10,000 bps (100.00%)
   - Post-trade constituent weights reside within declared bands (`[min_band_bps, max_band_bps]`)
   - No single asset exceeds `max_single_asset_bps` (default 2500 bps)
   - No single issuer exceeds `max_issuer_bps` (default 3000 bps)
   - Cash liquidity sleeve meets or exceeds `min_liquidity_sleeve_bps` (default 1000 bps)
   - Slippage per trade operation does not exceed `max_slippage_bps` (default 50 bps)
   - Proposal timestamp is within execution deadline (`proposal.deadline >= current_time`)
4. **Registry Verification:** Cross-contract call to the `contracts/registry` contract to confirm all traded assets are active in the closed universe.
5. **Auditable Event Logging:** Emit `POLICY_REGISTERED`, `POLICY_UPDATED`, and `REBALANCE_VALIDATED` events.

---

## 2. Smart Contract Interface (Soroban Rust)

```rust
#[contractclient(name = "PolicyContractClient")]
pub trait PolicyContractTrait {
    /// Initialize with a governance admin and the (immutable) NAV oracle used
    /// to value trade legs for slippage (audit P1).
    fn initialize(env: Env, admin: Address, oracle: Address);

    /// Register a signed policy (requires vault + owner auth; one-time).
    /// Updated 15 Nov 2026 for audit P3 — see CONTRACTS-SPEC.md "Signing model".
    fn register_policy(
        env: Env,
        vault: Address,
        owner: Address,
        policy_hash: BytesN<32>,
        registry_address: Address,
        min_liquidity_sleeve_bps: u32,
        max_single_asset_bps: u32,
        max_issuer_bps: u32,
        max_slippage_bps: u32,
        rebalance_band_bps: u32,
        sleeves: Vec<SleeveTarget>,
    );

    /// Propose an amendment (owner signature). Takes effect only via
    /// activate_policy_update after the notice period (>= 48h, audit P4).
    fn update_policy(
        env: Env,
        vault: Address,
        new_hash: BytesN<32>,
        min_liquidity_sleeve_bps: u32,
        max_single_asset_bps: u32,
        max_issuer_bps: u32,
        max_slippage_bps: u32,
        rebalance_band_bps: u32,
        new_sleeves: Vec<SleeveTarget>,
    );

    /// Get registered policy hash for a vault
    fn get_policy_hash(env: Env, vault: Address) -> BytesN<32>;

    /// Get policy configuration parameters
    fn get_policy_config(env: Env, vault: Address) -> Option<PolicyConfig>;

    /// Get sleeve targets and bands for a vault
    fn get_sleeve_targets(env: Env, vault: Address) -> Vec<SleeveTarget>;

    /// Validate a proposed rebalance. Returns ValidationResult(is_valid, code, message).
    fn validate_rebalance(
        env: Env,
        vault: Address,
        proposal: RebalanceProposal,
    ) -> ValidationResult;

    /// Get admin address
    fn get_admin(env: Env) -> Address;
}
```

### Core Data Structs
```rust
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SleeveTarget {
    pub asset_id: Symbol,
    pub target_weight_bps: u32,
    pub min_band_bps: u32,
    pub max_band_bps: u32,
    pub sleeve_type: Symbol, // e.g. "cash", "usd_gov_fund", "eur_sovereign"
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicyConfig {
    pub vault: Address,
    pub policy_hash: BytesN<32>,
    pub version: u32,
    pub registry_address: Address,
    pub min_liquidity_sleeve_bps: u32,
    pub max_single_asset_bps: u32,
    pub max_issuer_bps: u32,
    pub max_slippage_bps: u32,
    pub rebalance_band_bps: u32,
    pub is_active: bool,
    pub admin: Address,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TradeRoute {
    Issuer = 0,
    Soroswap = 1,
    Aquarius = 2,
    Sdex = 3,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TradeOperation {
    pub asset_in: Address,
    pub asset_out: Address,
    pub amount_in: i128,
    pub min_amount_out: i128, // Enforces maximum slippage
    pub route: TradeRoute,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RebalanceProposal {
    pub vault: Address,
    pub operations: Vec<TradeOperation>,
    pub weights_after: Vec<(Symbol, u32)>, // (asset_id, weight_bps)
    pub deadline: u64,
    pub proposer: Address,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidationResult {
    pub is_valid: bool,
    pub code: u32, // 0 = Valid, 1..=12 = Specific failure reason
    pub message: Symbol,
}
```

---

## 3. Mandatory Test Matrix (TDD Verification)

| Test ID | Test Scenario | Expected Outcome |
|---|---|---|
| **T26** | Policy Registration & Hash Storage | Admin registers policy for vault with SHA-256 hash; `get_policy_hash` returns exact 32-byte hash; unauthorized caller fails. |
| **T27** | Valid Rebalance Acceptance | Compliant rebalance proposal within bands, preserving liquidity floor and honoring caps returns `is_valid == true`, `code == 0`. |
| **T28** | Weight Band Breach Rejection | Proposal proposing weights exceeding sleeve min/max bands returns `is_valid == false`, code indicates band breach. |
| **T29** | Liquidity Floor Breach Rejection | Proposal that reduces cash sleeve below `min_liquidity_sleeve_bps` (1000 bps) is rejected. |
| **T30** | Single Asset & Issuer Cap Rejections | Proposal placing >2500 bps in single asset or >3000 bps in single issuer is rejected. |
| **T31** | Slippage & Deadline Rejections | Expired deadline or excessive trade slippage (`min_amount_out` deviation > `max_slippage_bps`) is rejected. |
| **T32** | Closed Universe Registry Cross-Check | Proposal trading an unregistered/deactivated asset is rejected. |

---

## 4. Acceptance Criteria to Gate Slice 5

- [x] Cargo workspace configured with `policy` member.
- [x] Contract authored with Soroban SDK and compiles cleanly.
- [x] `cargo test` in `contracts/policy` passes 100% of T26–T32 test cases.
- [x] High-frequency atomic commits and pushes to GitHub.
- [x] AI pauses and presents test assertion proofs before touching Slice 6.
