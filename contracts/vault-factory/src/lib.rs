//! Constella Private Vault Factory (decision D-C).
//!
//! Treasuries create their own private vault - the same audited
//! `vault-strategy` code as the shared vault - without Constella deploying it
//! for them. The split of control is the point:
//!
//! * **Fixed by the factory** (a creator cannot choose them): the vault WASM,
//!   the shared registry, NAV oracle and policy contracts, and the fee
//!   treasury. A vault can never be pointed at a fake price feed or registry.
//! * **Constella's fee (decision D-O)**: the performance fee paid to the
//!   Constella treasury is the factory's `default_fee_bps` (<= 10 % of profit,
//!   D-D). A creator alone can only take the default; any other rate (a
//!   negotiated plan discount) needs the factory admin's co-signature, as in
//!   Yearn v3's factory-held protocol fee config with governance per-vault
//!   overrides. Changing the default affects future vaults only; a vault's
//!   fee is fixed for its life (the vault has no fee setter).
//! * **Chosen by the creator**, within protocol bounds: the vault admin and
//!   pause authority (their own multisig / smart account), the numéraire (a
//!   registered cash asset - the vault constructor enforces it) and the caps
//!   (<= the factory's `max_tvl_cap`, AGENT-RULES Rule 6).
//!
//! Addresses derive from SHA-256(creator || salt), so nobody can front-run or
//! squat another creator's vault address. `is_vault` attests that a contract
//! was deployed by this factory from the approved code (like Blend's
//! `is_pool`). Note: a vault's own admin can still upgrade it after the
//! vault's 48 h public timelock; `is_vault` attests origin, not current code.
//!
//! Changing the WASM used for FUTURE vaults waits 48 h (watchers can react);
//! existing vaults are never touched.
#![no_std]
// The constructor takes exactly the factory's fixed settings (atomic with
// deployment, audit M3); #[contractimpl] also generates wrappers outside the
// impl block, so the allowance must be crate-level (as in nav-oracle).
#![allow(clippy::too_many_arguments)]

use soroban_sdk::{
    contract, contractclient, contracterror, contractevent, contractimpl, contracttype, ContractExecutable,
    panic_with_error, xdr::ToXdr, Address, Bytes, BytesN, Env, FromVal, IntoVal, String, Symbol, SymbolStr, Val, Vec,
};

/// Same 48 h as the vault upgrade timelock, policy notice, recovery delay.
pub const WASM_CHANGE_DELAY_SECONDS: u64 = 48 * 3600;
/// Mirrors vault-strategy MAX_PERFORMANCE_FEE_BPS (D-D): 10 % of profit.
pub const MAX_PERFORMANCE_FEE_BPS: u32 = 1_000;
/// Bound on the per-creator index (persistent, one entry per creator).
pub const MAX_VAULTS_PER_CREATOR: u32 = 100;
/// Vault share decimals (all Constella vaults: 7, like Stellar assets).
pub const VAULT_DECIMALS: u32 = 7;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum FactoryError {
    /// Fee above 10 % of profit, non-positive caps, or per-user > TVL cap.
    InvalidParams = 1,
    /// tvl_cap above the factory's protocol maximum (Rule 6).
    TvlCapTooHigh = 2,
    /// No WASM change is pending.
    NoPendingWasm = 3,
    /// The 48 h WASM-change delay has not elapsed.
    WasmChangeNotReady = 4,
    /// This creator reached MAX_VAULTS_PER_CREATOR.
    TooManyVaults = 5,
    /// Step 3: a policy sleeve is not an active registry asset.
    AssetNotRegistered = 6,
}

/// Step 3: one signed policy sleeve (mirror of `constella_policy::SleeveTarget`
/// / the vault's `PolicySleeveTargetRef`; contract types decode by field name).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicySleeveTarget {
    pub asset_id: Symbol,
    pub target_weight_bps: u32,
    pub min_band_bps: u32,
    pub max_band_bps: u32,
    pub sleeve_type: Symbol,
}

