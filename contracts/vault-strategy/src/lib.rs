#![no_std]

use soroban_sdk::{contract, contractimpl, Env};

#[contract]
pub struct StrategyVault;

#[contractimpl]
impl StrategyVault {
    pub fn ping(_env: Env) -> bool {
        true
    }
}
