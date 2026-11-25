//! Constella recovery registry (step 8; decision D-F; OPEN-ISSUES #2).
//!
//! A user who loses every passkey must not lose their account. The smart
//! account already enforces the rules (D-F / S4): a guardian rule scoped to the
//! account may only PROPOSE a new owner key; it takes effect after 48 hours,
//! the owner can cancel meanwhile, and anyone may execute it afterwards.
//!
//! This registry is the product layer around it (same pattern as the device
//! registry - one fixed address the relayer can sponsor):
//! * `set_guardians` - the account (owner passkey) installs ONE M-of-N guardian
//!   rule: `CallContract(account)` scope, the guardians' passkeys as signers,
//!   the shared threshold policy. The owner key cannot be its own guardian.
//! * `accounts_guarded_by` - a guardian finds the accounts it can help recover.
//! * `record_owner` - after a recovery, anyone may record "new key -> account"
//!   once the key is verifiably in the account's owner rule, so sign-in on the
//!   new device finds the account (the factory only knows the original key).
#![no_std]

use soroban_sdk::{
    contract, contractclient, contracterror, contractevent, contractimpl, contracttype, panic_with_error, Address, Bytes,
    BytesN, Env, IntoVal, Map, String, Val, Vec,
};
use stellar_accounts::{
    policies::simple_threshold::SimpleThresholdAccountParams,
    smart_account::{ContextRule, ContextRuleType, Signer},
};

const DAY_LEDGERS: u32 = 17_280;
const TTL_THRESHOLD: u32 = 29 * DAY_LEDGERS;
const TTL_EXTEND_TO: u32 = 30 * DAY_LEDGERS;
/// At most this many guardians per account (each is a signer the account's
/// `__check_auth` may verify).
pub const MAX_GUARDIANS: u32 = 5;
/// A guardian key may guard at most this many accounts (bounded index).
pub const MAX_GUARDED_PER_KEY: u32 = 20;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum RecoveryError {
    /// Not an account deployed by the Constella account factory.
    UnknownAccount = 1,
    /// Not a 65-byte uncompressed P-256 key, or the account's own owner key.
    InvalidKey = 2,
    /// Threshold must be 1..=number of guardians.
    InvalidThreshold = 3,
    DuplicateGuardian = 4,
    /// The account already has guardians.
    GuardiansAlreadySet = 5,
    TooManyGuardians = 6,
    /// The key is not (or no longer) a signer of the account's owner rule.
    NotOwnerKey = 7,
    /// This guardian key already guards MAX_GUARDED_PER_KEY accounts.
    GuardianOfTooMany = 8,
    /// The account has no pending recovery.
    NoPendingRecovery = 9,
}

/// Mirror of `constella_smart_account::PendingRecovery` (decoded by field name).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingRecoveryRef {
    pub new_owner: Signer,
    pub executable_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GuardianSet {
    pub rule_id: u32,
    pub threshold: u32,
    pub guardians: Vec<Bytes>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Settings {
    pub account_factory: Address,
    pub verifier: Address,
    pub threshold_policy: Address,
}

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Settings,
    Guardians(Address),
    /// SHA-256(guardian key) -> accounts it guards
    GuardedBy(BytesN<32>),
    /// SHA-256(owner key) -> account (recorded after a recovery)
    Owner(BytesN<32>),
}

#[contractevent(topics = ["GUARDIANS_SET"], data_format = "vec")]
pub struct GuardiansSet {
    #[topic]
    pub account: Address,
    pub rule_id: u32,
    pub threshold: u32,
    pub count: u32,
}

#[contractevent(topics = ["OWNER_RECORDED"], data_format = "single-value")]
pub struct OwnerRecorded {
    #[topic]
    pub account: Address,
    pub key: Bytes,
}

#[contractclient(name = "AccountFactoryClient")]
pub trait AccountFactoryRef {
    fn is_account(env: Env, account: Address) -> bool;
}

#[contractclient(name = "SmartAccountClient")]
pub trait SmartAccountRef {
    fn add_context_rule(env: Env, context_type: ContextRuleType, name: String, valid_until: Option<u32>,
                        signers: Vec<Signer>, policies: Map<Address, Val>) -> ContextRule;
    fn get_owner_rule(env: Env) -> u32;
    fn get_context_rule(env: Env, context_rule_id: u32) -> ContextRule;
    fn get_pending_recovery(env: Env) -> Option<PendingRecoveryRef>;
    fn execute_recovery(env: Env);
}

fn settings(env: &Env) -> Settings {
    env.storage().instance().extend_ttl(TTL_THRESHOLD, TTL_EXTEND_TO);
    env.storage().instance().get(&DataKey::Settings).unwrap()
}

fn key_hash(env: &Env, key: &Bytes) -> BytesN<32> {
    if key.len() != 65 || key.get(0) != Some(0x04) {
        panic_with_error!(env, RecoveryError::InvalidKey);
    }
    env.crypto().sha256(key).into()
}

fn renew(env: &Env, k: &DataKey) {
    env.storage().persistent().extend_ttl(k, TTL_THRESHOLD, TTL_EXTEND_TO);
}

fn owner_signers(env: &Env, account: &Address) -> Vec<Signer> {
    let acc = SmartAccountClient::new(env, account);
    acc.get_context_rule(&acc.get_owner_rule()).signers
}

