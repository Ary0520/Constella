//! Constella recovery registry (step 8; decision D-F; OPEN-ISSUES #2).
//! Stub: the interface the tests drive; implemented in the next commit.
#![no_std]

use soroban_sdk::{contract, contracterror, contractimpl, contracttype, panic_with_error, Address, Bytes, Env, Vec};

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum RecoveryError {
    UnknownAccount = 1,
    InvalidKey = 2,
    InvalidThreshold = 3,
    DuplicateGuardian = 4,
    GuardiansAlreadySet = 5,
    TooManyGuardians = 6,
    NotOwnerKey = 7,
    GuardianOfTooMany = 8,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GuardianSet {
    pub rule_id: u32,
    pub threshold: u32,
    pub guardians: Vec<Bytes>,
}

#[contract]
pub struct RecoveryRegistry;

#[contractimpl]
impl RecoveryRegistry {
    pub fn __constructor(_env: Env, _account_factory: Address, _verifier: Address, _threshold_policy: Address) {}
    pub fn set_guardians(env: Env, _account: Address, _guardians: Vec<Bytes>, _threshold: u32) -> u32 {
        panic_with_error!(&env, RecoveryError::UnknownAccount)
    }
    pub fn guardians_of(_env: Env, _account: Address) -> Option<GuardianSet> { None }
    pub fn accounts_guarded_by(env: Env, _key: Bytes) -> Vec<Address> { Vec::new(&env) }
    pub fn record_owner(env: Env, _account: Address, _key: Bytes) {
        panic_with_error!(&env, RecoveryError::NotOwnerKey)
    }
    pub fn owner_account(_env: Env, _key: Bytes) -> Option<Address> { None }
}

#[cfg(test)]
mod test;