/// Step 3: the signed policy a creator binds to their new vault - exactly the
/// `bind_policy` terms the AI engine derives from the hashed policy.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicyTerms {
    pub policy_hash: BytesN<32>,
    pub min_liquidity_sleeve_bps: u32,
    pub max_single_asset_bps: u32,
    pub max_issuer_bps: u32,
    pub max_slippage_bps: u32,
    pub rebalance_band_bps: u32,
    pub sleeves: Vec<PolicySleeveTarget>,
}

/// Mirror of `constella_vault_strategy::SleeveState` (enum fields as u32).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SleeveStateRef {
    pub asset_id: String,
    pub contract_address: Address,
    pub target_weight_bps: u32,
    pub band_bps: u32,
    pub current_balance: i128,
    pub sleeve_type: u32,
    pub liquidity_tier: u32,
}

/// Mirror of `constella_registry::AssetEntry` (sleeve type as u32; the
/// registry and vault sleeve-type numbering is identical: 0 cash ... 6 XLM).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegistryAssetRef {
    pub contract_address: Address,
    pub asset_id: Symbol,
    pub issuer: Symbol,
    pub sleeve_type: u32,
    pub liquidity_tier: u32,
    pub permissionless: bool,
    pub oracle_sources: Vec<Symbol>,
    pub eligible_jurisdictions: Vec<Symbol>,
    pub blocked_jurisdictions: Vec<Symbol>,
    pub investor_classes_required: Vec<Symbol>,
    pub is_active: bool,
    pub added_at: u64,
}

#[contractclient(name = "RegistryClient")]
pub trait RegistryInterface {
    fn get_asset(env: Env, asset_id: Symbol) -> Option<RegistryAssetRef>;
}

/// The vault admin calls the factory makes on the creator's behalf (the
/// creator's single signature covers them as sub-invocations).
#[contractclient(name = "VaultClient")]
pub trait VaultAdminInterface {
    fn bind_policy(env: Env, owner: Address, policy_hash: BytesN<32>, min_liquidity_sleeve_bps: u32, max_single_asset_bps: u32,
                   max_issuer_bps: u32, max_slippage_bps: u32, rebalance_band_bps: u32, sleeves: Vec<PolicySleeveTarget>);
    fn set_sleeves(env: Env, sleeves: Vec<SleeveStateRef>);
    fn set_keeper(env: Env, keeper: Address);
    fn propose_router(env: Env, route: u32, router: Address);
}

/// `constella_vault_strategy::TradeRoute::Soroswap`.
const ROUTE_SOROSWAP: u32 = 1;

/// Mirror of `constella_vault_strategy::VaultConfig` (field names and types
/// must match: contract types decode by field name).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VaultConfigRef {
    pub admin: Address,
    pub pause_authority: Address,
    pub numeraire: Address,
    pub policy_contract: Address,
    pub oracle_contract: Address,
    pub registry_contract: Address,
    pub treasury: Address,
    pub performance_fee_bps: u32,
    pub tvl_cap: i128,
    pub per_user_cap: i128,
}

/// What a creator chooses.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VaultParams {
    pub admin: Address,
    pub pause_authority: Address,
    pub numeraire: Address,
    pub performance_fee_bps: u32,
    pub tvl_cap: i128,
    pub per_user_cap: i128,
    pub name: String,
    pub symbol: String,
    pub salt: BytesN<32>,
}

/// What the factory fixes.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FactorySettings {
    pub admin: Address,
    pub vault_wasm_hash: BytesN<32>,
    pub registry: Address,
    pub oracle: Address,
    pub policy: Address,
    pub treasury: Address,
    pub max_tvl_cap: i128,
    /// Decision D-O: Constella's fee for factory vaults (bps of profit).
    pub default_fee_bps: u32,
    /// Step 3: Constella's keeper, set on vaults created with a policy.
    pub keeper: Address,
    /// Step 3: Soroswap router set on vaults created with a policy (if any).
    pub router: Option<Address>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingWasm {
    pub wasm_hash: BytesN<32>,
    pub effective_at: u64,
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Settings,
    PendingWasm,
    /// vault -> creator
    Vault(Address),
    /// creator -> vaults
    VaultsOf(Address),
    /// Step 3: number of vaults ever created (instance; append-only index).
    VaultCount,
    /// Step 3: index -> vault (persistent, renewed on write).
    VaultAt(u32),
}