#[contract]
pub struct RecoveryRegistry;

#[contractimpl]
impl RecoveryRegistry {
    pub fn __constructor(env: Env, account_factory: Address, verifier: Address, threshold_policy: Address) {
        env.storage().instance().set(&DataKey::Settings, &Settings { account_factory, verifier, threshold_policy });
    }

    /// The account (its owner passkey signs) installs its M-of-N guardian rule.
    pub fn set_guardians(env: Env, account: Address, guardians: Vec<Bytes>, threshold: u32) -> u32 {
        account.require_auth();
        let s = settings(&env);
        if !AccountFactoryClient::new(&env, &s.account_factory).is_account(&account) {
            panic_with_error!(&env, RecoveryError::UnknownAccount);
        }
        let gk = DataKey::Guardians(account.clone());
        if env.storage().persistent().has(&gk) {
            panic_with_error!(&env, RecoveryError::GuardiansAlreadySet);
        }
        if guardians.len() > MAX_GUARDIANS {
            panic_with_error!(&env, RecoveryError::TooManyGuardians);
        }
        if threshold == 0 || threshold > guardians.len() {
            panic_with_error!(&env, RecoveryError::InvalidThreshold);
        }
        let owners = owner_signers(&env, &account);
        let mut hashes: Vec<BytesN<32>> = Vec::new(&env);
        let mut signers: Vec<Signer> = Vec::new(&env);
        for g in guardians.iter() {
            let h = key_hash(&env, &g);
            if hashes.contains(&h) {
                panic_with_error!(&env, RecoveryError::DuplicateGuardian);
            }
            let signer = Signer::External(s.verifier.clone(), g.clone());
            if owners.contains(&signer) {
                panic_with_error!(&env, RecoveryError::InvalidKey); // the owner cannot guard itself
            }
            hashes.push_back(h);
            signers.push_back(signer);
        }
        let mut policies: Map<Address, Val> = Map::new(&env);
        policies.set(s.threshold_policy.clone(), SimpleThresholdAccountParams { threshold }.into_val(&env));
        let rule = SmartAccountClient::new(&env, &account).add_context_rule(
            &ContextRuleType::CallContract(account.clone()), &String::from_str(&env, "recovery"), &None, &signers, &policies);
        env.storage().persistent().set(&gk, &GuardianSet { rule_id: rule.id, threshold, guardians: guardians.clone() });
        renew(&env, &gk);
        for h in hashes.iter() {
            let k = DataKey::GuardedBy(h);
            let mut list: Vec<Address> = env.storage().persistent().get(&k).unwrap_or_else(|| Vec::new(&env));
            if list.len() >= MAX_GUARDED_PER_KEY {
                panic_with_error!(&env, RecoveryError::GuardianOfTooMany);
            }
            list.push_back(account.clone());
            env.storage().persistent().set(&k, &list);
            renew(&env, &k);
        }
        GuardiansSet { account, rule_id: rule.id, threshold, count: guardians.len() }.publish(&env);
        rule.id
    }

    pub fn guardians_of(env: Env, account: Address) -> Option<GuardianSet> {
        env.storage().persistent().get(&DataKey::Guardians(account))
    }

    /// The accounts this guardian key can help recover.
    pub fn accounts_guarded_by(env: Env, key: Bytes) -> Vec<Address> {
        env.storage().persistent().get(&DataKey::GuardedBy(key_hash(&env, &key))).unwrap_or_else(|| Vec::new(&env))
    }

    /// After a recovery: record that `key` now owns `account` (permissionless;
    /// verified against the account's owner rule).
    pub fn record_owner(env: Env, account: Address, key: Bytes) {
        let s = settings(&env);
        let h = key_hash(&env, &key);
        if !AccountFactoryClient::new(&env, &s.account_factory).is_account(&account)
            || !owner_signers(&env, &account).contains(Signer::External(s.verifier, key.clone())) {
            panic_with_error!(&env, RecoveryError::NotOwnerKey);
        }
        let k = DataKey::Owner(h);
        env.storage().persistent().set(&k, &account);
        renew(&env, &k);
        OwnerRecorded { account, key }.publish(&env);
    }

    /// Finish a due recovery in one permissionless call: execute it on the
    /// account (which enforces the 48 h delay) and record the new owner key.
    /// Returns that key. The relayer builds this call itself (no signature).
    pub fn finish(env: Env, account: Address) -> Bytes {
        let acc = SmartAccountClient::new(&env, &account);
        let pending = match acc.get_pending_recovery() {
            Some(p) => p,
            None => panic_with_error!(&env, RecoveryError::NoPendingRecovery),
        };
        let key = match pending.new_owner {
            Signer::External(_, key) => key,
            _ => panic_with_error!(&env, RecoveryError::NotOwnerKey),
        };
        acc.execute_recovery();
        Self::record_owner(env, account, key.clone());
        key
    }

    /// The account a recovered owner key controls, if recorded.
    pub fn owner_account(env: Env, key: Bytes) -> Option<Address> {
        env.storage().persistent().get(&DataKey::Owner(key_hash(&env, &key)))
    }
}

#[cfg(test)]
mod test;
