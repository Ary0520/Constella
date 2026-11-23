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
//!   guardian passkeys under the threshold policy. Guardians can only
//!   *propose* a new owner key; it takes effect after a 48-hour delay unless
//!   the owner cancels (decision D-F, audit S4). They can NOT authorize
//!   transfers, deposits, any other contract call, or any other management
//!   call: only the owner rule may manage the account.
//!
//! Passkeys are `Signer::External(webauthn_verifier, public_key)`; each one
//! signs a WebAuthn assertion over SHA-256(account, signature_payload,
//! context_rule_ids), so the chosen rules are part of what the user approves.
//!
//! There is deliberately no `execute` entry point: a self-scoped recovery rule
//! must not become a path to arbitrary calls.
#![no_std]

use soroban_sdk::{
    auth::{Context, ContractContext, CustomAccountInterface},
    contract, contractclient, contracterror, contractevent, contractimpl, contracttype,
    crypto::Hash,
    panic_with_error, vec, Address, Bytes, BytesN, Env, Map, String, Symbol, Val, Vec,
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

/// Guardians proposed `new_owner`; executable at `executable_at` (audit S4).
#[contractevent(topics = ["RECOVERY_PROPOSED"], data_format = "vec")]
pub struct RecoveryProposed {
    #[topic]
    pub account: Address,
    pub new_owner: Signer,
    pub executable_at: u64,
}

#[contractevent(topics = ["RECOVERY_CANCELLED"], data_format = "single-value")]
pub struct RecoveryCancelled {
    #[topic]
    pub account: Address,
    pub new_owner: Signer,
}

#[contractevent(topics = ["RECOVERY_EXECUTED"], data_format = "single-value")]
pub struct RecoveryExecuted {
    #[topic]
    pub account: Address,
    pub new_owner: Signer,
}

/// Delay before a guardian-proposed recovery can take effect (decision D-F):
/// 48 h, the same window as the vault upgrade timelock and the policy
/// amendment notice period.
pub const RECOVERY_DELAY_SECONDS: u64 = 48 * 3600;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingRecovery {
    pub new_owner: Signer,
    pub executable_at: u64,
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
    /// Only the owner rule may authorize account management; other rules may
    /// only propose a recovery (audit S4).
    ManagementRequiresOwner = 5,
    /// No recovery is pending.
    NoPendingRecovery = 6,
    /// The 48-hour recovery delay has not elapsed.
    RecoveryNotReady = 7,
    /// The proposed key already belongs to the owner rule.
    AlreadyOwner = 8,
    /// Only a rule scoped to this account (the guardian recovery rule,
    /// `CallContract(self)`) may propose a recovery; a `Default` rule such as
    /// a device rule may not (OPEN-ISSUES #1).
    RecoveryRequiresRecoveryRule = 9,
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    OwnerRule,
    PendingRecovery,
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
        oz::do_check_auth(&e, &signature_payload, &signatures, &auth_contexts)?;
        // Audit S4: a call to this account's own functions authorized under
        // any rule other than the owner rule may only propose a recovery.
        // (do_check_auth has already checked one rule id per context.)
        //
        // OPEN-ISSUES #1: and only a rule scoped to THIS account (the guardian
        // recovery rule, `CallContract(self)`) may propose it. OpenZeppelin
        // lets a `Default` rule authorize any context, so without this check a
        // device rule (Default, added by the device registry) could start a
        // takeover of the owner key.
        let me = e.current_contract_address();
        let owner = owner_rule_id(&e);
        let propose = Symbol::new(&e, "propose_recovery");
        let recovery_scope = ContextRuleType::CallContract(me.clone());
        for (i, context) in auth_contexts.iter().enumerate() {
            if let Context::Contract(ContractContext { contract, fn_name, .. }) = context {
                let rule_id = signatures.context_rule_ids.get_unchecked(i as u32);
                if contract != me || rule_id == owner {
                    continue;
                }
                if fn_name != propose {
                    panic_with_error!(&e, AccError::ManagementRequiresOwner);
                }
                if oz::get_context_rule(&e, rule_id).context_type != recovery_scope {
                    panic_with_error!(&e, AccError::RecoveryRequiresRecoveryRule);
                }
            }
        }
        Ok(())
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

    /// Guardians propose a new owner key (audit S4). It can be executed after
    /// `RECOVERY_DELAY_SECONDS`; a new proposal replaces the pending one and
    /// restarts the clock. Authorized through `__check_auth` (recovery rule).
    pub fn propose_recovery(e: Env, new_owner: Signer) {
        e.current_contract_address().require_auth();
        oz::validate_signer_key_size(&e, &new_owner);
        if oz::get_context_rule(&e, owner_rule_id(&e)).signers.contains(&new_owner) {
            panic_with_error!(&e, AccError::AlreadyOwner);
        }
        let executable_at = e.ledger().timestamp().saturating_add(RECOVERY_DELAY_SECONDS);
        e.storage().instance().set(&DataKey::PendingRecovery, &PendingRecovery { new_owner: new_owner.clone(), executable_at });
        RecoveryProposed { account: e.current_contract_address(), new_owner, executable_at }.publish(&e);
    }

    /// The owner cancels a pending recovery (owner rule only, see __check_auth).
    pub fn cancel_recovery(e: Env) {
        e.current_contract_address().require_auth();
        let pending: PendingRecovery = e.storage().instance().get(&DataKey::PendingRecovery)
            .unwrap_or_else(|| panic_with_error!(&e, AccError::NoPendingRecovery));
        e.storage().instance().remove(&DataKey::PendingRecovery);
        RecoveryCancelled { account: e.current_contract_address(), new_owner: pending.new_owner }.publish(&e);
    }

    /// After the delay, anyone may execute the recovery: the owner rule's
    /// signers are replaced by the proposed key (added first, so the owner
    /// rule is never empty). No authorization: the guardians' threshold and
    /// the owner's 48-hour chance to cancel already happened.
    pub fn execute_recovery(e: Env) {
        let pending: PendingRecovery = e.storage().instance().get(&DataKey::PendingRecovery)
            .unwrap_or_else(|| panic_with_error!(&e, AccError::NoPendingRecovery));
        if e.ledger().timestamp() < pending.executable_at {
            panic_with_error!(&e, AccError::RecoveryNotReady);
        }
        let owner = owner_rule_id(&e);
        let old_ids = oz::get_context_rule(&e, owner).signer_ids;
        oz::add_signer(&e, owner, &pending.new_owner);
        for id in old_ids.iter() {
            oz::remove_signer(&e, owner, id);
        }
        e.storage().instance().remove(&DataKey::PendingRecovery);
        RecoveryExecuted { account: e.current_contract_address(), new_owner: pending.new_owner }.publish(&e);
    }

    pub fn get_pending_recovery(e: Env) -> Option<PendingRecovery> {
        e.storage().instance().get(&DataKey::PendingRecovery)
    }
}

#[cfg(test)]
mod test;
