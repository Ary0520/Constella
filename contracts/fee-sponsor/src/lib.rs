#![no_std]

use soroban_sdk::{contract, contractimpl, Env};

#[contract]
pub struct FeeSponsorContract;

#[contractimpl]
impl FeeSponsorContract {
    pub fn hello(env: Env) -> bool {
        let _ = env;
        true
    }
}
