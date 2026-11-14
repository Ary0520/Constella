# Slice 4 Micro-Spec: Closed Universe Asset Registry Contract (Soroban Rust)

> **Module:** `contracts/registry`  
> **Status:** Specification & Plan Phase (Pre-Implementation)  
> **Reference:** [`docs/build-spec/contracts/CONTRACTS-SPEC.md`](../build-spec/contracts/CONTRACTS-SPEC.md#L32-L70), [`docs/build-spec/ai-engine/REGISTRY.md`](../build-spec/ai-engine/REGISTRY.md), [`docs/build-spec/AGENT-RULES.md`](../build-spec/AGENT-RULES.md) (Rule 1, 4, 6, 8, 11, 12)

---

## 1. Objectives of Slice 4

Implement and verify the on-chain Closed Universe Asset Registry contract in Soroban Rust:
1. **On-Chain Closed Universe Enforcement:** Ensure no asset can ever be held, rebalanced, or priced by Constella vaults unless it has been explicitly approved and registered by protocol governance.
2. **Access Controlled Governance:** Only the authorized protocol admin/multisig can register or deactivate assets.
3. **Dual Lookups:** Support lookups by both symbolic identifier (`Symbol`, e.g. `USDC`, `FRANKLIN_BENJI`) and on-chain contract address (`Address`).
4. **Jurisdiction & Classification Metadata:** Store regulatory eligibility, jurisdiction blocks, and oracle source keys directly on-chain.
5. **Auditable Event Logging:** Emit standardized on-chain events (`ASSET_ADDED`, `ASSET_DEACTIVATED`) on state changes.

---

## 2. Smart Contract Interface (Soroban Rust)

```rust
#[contractclient(name = "AssetRegistryClient")]
pub trait AssetRegistryTrait {
    /// Initialize the registry with an authorized admin address (multisig)
    fn initialize(env: Env, admin: Address);

    /// Add an approved asset to the closed universe (admin only)
    fn add_asset(env: Env, entry: AssetEntry);

    /// Deactivate an asset, disabling fresh allocations (admin only)
    fn deactivate_asset(env: Env, asset_id: Symbol);

    /// Query an asset entry by its symbolic identifier
    fn get_asset(env: Env, asset_id: Symbol) -> Option<AssetEntry>;

    /// Query an asset entry by its Soroban contract address
    fn get_asset_by_address(env: Env, contract_address: Address) -> Option<AssetEntry>;

    /// Check if a contract address is currently an active registered asset
    fn is_registered(env: Env, contract_address: Address) -> bool;

    /// Return all registered asset entries
    fn list_assets(env: Env) -> Vec<AssetEntry>;

    /// Return the current registry admin address
    fn get_admin(env: Env) -> Address;
}
```

### Core Data Structs
```rust
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SleeveType {
    Cash = 0,
    UsdGovFund = 1,
    EurSovereign = 2,
    YieldUsd = 3,
    Lending = 4,
    Equity = 5,
    Xlm = 6,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssetEntry {
    pub contract_address: Address,
    pub asset_id: Symbol,
    pub sleeve_type: SleeveType,
    pub liquidity_tier: u32, // 0=instant, 1=24h, 2=1week, 3=illiquid
    pub permissionless: bool,
    pub oracle_sources: Vec<Symbol>,
    pub eligible_jurisdictions: Vec<Symbol>, // empty = all
    pub blocked_jurisdictions: Vec<Symbol>,
    pub investor_classes_required: Vec<Symbol>, // empty = all
    pub is_active: bool,
    pub added_at: u64,
}
```

---

## 3. Mandatory Test Matrix (TDD Verification)

| Test ID | Test Scenario | Expected Outcome |
|---|---|---|
| **T21** | Initialization & Admin Auth | Registry initializes with admin; non-admin caller attempting `add_asset` fails with authentication error. |
| **T22** | Register Permissionless Assets (Phase 1) | Admin adds `USDC`, `EURC`, `BLEND_USDC_SUPPLY`; each is verified retrievable by ID and Address with `is_registered == true`. |
| **T23** | Asset Deactivation | Admin deactivates `USDC`; `get_asset` reflects `is_active == false`; `is_registered` returns `false`. |
| **T24** | Duplicate & Unregistered Queries | Duplicate addition updates/overwrites or errors cleanly; unregistered address queries return `None` and `is_registered == false`. |
| **T25** | Event Emission Verification | `ASSET_ADDED` and `ASSET_DEACTIVATED` topics and payloads match spec. |

---

## 4. Acceptance Criteria to Gate Slice 4

- [x] Cargo workspace configured with `registry` member.
- [x] Contract authored with Soroban SDK and compiles cleanly.
- [x] `cargo test` in `contracts/registry` passes 100% of T21–T25 test cases.
- [x] Atomic commits and pushes to GitHub.
- [x] AI pauses and presents test assertion proofs before touching Slice 5.