#[contractevent(topics = ["VAULT_CREATED"], data_format = "vec")]
pub struct VaultCreated {
    #[topic]
    pub creator: Address,
    pub vault: Address,
    pub wasm_hash: BytesN<32>,
}

#[contractevent(topics = ["VAULT_WASM_PROPOSED"], data_format = "vec")]
pub struct VaultWasmProposed {
    pub wasm_hash: BytesN<32>,
    pub effective_at: u64,
}

/// Topics [DEFAULT_FEE_SET]; data (previous_bps, new_bps). Future vaults only.
#[contractevent(topics = ["DEFAULT_FEE_SET"], data_format = "vec")]
pub struct DefaultFeeSet {
    pub previous_bps: u32,
    pub new_bps: u32,
}

#[contractevent(topics = ["VAULT_WASM_ACTIVATED"], data_format = "single-value")]
pub struct VaultWasmActivated {
    pub wasm_hash: BytesN<32>,
}

const DAY_LEDGERS: u32 = 17_280;
const TTL_THRESHOLD: u32 = 29 * DAY_LEDGERS;
const TTL_EXTEND_TO: u32 = 30 * DAY_LEDGERS;

fn settings(env: &Env) -> FactorySettings {
    env.storage().instance().get(&DataKey::Settings).unwrap()
}

fn extend(env: &Env) {
    env.storage().instance().extend_ttl(TTL_THRESHOLD, TTL_EXTEND_TO);
}

/// Shared by both creation paths: protocol bounds, the D-O fee rule, the
/// creator-bound address, the per-creator and global indexes, the event.
fn deploy_vault(env: &Env, creator: &Address, params: VaultParams) -> Address {
    let env = env.clone();
    let creator = creator.clone();
    let s = settings(&env);
    if params.performance_fee_bps > MAX_PERFORMANCE_FEE_BPS
        || params.tvl_cap <= 0
        || params.per_user_cap <= 0
        || params.per_user_cap > params.tvl_cap
    {
        panic_with_error!(&env, FactoryError::InvalidParams);
    }
    // D-O: Constella's fee is Constella's to set. Anything but the default
    // must be co-signed by the factory admin.
    if params.performance_fee_bps != s.default_fee_bps {
        s.admin.require_auth();
    }
    if params.tvl_cap > s.max_tvl_cap {
        panic_with_error!(&env, FactoryError::TvlCapTooHigh);
    }
    let mut mine: Vec<Address> = env.storage().persistent().get(&DataKey::VaultsOf(creator.clone())).unwrap_or_else(|| Vec::new(&env));
    if mine.len() >= MAX_VAULTS_PER_CREATOR {
        panic_with_error!(&env, FactoryError::TooManyVaults);
    }
    // Address bound to the creator: SHA-256(creator XDR || salt).
    let mut seed = Bytes::new(&env);
    seed.append(&creator.clone().to_xdr(&env));
    seed.append(&params.salt.clone().into());
    let salt: BytesN<32> = env.crypto().sha256(&seed).into();
    let config = VaultConfigRef {
        admin: params.admin,
        pause_authority: params.pause_authority,
        numeraire: params.numeraire,
        policy_contract: s.policy.clone(),
        oracle_contract: s.oracle.clone(),
        registry_contract: s.registry.clone(),
        treasury: s.treasury.clone(),
        performance_fee_bps: params.performance_fee_bps,
        tvl_cap: params.tvl_cap,
        per_user_cap: params.per_user_cap,
    };
    let args: Vec<Val> = (config, params.name, params.symbol, VAULT_DECIMALS).into_val(&env);
    let vault = env.deployer().with_current_contract(salt)
        .deploy_contract(ContractExecutable::Wasm(s.vault_wasm_hash.clone()), args);

    env.storage().persistent().set(&DataKey::Vault(vault.clone()), &creator);
    env.storage().persistent().extend_ttl(&DataKey::Vault(vault.clone()), TTL_THRESHOLD, TTL_EXTEND_TO);
    mine.push_back(vault.clone());
    env.storage().persistent().set(&DataKey::VaultsOf(creator.clone()), &mine);
    // Step 3: global append-only index for services (keeper, relayer).
    let n: u32 = env.storage().instance().get(&DataKey::VaultCount).unwrap_or(0);
    env.storage().persistent().set(&DataKey::VaultAt(n), &vault);
    env.storage().persistent().extend_ttl(&DataKey::VaultAt(n), TTL_THRESHOLD, TTL_EXTEND_TO);
    env.storage().instance().set(&DataKey::VaultCount, &(n + 1));
    env.storage().persistent().extend_ttl(&DataKey::VaultsOf(creator.clone()), TTL_THRESHOLD, TTL_EXTEND_TO);
    extend(&env);
    VaultCreated { creator, vault: vault.clone(), wasm_hash: s.vault_wasm_hash }.publish(&env);
    vault
}

