#![no_std]

use soroban_sdk::{
    auth::{Context, CustomAccountInterface},
    contract, contracterror, contractevent, contractimpl, contracttype,
    crypto::Hash,
    Address, BytesN, Env, String, Vec,
};

// Events (audit W4.1b: typed #[contractevent]s, byte-identical to the former
// Events::publish encodings; pinned by golden/events.txt). Each has topics
// [NAME, account] and the affected key / threshold as single-value data.

#[contractevent(topics = ["ACCOUNT_INITIALIZED"], data_format = "single-value")]
pub struct AccountInitialized {
    #[topic]
    pub account: Address,
    pub public_key: BytesN<65>,
}

#[contractevent(topics = ["SIGNER_ADDED"], data_format = "single-value")]
pub struct SignerAdded {
    #[topic]
    pub account: Address,
    pub public_key: BytesN<65>,
}

#[contractevent(topics = ["SIGNER_UPDATED"], data_format = "single-value")]
pub struct SignerUpdated {
    #[topic]
    pub account: Address,
    pub public_key: BytesN<65>,
}

#[contractevent(topics = ["SIGNER_REMOVED"], data_format = "single-value")]
pub struct SignerRemoved {
    #[topic]
    pub account: Address,
    pub public_key: BytesN<65>,
}

#[contractevent(topics = ["THRESHOLD_UPDATED"], data_format = "single-value")]
pub struct ThresholdUpdated {
    #[topic]
    pub account: Address,
    pub threshold: u32,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum AccError {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    InvalidSignature = 4,
    ThresholdNotMet = 5,
    SignerNotFound = 6,
    SignerAlreadyExists = 7,
    InvalidThreshold = 8,
    CannotRemoveBelowThreshold = 9,
    DuplicateSigner = 10,
    InvalidSignerWeight = 11,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[contracttype]
#[repr(u32)]
pub enum SignerRole {
    Owner = 0,
    Backup = 1,
    Guardian = 2,
}

#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct Signer {
    pub public_key: BytesN<65>,
    pub weight: u32,
    pub role: SignerRole,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct PasskeySignature {
    pub public_key: BytesN<65>,
    pub signature: BytesN<64>,
}

#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct PasskeyAuth {
    pub signatures: Vec<PasskeySignature>,
}

#[derive(Clone)]
#[contracttype]
pub enum DataKey {
    Initialized,
    Threshold,
    Signer(BytesN<65>),
    SignerKeys,
}

#[contract]
pub struct SmartAccountContract;

#[contractimpl]
impl CustomAccountInterface for SmartAccountContract {
    type Signature = PasskeyAuth;
    type Error = AccError;

    fn __check_auth(
        env: Env,
        signature_payload: Hash<32>,
        signatures: PasskeyAuth,
        _auth_contexts: Vec<Context>,
    ) -> Result<(), AccError> {
        if !env.storage().instance().has(&DataKey::Initialized) {
            return Err(AccError::NotInitialized);
        }

        let threshold: u32 = env
            .storage()
            .instance()
            .get(&DataKey::Threshold)
            .unwrap_or(1);

        if signatures.signatures.is_empty() {
            return Err(AccError::ThresholdNotMet);
        }

        let mut accumulated_weight: u32 = 0;
        let mut seen_keys = Vec::<BytesN<65>>::new(&env);

        for sig in signatures.signatures.iter() {
            // Prevent duplicate signatures inflating weight
            for seen in seen_keys.iter() {
                if seen == sig.public_key {
                    return Err(AccError::DuplicateSigner);
                }
            }
            seen_keys.push_back(sig.public_key.clone());

            // Validate public key format: 65 bytes uncompressed SEC-1 begins with 0x04 and has non-zero coordinates
            let pk_arr = sig.public_key.to_array();
            if pk_arr[0] != 0x04 || pk_arr[1..].iter().all(|&b| b == 0) {
                return Err(AccError::InvalidSignature);
            }

            // Validate raw signature: neither r nor s can be all zeros
            let sig_arr = sig.signature.to_array();
            let r_is_zero = sig_arr[..32].iter().all(|&b| b == 0);
            let s_is_zero = sig_arr[32..].iter().all(|&b| b == 0);
            if r_is_zero || s_is_zero {
                return Err(AccError::InvalidSignature);
            }

            // Look up registered signer
            let signer: Signer = match env
                .storage()
                .instance()
                .get(&DataKey::Signer(sig.public_key.clone()))
            {
                Some(s) => s,
                None => return Err(AccError::InvalidSignature),
            };

            // Cryptographic secp256r1 P-256 verification
            env.crypto()
                .secp256r1_verify(&sig.public_key, &signature_payload, &sig.signature);

            accumulated_weight = accumulated_weight.saturating_add(signer.weight);
        }

        if accumulated_weight < threshold {
            return Err(AccError::ThresholdNotMet);
        }

        Ok(())
    }
}

#[contractimpl]
impl SmartAccountContract {
    /// Install the primary owner passkey at deployment (atomic, so no one can
    /// claim a fresh account by installing their own key - audit M3). There is
    /// no `initialize` entrypoint.
    pub fn __constructor(env: Env, admin_public_key: BytesN<65>, threshold: u32, name: String) {
        if threshold == 0 {
            soroban_sdk::panic_with_error!(&env, AccError::InvalidThreshold);
        }

        let pk_arr = admin_public_key.to_array();
        if pk_arr[0] != 0x04 || pk_arr[1..].iter().all(|&b| b == 0) {
            soroban_sdk::panic_with_error!(&env, AccError::InvalidSignature);
        }

        // The primary owner signer is given weight equal to threshold to be immediately operational
        let initial_weight = threshold;
        let owner = Signer {
            public_key: admin_public_key.clone(),
            weight: initial_weight,
            role: SignerRole::Owner,
            name,
        };

        env.storage().instance().set(&DataKey::Threshold, &threshold);
        env.storage()
            .instance()
            .set(&DataKey::Signer(admin_public_key.clone()), &owner);

        let mut keys = Vec::new(&env);
        keys.push_back(admin_public_key.clone());
        env.storage().instance().set(&DataKey::SignerKeys, &keys);

        env.storage().instance().set(&DataKey::Initialized, &true);

        AccountInitialized {
            account: env.current_contract_address(),
            public_key: admin_public_key,
        }
        .publish(&env);

    }

