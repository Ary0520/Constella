#![no_std]
//! Constella Account Factory (audit A2: sponsored account creation).
//!
//! A new user holds no XLM, so somebody else must deploy their passkey smart
//! account. `create_account(owner_key)` is PERMISSIONLESS and deploys the
//! audited smart-account WASM with the factory's fixed WebAuthn verifier, at
//! an address derived from the key alone (salt = SHA-256(owner_key)). The
//! same key always gets the same address, which anyone can compute beforehand
//! (`address_for`), so nobody can squat it; only that key controls the
//! account, so creating it "for" someone is harmless; and a repeated call
//! returns the existing account (idempotent).
//! The Constella relayer pays the fee (rate-limited off chain, budgeted on
//! chain by the fee-sponsor). Code for FUTURE accounts changes only after a
//! public 48 h delay.

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, panic_with_error, Address, Bytes, BytesN, Env,
    IntoVal, String, Val, Vec, ContractExecutable,
};

pub const WASM_CHANGE_DELAY_SECONDS: u64 = 48 * 3600;
const DAY_LEDGERS: u32 = 17_280;
const TTL_THRESHOLD: u32 = 29 * DAY_LEDGERS;
const TTL_EXTEND_TO: u32 = 30 * DAY_LEDGERS;
/// Uncompressed P-256 public key (0x04 || X || Y).
pub const OWNER_KEY_LEN: u32 = 65;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum AccountFactoryError {
    /// Not a 65-byte uncompressed P-256 key.
    InvalidOwnerKey = 1,
    NoPendingWasm = 2,
    WasmChangeNotReady = 3,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Settings {
    pub admin: Address,
    pub verifier: Address,
    pub account_wasm_hash: BytesN<32>,
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
    /// SHA-256(owner_key) -> account
    AccountOf(BytesN<32>),
    /// account -> true (origin attestation)
    IsAccount(Address),
}

#[contractevent(topics = ["ACCOUNT_CREATED"], data_format = "vec")]
pub struct AccountCreated {
    #[topic]
    pub account: Address,
    pub owner_key: Bytes,
    pub wasm_hash: BytesN<32>,
}

#[contractevent(topics = ["ACCOUNT_WASM_PROPOSED"], data_format = "vec")]
pub struct AccountWasmProposed {
    pub wasm_hash: BytesN<32>,
    pub effective_at: u64,
}

#[contractevent(topics = ["ACCOUNT_WASM_ACTIVATED"], data_format = "single-value")]
pub struct AccountWasmActivated {
    pub wasm_hash: BytesN<32>,
}

fn settings(env: &Env) -> Settings {
    env.storage().instance().extend_ttl(TTL_THRESHOLD, TTL_EXTEND_TO);
    env.storage().instance().get(&DataKey::Settings).unwrap()
}

fn salt_of(env: &Env, owner_key: &Bytes) -> BytesN<32> {
    if owner_key.len() != OWNER_KEY_LEN || owner_key.get(0) != Some(0x04) {
        panic_with_error!(env, AccountFactoryError::InvalidOwnerKey);
    }
    env.crypto().sha256(owner_key).into()
}

fn renew(env: &Env, key: &DataKey) {
    env.storage().persistent().extend_ttl(key, TTL_THRESHOLD, TTL_EXTEND_TO);
}

#[contract]
pub struct AccountFactory;

#[contractimpl]
impl AccountFactory {
    pub fn __constructor(env: Env, admin: Address, verifier: Address, account_wasm_hash: BytesN<32>) {
        env.storage().instance().set(&DataKey::Settings, &Settings { admin, verifier, account_wasm_hash });
    }

    /// Deploy (or return) the passkey account controlled by `owner_key`.
    pub fn create_account(env: Env, owner_key: Bytes) -> Address {
        let s = settings(&env);
        let salt = salt_of(&env, &owner_key);
        let key = DataKey::AccountOf(salt.clone());
        if let Some(existing) = env.storage().persistent().get::<_, Address>(&key) {
            renew(&env, &key);
            return existing;
        }
        let args: Vec<Val> = (s.verifier.clone(), owner_key.clone(), String::from_str(&env, "owner")).into_val(&env);
        let account = env.deployer().with_current_contract(salt)
            .deploy_contract(ContractExecutable::Wasm(s.account_wasm_hash.clone()), args);
        env.storage().persistent().set(&key, &account);
        renew(&env, &key);
        let is = DataKey::IsAccount(account.clone());
        env.storage().persistent().set(&is, &true);
        renew(&env, &is);
        AccountCreated { account: account.clone(), owner_key, wasm_hash: s.account_wasm_hash }.publish(&env);
        account
    }

    /// The address `create_account(owner_key)` deploys to (deployed or not).
    pub fn address_for(env: Env, owner_key: Bytes) -> Address {
        let salt = salt_of(&env, &owner_key);
        env.deployer().with_current_contract(salt).deployed_address()
    }

    pub fn account_of(env: Env, owner_key: Bytes) -> Option<Address> {
        let salt = salt_of(&env, &owner_key);
        env.storage().persistent().get(&DataKey::AccountOf(salt))
    }

    /// True only for accounts this factory deployed.
    pub fn is_account(env: Env, account: Address) -> bool {
        env.storage().persistent().has(&DataKey::IsAccount(account))
    }

    pub fn get_settings(env: Env) -> Settings {
        settings(&env)
    }

    /// Propose the WASM for FUTURE accounts; effective after 48 h. Admin only.
    pub fn propose_account_wasm(env: Env, wasm_hash: BytesN<32>) {
        settings(&env).admin.require_auth();
        let effective_at = env.ledger().timestamp().saturating_add(WASM_CHANGE_DELAY_SECONDS);
        env.storage().instance().set(&DataKey::PendingWasm, &PendingWasm { wasm_hash: wasm_hash.clone(), effective_at });
        AccountWasmProposed { wasm_hash, effective_at }.publish(&env);
    }

    /// Anyone may activate a pending WASM once its delay elapsed.
    pub fn activate_account_wasm(env: Env) {
        let mut s = settings(&env);
        let p: PendingWasm = env.storage().instance().get(&DataKey::PendingWasm)
            .unwrap_or_else(|| panic_with_error!(&env, AccountFactoryError::NoPendingWasm));
        if env.ledger().timestamp() < p.effective_at {
            panic_with_error!(&env, AccountFactoryError::WasmChangeNotReady);
        }
        s.account_wasm_hash = p.wasm_hash.clone();
        env.storage().instance().set(&DataKey::Settings, &s);
        env.storage().instance().remove(&DataKey::PendingWasm);
        AccountWasmActivated { wasm_hash: p.wasm_hash }.publish(&env);
    }

    pub fn cancel_account_wasm(env: Env) {
        settings(&env).admin.require_auth();
        if !env.storage().instance().has(&DataKey::PendingWasm) {
            panic_with_error!(&env, AccountFactoryError::NoPendingWasm);
        }
        env.storage().instance().remove(&DataKey::PendingWasm);
    }

    pub fn get_pending_account_wasm(env: Env) -> Option<PendingWasm> {
        env.storage().instance().get(&DataKey::PendingWasm)
    }
}

#[cfg(test)]
mod test;