/// A registry asset ID as the vault's sleeve label (as `vault-strategy` does).
fn symbol_string(env: &Env, sym: &Symbol) -> String {
    let s: SymbolStr = SymbolStr::from_val(env, &sym.to_symbol_val());
    let bytes: &[u8] = s.as_ref();
    match core::str::from_utf8(bytes) {
        Ok(text) => String::from_str(env, text),
        Err(_) => panic_with_error!(env, FactoryError::AssetNotRegistered),
    }
}

#[contract]
pub struct VaultFactory;

#[contractimpl]
impl VaultFactory {
    pub fn __constructor(
        env: Env,
        admin: Address,
        vault_wasm_hash: BytesN<32>,
        registry: Address,
        oracle: Address,
        policy: Address,
        treasury: Address,
        max_tvl_cap: i128,
        default_fee_bps: u32,
        keeper: Address,
        router: Option<Address>,
    ) {
        if max_tvl_cap <= 0 || default_fee_bps > MAX_PERFORMANCE_FEE_BPS {
            panic_with_error!(&env, FactoryError::InvalidParams);
        }
        let s = FactorySettings { admin, vault_wasm_hash, registry, oracle, policy, treasury, max_tvl_cap, default_fee_bps,
                                  keeper, router };
        env.storage().instance().set(&DataKey::Settings, &s);
    }

    /// Deploy a private vault for `creator` (who must sign). Returns its address.
    pub fn create_vault(env: Env, creator: Address, params: VaultParams) -> Address {
        creator.require_auth();
        deploy_vault(&env, &creator, params)
    }

    /// Step 3: deploy a private vault and set it up from a signed policy in one
    /// call - the creator signs once. The creator must be the vault's admin and
    /// pause authority (a treasury with a separate multisig admin uses
    /// `create_vault` and the vault's admin calls instead). Then, re-using the
    /// vault's own checks: `bind_policy` (the creator is the policy owner),
    /// `set_sleeves` built from the registry (address, sleeve type, liquidity
    /// tier - never from the caller), Constella's keeper and router. Any
    /// failure reverts everything.
    pub fn create_vault_with_policy(env: Env, creator: Address, params: VaultParams, terms: PolicyTerms) -> Address {
        creator.require_auth();
        if params.admin != creator || params.pause_authority != creator {
            panic_with_error!(&env, FactoryError::InvalidParams);
        }
        let s = settings(&env);
        let vault = deploy_vault(&env, &creator, params);
        let registry = RegistryClient::new(&env, &s.registry);
        let mut sleeves: Vec<SleeveStateRef> = Vec::new(&env);
        for t in terms.sleeves.iter() {
            let entry = match registry.get_asset(&t.asset_id) {
                Some(e) if e.is_active => e,
                _ => panic_with_error!(&env, FactoryError::AssetNotRegistered),
            };
            sleeves.push_back(SleeveStateRef {
                asset_id: symbol_string(&env, &entry.asset_id),
                contract_address: entry.contract_address,
                target_weight_bps: t.target_weight_bps,
                band_bps: terms.rebalance_band_bps,
                current_balance: 0,
                sleeve_type: entry.sleeve_type,
                liquidity_tier: entry.liquidity_tier,
            });
        }
        let v = VaultClient::new(&env, &vault);
        v.bind_policy(&creator, &terms.policy_hash, &terms.min_liquidity_sleeve_bps, &terms.max_single_asset_bps,
                      &terms.max_issuer_bps, &terms.max_slippage_bps, &terms.rebalance_band_bps, &terms.sleeves);
        v.set_sleeves(&sleeves);
        v.set_keeper(&s.keeper);
        if let Some(router) = s.router {
            v.propose_router(&ROUTE_SOROSWAP, &router);
        }
        vault
    }