    /// Add a new signer (requires self-authorization by the smart account)
    pub fn add_signer(env: Env, signer: Signer) -> Result<(), AccError> {
        env.current_contract_address().require_auth();

        if !env.storage().instance().has(&DataKey::Initialized) {
            return Err(AccError::NotInitialized);
        }

        if signer.weight == 0 {
            return Err(AccError::InvalidSignerWeight);
        }

        let pk_arr = signer.public_key.to_array();
        if pk_arr[0] != 0x04 || pk_arr[1..].iter().all(|&b| b == 0) {
            return Err(AccError::InvalidSignature);
        }

        if env
            .storage()
            .instance()
            .has(&DataKey::Signer(signer.public_key.clone()))
        {
            return Err(AccError::SignerAlreadyExists);
        }

        env.storage()
            .instance()
            .set(&DataKey::Signer(signer.public_key.clone()), &signer);

        let mut keys: Vec<BytesN<65>> = env
            .storage()
            .instance()
            .get(&DataKey::SignerKeys)
            .unwrap_or_else(|| Vec::new(&env));
        keys.push_back(signer.public_key.clone());
        env.storage().instance().set(&DataKey::SignerKeys, &keys);

        SignerAdded {
            account: env.current_contract_address(),
            public_key: signer.public_key,
        }
        .publish(&env);

        Ok(())
    }

    /// Update an existing signer's role, weight, or name (requires self-authorization; anti-bricking safety enforced)
    pub fn update_signer(env: Env, signer: Signer) -> Result<(), AccError> {
        env.current_contract_address().require_auth();

        if !env.storage().instance().has(&DataKey::Initialized) {
            return Err(AccError::NotInitialized);
        }

        if signer.weight == 0 {
            return Err(AccError::InvalidSignerWeight);
        }

        let pk_arr = signer.public_key.to_array();
        if pk_arr[0] != 0x04 || pk_arr[1..].iter().all(|&b| b == 0) {
            return Err(AccError::InvalidSignature);
        }

        let existing: Signer = match env
            .storage()
            .instance()
            .get(&DataKey::Signer(signer.public_key.clone()))
        {
            Some(s) => s,
            None => return Err(AccError::SignerNotFound),
        };

        let threshold: u32 = env
            .storage()
            .instance()
            .get(&DataKey::Threshold)
            .unwrap_or(1);

        let keys: Vec<BytesN<65>> = env
            .storage()
            .instance()
            .get(&DataKey::SignerKeys)
            .unwrap_or_else(|| Vec::new(&env));

        let mut total_weight: u32 = 0;
        for k in keys.iter() {
            if let Some(s) = env
                .storage()
                .instance()
                .get::<_, Signer>(&DataKey::Signer(k))
            {
                total_weight = total_weight.saturating_add(s.weight);
            }
        }

        let new_total_weight = total_weight
            .saturating_sub(existing.weight)
            .saturating_add(signer.weight);
        if new_total_weight < threshold {
            return Err(AccError::CannotRemoveBelowThreshold);
        }

        env.storage()
            .instance()
            .set(&DataKey::Signer(signer.public_key.clone()), &signer);

        SignerUpdated {
            account: env.current_contract_address(),
            public_key: signer.public_key,
        }
        .publish(&env);

        Ok(())
    }

