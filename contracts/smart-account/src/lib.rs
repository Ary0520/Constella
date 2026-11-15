#![no_std]

use soroban_sdk::{contract, contractimpl, Env};

#[contract]
pub struct SmartAccountContract;

#[contractimpl]
impl SmartAccountContract {
    pub fn hello(env: Env) -> bool {
        let _ = env;
        true
    }
}