    /// True only for contracts this factory deployed.
    pub fn is_vault(env: Env, vault: Address) -> bool {
        env.storage().persistent().has(&DataKey::Vault(vault))
    }

    pub fn vault_creator(env: Env, vault: Address) -> Option<Address> {
        env.storage().persistent().get(&DataKey::Vault(vault))
    }

    pub fn vaults_of(env: Env, creator: Address) -> Vec<Address> {
        env.storage().persistent().get(&DataKey::VaultsOf(creator)).unwrap_or_else(|| Vec::new(&env))
    }

    /// Step 3: number of vaults this factory has created.
    pub fn vault_count(env: Env) -> u32 {
        env.storage().instance().get(&DataKey::VaultCount).unwrap_or(0)
    }

    /// Step 3: the `index`-th vault created (0-based), in creation order.
    pub fn vault_at(env: Env, index: u32) -> Option<Address> {
        env.storage().persistent().get(&DataKey::VaultAt(index))
    }

    pub fn get_settings(env: Env) -> FactorySettings {
        settings(&env)
    }

    /// D-O: set Constella's default fee for FUTURE factory vaults. Admin only;
    /// at most the D-D ceiling. Existing vaults keep their fee for life.
    pub fn set_default_fee(env: Env, fee_bps: u32) {
        let mut s = settings(&env);
        s.admin.require_auth();
        if fee_bps > MAX_PERFORMANCE_FEE_BPS {
            panic_with_error!(&env, FactoryError::InvalidParams);
        }
        let previous_bps = s.default_fee_bps;
        s.default_fee_bps = fee_bps;
        env.storage().instance().set(&DataKey::Settings, &s);
        extend(&env);
        DefaultFeeSet { previous_bps, new_bps: fee_bps }.publish(&env);
    }

    /// Propose the WASM for FUTURE vaults; effective after 48 h. Admin only.
    pub fn propose_vault_wasm(env: Env, wasm_hash: BytesN<32>) {
        let s = settings(&env);
        s.admin.require_auth();
        let effective_at = env.ledger().timestamp().saturating_add(WASM_CHANGE_DELAY_SECONDS);
        env.storage().instance().set(&DataKey::PendingWasm, &PendingWasm { wasm_hash: wasm_hash.clone(), effective_at });
        extend(&env);
        VaultWasmProposed { wasm_hash, effective_at }.publish(&env);
    }

    /// Anyone may activate a pending WASM once its delay elapsed.
    pub fn activate_vault_wasm(env: Env) {
        let p: PendingWasm = env.storage().instance().get(&DataKey::PendingWasm)
            .unwrap_or_else(|| panic_with_error!(&env, FactoryError::NoPendingWasm));
        if env.ledger().timestamp() < p.effective_at {
            panic_with_error!(&env, FactoryError::WasmChangeNotReady);
        }
        let mut s = settings(&env);
        s.vault_wasm_hash = p.wasm_hash.clone();
        env.storage().instance().set(&DataKey::Settings, &s);
        env.storage().instance().remove(&DataKey::PendingWasm);
        extend(&env);
        VaultWasmActivated { wasm_hash: p.wasm_hash }.publish(&env);
    }

    /// The admin withdraws a pending WASM change.
    pub fn cancel_vault_wasm(env: Env) {
        settings(&env).admin.require_auth();
        if !env.storage().instance().has(&DataKey::PendingWasm) {
            panic_with_error!(&env, FactoryError::NoPendingWasm);
        }
        env.storage().instance().remove(&DataKey::PendingWasm);
    }

    pub fn get_pending_vault_wasm(env: Env) -> Option<PendingWasm> {
        env.storage().instance().get(&DataKey::PendingWasm)
    }
}

#[cfg(test)]
mod test;
