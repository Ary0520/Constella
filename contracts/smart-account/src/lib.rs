//! Constella passkey smart account on the OpenZeppelin smart-account framework
//! (audit W4.3b; `stellar-accounts` 0.8.0).
//!
//! Authorization is context-centric: every operation the account authorizes
//! (`auth_contexts`) must be matched, by an ID the signer chooses and signs,
//! to a stored context rule that scopes what its signers may do:
//!
//! * **Owner rule** (installed by the constructor): `Default` scope, the
//!   owner's passkey. Full control. It can never be removed, never expire and
//!   never lose its last signer.
//! * **Recovery rule** (added by the owner): scoped to `CallContract(self)` -
//!   the account's own management functions only - with M-of-N backup /
//!   guardian passkeys under the threshold policy. Recovery signers can rotate
//!   the owner key but can NOT authorize transfers, deposits or any other
//!   contract call.
//!
//! Passkeys are `Signer::External(webauthn_verifier, public_key)`; each one
//! signs a WebAuthn assertion over SHA-256(account, signature_payload,
//! context_rule_ids), so the chosen rules are part of what the user approves.
//!
//! There is deliberately no `execute` entry point: a self-scoped recovery rule
//! must not become a path to arbitrary calls.
#![no_std]

use soroban_sdk::{
    auth::{Context, CustomAccountInterface},
    contract, contractclient, contracterror, contractevent, contractimpl, contracttype,
    crypto::Hash,
    panic_with_error, vec, Address, Bytes, BytesN, Env, Map, String, Val, Vec,
};
use stellar_accounts::smart_account::{
    self as oz, AuthDigestPreimage, AuthPayload, ContextRule, ContextRuleType, Signer, SmartAccount, SmartAccountError,
};

// Constella's own event (Rule 8 name), byte-identical to the pre-W4.3b
// encoding. Rule / signer / policy management events are emitted by the
// OpenZeppelin library itself (context_rule_added, signer_registered, ...).
#[contractevent(topics = ["ACCOUNT_INITIALIZED"], data_format = "single-value")]
pub struct AccountInitialized {
    #[topic]
    pub account: Address,
    pub public_key: BytesN<65>,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum AccError {
    /// The owner key is not a 65-byte uncompressed SEC-1 P-256 point.
    InvalidPublicKey = 1,
    /// The owner rule cannot be removed or given an expiry (anti-bricking).
    OwnerRuleProtected = 2,
    /// The owner rule must keep at least one signer (anti-bricking).
    OwnerRuleNeedsSigner = 3,
    /// A threshold policy on the rule would require more signers than remain.
    ThresholdUnreachable = 4,
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    OwnerRule,
}

/// Read-only view implemented by `constella-threshold-policy`; used to keep a
/// governed rule's signer count >= its threshold.
#[contractclient(name = "ThresholdQueryClient")]
pub trait ThresholdQuery {
    fn get_threshold(e: Env, context_rule_id: u32, smart_account: Address) -> u32;
}

fn owner_rule_id(e: &Env) -> u32 {
    e.storage().instance().get(&DataKey::OwnerRule).unwrap_or_else(|| panic_with_error!(e, SmartAccountError::ContextRuleNotFound))
}

fn validate_owner_key(e: &Env, key: &Bytes) -> BytesN<65> {
    if key.len() < 65 {
        panic_with_error!(e, AccError::InvalidPublicKey);
    }
    let pk = match BytesN::<65>::try_from(key.slice(0..65)) {
        Ok(pk) => pk,
        Err(_) => panic_with_error!(e, AccError::InvalidPublicKey),
    };
    let a = pk.to_array();
    if a[0] != 0x04 || a[1..].iter().all(|&b| b == 0) {
        panic_with_error!(e, AccError::InvalidPublicKey);
    }
    pk
}

/// OZ warns that a threshold policy is not told about signer changes; refuse
/// any change that would make a governed rule impossible to satisfy.
fn check_thresholds_reachable(e: &Env, rule: &ContextRule) {
    let account = e.current_contract_address();
    for policy in rule.policies.iter() {
        if let Ok(Ok(threshold)) = ThresholdQueryClient::new(e, &policy).try_get_threshold(&rule.id, &account) {
            if threshold > rule.signers.len() {
                panic_with_error!(e, AccError::ThresholdUnreachable);
            }
        }
    }
}

#[contract]
pub struct SmartAccountContract;

#[contractimpl]
impl CustomAccountInterface for SmartAccountContract {
    type Signature = AuthPayload;
    type Error = SmartAccountError;

    fn __check_auth(
        e: Env,
        signature_payload: Hash<32>,
        signatures: AuthPayload,
        auth_contexts: Vec<Context>,
    ) -> Result<(), SmartAccountError> {
        oz::do_check_auth(&e, &signature_payload, &signatures, &auth_contexts)
    }
}

#[contractimpl(contracttrait)]
impl SmartAccount for SmartAccountContract {
    fn remove_context_rule(e: &Env, context_rule_id: u32) {
        e.current_contract_address().require_auth();
        if context_rule_id == owner_rule_id(e) {
            panic_with_error!(e, AccError::OwnerRuleProtected);
        }
        oz::remove_context_rule(e, context_rule_id);
    }

    fn update_context_rule_valid_until(e: &Env, context_rule_id: u32, valid_until: Option<u32>) -> ContextRule {
        e.current_contract_address().require_auth();
        if context_rule_id == owner_rule_id(e) && valid_until.is_some() {
            panic_with_error!(e, AccError::OwnerRuleProtected);
        }
        oz::update_context_rule_valid_until(e, context_rule_id, valid_until)
    }

    fn remove_signer(e: &Env, context_rule_id: u32, signer_id: u32) {
        e.current_contract_address().require_auth();
        oz::remove_signer(e, context_rule_id, signer_id);
        let rule = oz::get_context_rule(e, context_rule_id);
        if context_rule_id == owner_rule_id(e) && rule.signers.is_empty() {
            panic_with_error!(e, AccError::OwnerRuleNeedsSigner);
        }
        check_thresholds_reachable(e, &rule);
    }
}

#[contractimpl]
impl SmartAccountContract {
    /// Installs the owner rule atomically with deployment, so no one can claim
    /// a fresh account (audit M3). `owner_key` is the passkey's 65-byte P-256
    /// public key (optionally followed by its credential ID); `verifier` is
    /// the shared `constella-webauthn-verifier` deployment.
    pub fn __constructor(e: Env, verifier: Address, owner_key: Bytes, name: String) {
        let pk = validate_owner_key(&e, &owner_key);
        let owner = Signer::External(verifier, owner_key);
        let rule = oz::add_context_rule(&e, &ContextRuleType::Default, &name, None, &vec![&e, owner], &Map::new(&e));
        e.storage().instance().set(&DataKey::OwnerRule, &rule.id);
        AccountInitialized { account: e.current_contract_address(), public_key: pk }.publish(&e);
    }

    /// ID of the protected owner rule.
    pub fn get_owner_rule(e: Env) -> u32 {
        owner_rule_id(&e)
    }
}

#[cfg(test)]
mod test;
