//! Constella Private Vault Factory (decision D-C).
//!
//! Treasuries create their own private vault - the same audited
//! `vault-strategy` code as the shared vault - without Constella deploying it
//! for them. The split of control is the point:
//!
//! * **Fixed by the factory** (a creator cannot choose them): the vault WASM,
//!   the shared registry, NAV oracle and policy contracts, and the fee
//!   treasury. A vault can never be pointed at a fake price feed or registry.
//! * **Chosen by the creator**, within protocol bounds: the vault admin and
//!   pause authority (their own multisig / smart account), the numéraire (a
//!   registered cash asset - the vault constructor enforces it), the
//!   performance fee (<= 10 % of profit, D-D) and the caps (<= the factory's
//!   `max_tvl_cap`, AGENT-RULES Rule 6).
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
    contract, contracterror, contractevent, contractimpl, contracttype, ContractExecutable, panic_with_error,
    xdr::ToXdr, Address, Bytes,
    BytesN, Env, IntoVal, String, Val, Vec,
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
}

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
    ) {
        if max_tvl_cap <= 0 {
            panic_with_error!(&env, FactoryError::InvalidParams);
        }
        let s = FactorySettings { admin, vault_wasm_hash, registry, oracle, policy, treasury, max_tvl_cap };
        env.storage().instance().set(&DataKey::Settings, &s);
    }

    /// Deploy a private vault for `creator` (who must sign). Returns its address.
    pub fn create_vault(env: Env, creator: Address, params: VaultParams) -> Address {
        creator.require_auth();
        let s = settings(&env);
        if params.performance_fee_bps > MAX_PERFORMANCE_FEE_BPS
            || params.tvl_cap <= 0
            || params.per_user_cap <= 0
            || params.per_user_cap > params.tvl_cap
        {
            panic_with_error!(&env, FactoryError::InvalidParams);
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
        env.storage().persistent().extend_ttl(&DataKey::VaultsOf(creator.clone()), TTL_THRESHOLD, TTL_EXTEND_TO);
        extend(&env);
        VaultCreated { creator, vault: vault.clone(), wasm_hash: s.vault_wasm_hash }.publish(&env);
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

    pub fn get_settings(env: Env) -> FactorySettings {
        settings(&env)
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
