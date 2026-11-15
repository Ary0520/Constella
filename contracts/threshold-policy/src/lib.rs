//! M-of-N threshold policy for OpenZeppelin smart accounts (audit W4.3b).
//! One deployment serves every Constella account; state is keyed by
//! (smart_account, context_rule_id). Used for social recovery: e.g. 2 of
//! {backup passkey, guardian A, guardian B} may rotate the owner passkey.
//!
//! Every function is OpenZeppelin's `policies::simple_threshold`, and each one
//! requires the smart account's own authorization (the account is the direct
//! caller when it installs or enforces the policy).
//!
//! OZ warns that the stored threshold is not updated when the rule's signers
//! change. The Constella smart account therefore refuses any signer change
//! that would leave a governed rule with fewer signers than its threshold.
#![no_std]

use soroban_sdk::{auth::Context, contract, contractimpl, Address, Env, Vec};
use stellar_accounts::{
    policies::{
        simple_threshold::{self, SimpleThresholdAccountParams},
        Policy,
    },
    smart_account::{ContextRule, Signer},
};

#[contract]
pub struct ThresholdPolicy;

#[contractimpl]
impl Policy for ThresholdPolicy {
    type AccountParams = SimpleThresholdAccountParams;

    fn enforce(e: &Env, context: Context, authenticated_signers: Vec<Signer>, context_rule: ContextRule, smart_account: Address) {
        simple_threshold::enforce(e, &context, &authenticated_signers, &context_rule, &smart_account);
    }

    fn install(e: &Env, install_params: SimpleThresholdAccountParams, context_rule: ContextRule, smart_account: Address) {
        simple_threshold::install(e, &install_params, &context_rule, &smart_account);
    }

    fn uninstall(e: &Env, context_rule: ContextRule, smart_account: Address) {
        simple_threshold::uninstall(e, &context_rule, &smart_account);
    }
}

#[contractimpl]
impl ThresholdPolicy {
    /// Current threshold for (smart_account, rule).
    pub fn get_threshold(e: &Env, context_rule_id: u32, smart_account: Address) -> u32 {
        simple_threshold::get_threshold(e, context_rule_id, &smart_account)
    }

    /// Changes the threshold (1..=signers of the rule); the account authorizes.
    pub fn set_threshold(e: &Env, threshold: u32, context_rule: ContextRule, smart_account: Address) {
        simple_threshold::set_threshold(e, threshold, &context_rule, &smart_account);
    }
}

#[cfg(test)]
mod test;