    /// Remove a signer (requires self-authorization; anti-bricking safety enforced)
    pub fn remove_signer(env: Env, public_key: BytesN<65>) -> Result<(), AccError> {
        env.current_contract_address().require_auth();

        if !env.storage().instance().has(&DataKey::Initialized) {
            return Err(AccError::NotInitialized);
        }

        let signer: Signer = match env
            .storage()
            .instance()
            .get(&DataKey::Signer(public_key.clone()))
        {
            Some(s) => s,
            None => return Err(AccError::SignerNotFound),
        };

        let threshold: u32 = env
            .storage()
            .instance()
            .get(&DataKey::Threshold)
            .unwrap_or(1);

        let keys: Vec<BytesN<65>> = env
            .storage()
            .instance()
            .get(&DataKey::SignerKeys)
            .unwrap_or_else(|| Vec::new(&env));

        let mut total_weight: u32 = 0;
        for k in keys.iter() {
            if let Some(s) = env
                .storage()
                .instance()
                .get::<_, Signer>(&DataKey::Signer(k))
            {
                total_weight = total_weight.saturating_add(s.weight);
            }
        }

        // Anti-bricking check: remaining signer weight cannot fall below threshold
        let remaining_weight = total_weight.saturating_sub(signer.weight);
        if remaining_weight < threshold {
            return Err(AccError::CannotRemoveBelowThreshold);
        }

        // Storage rent hygiene: completely remove zeroed / deprecated storage entry
        env.storage()
            .instance()
            .remove(&DataKey::Signer(public_key.clone()));

        let mut new_keys = Vec::new(&env);
        for k in keys.iter() {
            if k != public_key {
                new_keys.push_back(k);
            }
        }
        env.storage().instance().set(&DataKey::SignerKeys, &new_keys);

        SignerRemoved {
            account: env.current_contract_address(),
            public_key,
        }
        .publish(&env);

        Ok(())
    }

    /// Update authentication threshold (requires self-authorization; anti-bricking safety enforced)
    pub fn update_threshold(env: Env, new_threshold: u32) -> Result<(), AccError> {
        env.current_contract_address().require_auth();

        if !env.storage().instance().has(&DataKey::Initialized) {
            return Err(AccError::NotInitialized);
        }

        if new_threshold == 0 {
            return Err(AccError::InvalidThreshold);
        }

        let keys: Vec<BytesN<65>> = env
            .storage()
            .instance()
            .get(&DataKey::SignerKeys)
            .unwrap_or_else(|| Vec::new(&env));

        let mut total_weight: u32 = 0;
        for k in keys.iter() {
            if let Some(s) = env
                .storage()
                .instance()
                .get::<_, Signer>(&DataKey::Signer(k))
            {
                total_weight = total_weight.saturating_add(s.weight);
            }
        }

        if total_weight < new_threshold {
            return Err(AccError::InvalidThreshold);
        }

        env.storage()
            .instance()
            .set(&DataKey::Threshold, &new_threshold);

        ThresholdUpdated {
            account: env.current_contract_address(),
            threshold: new_threshold,
        }
        .publish(&env);

        Ok(())
    }

    /// Query the current threshold
    pub fn get_threshold(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::Threshold)
            .unwrap_or(0)
    }

    /// Query an individual signer by public key
    pub fn get_signer(env: Env, public_key: BytesN<65>) -> Option<Signer> {
        env.storage().instance().get(&DataKey::Signer(public_key))
    }

    /// Query all registered signers
    pub fn get_signers(env: Env) -> Vec<Signer> {
        let mut signers = Vec::new(&env);
        if let Some(keys) = env
            .storage()
            .instance()
            .get::<_, Vec<BytesN<65>>>(&DataKey::SignerKeys)
        {
            for k in keys.iter() {
                if let Some(s) = env
                    .storage()
                    .instance()
                    .get::<_, Signer>(&DataKey::Signer(k))
                {
                    signers.push_back(s);
                }
            }
        }
        signers
    }
}

#[cfg(test)]
mod test;

