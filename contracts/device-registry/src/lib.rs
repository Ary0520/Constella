#![no_std]
//! Constella Device Registry: one account, several devices.
//!
//! `add_device(account, device_key, name)` - authorized by the ACCOUNT (its
//! owner passkey signs, like any action) - adds a context rule to the account
//! whose ONLY signer is the new device's passkey (OpenZeppelin requires every
//! signer of a policy-less rule, so each device gets its own rule and signs
//! alone), and records `device key -> (account, rule id)` so the device can
//! sign in on its own later (even after its browser data is cleared).
//!
//! The account's own guard (audit S4) keeps account management on the OWNER
//! rule: an added device can deposit / redeem / withdraw / transfer (its rule
//! is `Default` scope, so it has full spending power), but cannot add or
//! remove devices, change the owner, or propose a recovery (only the guardian
//! rule scoped to the account may; OPEN-ISSUES #1, fixed 24 Nov 2026) - a lost
//! secondary device cannot lock the owner out. Only accounts deployed by the Constella account factory can
//! register devices, and a key that already owns an account (or is already a
//! device) is refused, so lookups are unambiguous.

use soroban_sdk::{
    contract, contractclient, contracterror, contractevent, contractimpl, contracttype, panic_with_error, vec, Address,
    Bytes, BytesN, Env, Map, String, Val, Vec,
};
use stellar_accounts::smart_account::{ContextRule, ContextRuleType, Signer};

const DAY_LEDGERS: u32 = 17_280;
const TTL_THRESHOLD: u32 = 29 * DAY_LEDGERS;
const TTL_EXTEND_TO: u32 = 30 * DAY_LEDGERS;
pub const MAX_DEVICES_PER_ACCOUNT: u32 = 10;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum DeviceError {
    /// Not an account deployed by the Constella account factory.
    UnknownAccount = 1,
    /// Not a 65-byte uncompressed P-256 key.
    InvalidKey = 2,
    /// This key already owns an account or is already a device.
    KeyInUse = 3,
    TooManyDevices = 4,
    NotADevice = 5,
}

/// Where a device key signs: the account and its rule id there.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeviceLink {
    pub account: Address,
    pub rule_id: u32,
    pub name: String,
}

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Settings,
    /// SHA-256(device key) -> DeviceLink
    Device(BytesN<32>),
    /// account -> device key hashes
    DevicesOf(Address),
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Settings {
    pub account_factory: Address,
    pub verifier: Address,
}

#[contractevent(topics = ["DEVICE_ADDED"], data_format = "vec")]
pub struct DeviceAdded {
    #[topic]
    pub account: Address,
    pub rule_id: u32,
    pub device_key: Bytes,
}

#[contractclient(name = "AccountFactoryClient")]
pub trait AccountFactoryRef {
    fn is_account(env: Env, account: Address) -> bool;
    fn account_of(env: Env, owner_key: Bytes) -> Option<Address>;
}

#[contractclient(name = "SmartAccountClient")]
pub trait SmartAccountRef {
    fn add_context_rule(env: Env, context_type: ContextRuleType, name: String, valid_until: Option<u32>,
                        signers: Vec<Signer>, policies: Map<Address, Val>) -> ContextRule;
}

fn settings(env: &Env) -> Settings {
    env.storage().instance().extend_ttl(TTL_THRESHOLD, TTL_EXTEND_TO);
    env.storage().instance().get(&DataKey::Settings).unwrap()
}

fn key_hash(env: &Env, key: &Bytes) -> BytesN<32> {
    if key.len() != 65 || key.get(0) != Some(0x04) {
        panic_with_error!(env, DeviceError::InvalidKey);
    }
    env.crypto().sha256(key).into()
}

fn renew(env: &Env, k: &DataKey) {
    env.storage().persistent().extend_ttl(k, TTL_THRESHOLD, TTL_EXTEND_TO);
}

#[contract]
pub struct DeviceRegistry;

#[contractimpl]
impl DeviceRegistry {
    pub fn __constructor(env: Env, account_factory: Address, verifier: Address) {
        env.storage().instance().set(&DataKey::Settings, &Settings { account_factory, verifier });
    }

    /// Add `device_key` as a device of `account` (the account must authorize).
    pub fn add_device(env: Env, account: Address, device_key: Bytes, name: String) -> u32 {
        account.require_auth();
        let s = settings(&env);
        let factory = AccountFactoryClient::new(&env, &s.account_factory);
        if !factory.is_account(&account) {
            panic_with_error!(&env, DeviceError::UnknownAccount);
        }
        let h = key_hash(&env, &device_key);
        let dk = DataKey::Device(h.clone());
        if env.storage().persistent().has(&dk) || factory.account_of(&device_key).is_some() {
            panic_with_error!(&env, DeviceError::KeyInUse);
        }
        let lk = DataKey::DevicesOf(account.clone());
        let mut list: Vec<BytesN<32>> = env.storage().persistent().get(&lk).unwrap_or_else(|| Vec::new(&env));
        if list.len() >= MAX_DEVICES_PER_ACCOUNT {
            panic_with_error!(&env, DeviceError::TooManyDevices);
        }
        let signer = Signer::External(s.verifier, device_key.clone());
        let rule = SmartAccountClient::new(&env, &account)
            .add_context_rule(&ContextRuleType::Default, &name, &None, &vec![&env, signer], &Map::new(&env));
        env.storage().persistent().set(&dk, &DeviceLink { account: account.clone(), rule_id: rule.id, name });
        renew(&env, &dk);
        list.push_back(h);
        env.storage().persistent().set(&lk, &list);
        renew(&env, &lk);
        DeviceAdded { account, rule_id: rule.id, device_key }.publish(&env);
        rule.id
    }

    /// Where this device key signs, if it was added to an account.
    pub fn device_of(env: Env, device_key: Bytes) -> Option<DeviceLink> {
        let dk = DataKey::Device(key_hash(&env, &device_key));
        env.storage().persistent().get(&dk)
    }

    pub fn devices_of(env: Env, account: Address) -> Vec<DeviceLink> {
        let list: Vec<BytesN<32>> = env.storage().persistent().get(&DataKey::DevicesOf(account)).unwrap_or_else(|| Vec::new(&env));
        let mut out = Vec::new(&env);
        for h in list.iter() {
            if let Some(l) = env.storage().persistent().get::<_, DeviceLink>(&DataKey::Device(h)) {
                out.push_back(l);
            }
        }
        out
    }
}

#[cfg(test)]
mod test;
