#![cfg(test)]

use super::*;
use p256::ecdsa::{signature::hazmat::PrehashSigner, Signature, SigningKey, VerifyingKey};
use soroban_sdk::{
    auth::Context,
    vec, Bytes, BytesN, Env, IntoVal, String, Vec,
};

struct KeyPair {
    signing_key: SigningKey,
    public_key_bytes: BytesN<65>,
}

fn generate_keypair(env: &Env) -> KeyPair {
    let signing_key = SigningKey::random(&mut rand_core::OsRng);
    let verifying_key = VerifyingKey::from(&signing_key);
    let point = verifying_key.to_encoded_point(false);
    let pk_bytes = BytesN::<65>::from_array(env, point.as_bytes().try_into().unwrap());
    KeyPair {
        signing_key,
        public_key_bytes: pk_bytes,
    }
}

fn sign_payload(env: &Env, keypair: &KeyPair, payload_hash: &[u8; 32]) -> BytesN<64> {
    let sig: Signature = keypair.signing_key.sign_prehash(payload_hash).unwrap();
    let normalized = sig.normalize_s().unwrap_or(sig);
    let bytes = normalized.to_bytes();
    let mut sig_arr = [0u8; 64];
    sig_arr.copy_from_slice(&bytes);
    BytesN::<64>::from_array(env, &sig_arr)
}

fn make_payload_hash(env: &Env, seed: &[u8]) -> (Hash<32>, BytesN<32>, [u8; 32]) {
    let mut msg_bytes = Bytes::new(env);
    msg_bytes.append(&Bytes::from_slice(env, seed));
    let hash_obj = env.crypto().sha256(&msg_bytes);
    (hash_obj.clone(), hash_obj.to_bytes(), hash_obj.to_array())
}

#[test]
fn test_t47_passkey_account_init() {
    let env = Env::default();
    let contract_id = env.register(SmartAccountContract, ());
    let client = SmartAccountContractClient::new(&env, &contract_id);

    let admin_kp = generate_keypair(&env);

    // Initialize with threshold 1
    let init_res = client.try_initialize(
        &admin_kp.public_key_bytes,
        &1,
        &String::from_str(&env, "Treasury Admin Passkey"),
    );
    assert!(init_res.is_ok());

    // Verify threshold
    assert_eq!(client.get_threshold(), 1);

    // Verify registered signer
    let signer = client.get_signer(&admin_kp.public_key_bytes).unwrap();
    assert_eq!(signer.public_key, admin_kp.public_key_bytes);
    assert_eq!(signer.weight, 1);
    assert_eq!(signer.role, SignerRole::Owner);
    assert_eq!(
        signer.name,
        String::from_str(&env, "Treasury Admin Passkey")
    );

    let all_signers = client.get_signers();
    assert_eq!(all_signers.len(), 1);

    // Idempotency check: Cannot re-initialize
    let second_kp = generate_keypair(&env);
    let reinit_res = client.try_initialize(
        &second_kp.public_key_bytes,
        &1,
        &String::from_str(&env, "Attacker Key"),
    );
    assert_eq!(reinit_res, Err(Ok(AccError::AlreadyInitialized)));

    // Zero threshold rejected on fresh deployment
    let fresh_id = env.register(SmartAccountContract, ());
    let fresh_client = SmartAccountContractClient::new(&env, &fresh_id);
    let zero_thresh_res = fresh_client.try_initialize(
        &second_kp.public_key_bytes,
        &0,
        &String::from_str(&env, "Zero Thresh"),
    );
    assert_eq!(zero_thresh_res, Err(Ok(AccError::InvalidThreshold)));
}

#[test]
fn test_t48_secp256r1_valid_auth() {
    let env = Env::default();
    let contract_id = env.register(SmartAccountContract, ());
    let client = SmartAccountContractClient::new(&env, &contract_id);

    let admin_kp = generate_keypair(&env);
    client.initialize(
        &admin_kp.public_key_bytes,
        &1,
        &String::from_str(&env, "Alice iPhone Passkey"),
    );

    let (_hash_obj, payload_hash, payload_bytes) = make_payload_hash(&env, b"tx_authorization_payload_t48");
    let signature_bytes = sign_payload(&env, &admin_kp, &payload_bytes);

    let auth = PasskeyAuth {
        signatures: vec![
            &env,
            PasskeySignature {
                public_key: admin_kp.public_key_bytes.clone(),
                signature: signature_bytes,
            },
        ],
    };

    let empty_contexts: Vec<Context> = vec![&env];

    let check_res = env.try_invoke_contract_check_auth::<AccError>(
        &contract_id,
        &payload_hash,
        auth.into_val(&env),
        &empty_contexts,
    );
    assert_eq!(check_res, Ok(()));
}

#[test]
fn test_t49_invalid_signature_rejection() {
    let env = Env::default();
    let contract_id = env.register(SmartAccountContract, ());
    let client = SmartAccountContractClient::new(&env, &contract_id);

    let admin_kp = generate_keypair(&env);
    client.initialize(
        &admin_kp.public_key_bytes,
        &1,
        &String::from_str(&env, "Alice Key"),
    );

    let (_hash_obj, payload_hash, payload_bytes) = make_payload_hash(&env, b"tx_authorization_payload_t49");
    let signature_bytes = sign_payload(&env, &admin_kp, &payload_bytes);

    let empty_contexts: Vec<Context> = vec![&env];

    // Case 1: Unregistered / wrong public key submitted
    let stranger_kp = generate_keypair(&env);
    let stranger_auth = PasskeyAuth {
        signatures: vec![
            &env,
            PasskeySignature {
                public_key: stranger_kp.public_key_bytes.clone(),
                signature: signature_bytes.clone(),
            },
        ],
    };
    let stranger_res = env.try_invoke_contract_check_auth::<AccError>(
        &contract_id,
        &payload_hash,
        stranger_auth.into_val(&env),
        &empty_contexts,
    );
    assert_eq!(stranger_res, Err(Ok(AccError::InvalidSignature)));

    // Case 2: All-zero signature submitted
    let zero_sig = BytesN::<64>::from_array(&env, &[0u8; 64]);
    let zero_auth = PasskeyAuth {
        signatures: vec![
            &env,
            PasskeySignature {
                public_key: admin_kp.public_key_bytes.clone(),
                signature: zero_sig,
            },
        ],
    };
    let zero_res = env.try_invoke_contract_check_auth::<AccError>(
        &contract_id,
        &payload_hash,
        zero_auth.into_val(&env),
        &empty_contexts,
    );
    assert_eq!(zero_res, Err(Ok(AccError::InvalidSignature)));

    // Case 3: Altered payload hash (signed for payload A, submitted with payload B)
    let (_alt_hash, altered_payload_hash, _) = make_payload_hash(&env, b"altered_payload_seed");
    let valid_auth = PasskeyAuth {
        signatures: vec![
            &env,
            PasskeySignature {
                public_key: admin_kp.public_key_bytes.clone(),
                signature: signature_bytes.clone(),
            },
        ],
    };
    let altered_res = env.try_invoke_contract_check_auth::<AccError>(
        &contract_id,
        &altered_payload_hash,
        valid_auth.into_val(&env),
        &empty_contexts,
    );
    assert!(altered_res.is_err());

    // Case 4: Duplicate signer in same auth
    let duplicate_auth = PasskeyAuth {
        signatures: vec![
            &env,
            PasskeySignature {
                public_key: admin_kp.public_key_bytes.clone(),
                signature: signature_bytes.clone(),
            },
            PasskeySignature {
                public_key: admin_kp.public_key_bytes.clone(),
                signature: signature_bytes.clone(),
            },
        ],
    };
    let dup_res = env.try_invoke_contract_check_auth::<AccError>(
        &contract_id,
        &payload_hash,
        duplicate_auth.into_val(&env),
        &empty_contexts,
    );
    assert_eq!(dup_res, Err(Ok(AccError::DuplicateSigner)));

    // Case 5: Empty signatures vector
    let empty_auth = PasskeyAuth {
        signatures: vec![&env],
    };
    let empty_res = env.try_invoke_contract_check_auth::<AccError>(
        &contract_id,
        &payload_hash,
        empty_auth.into_val(&env),
        &empty_contexts,
    );
    assert_eq!(empty_res, Err(Ok(AccError::ThresholdNotMet)));
}

#[test]
fn test_t50_multi_signer_threshold() {
    let env = Env::default();
    let contract_id = env.register(SmartAccountContract, ());
    let client = SmartAccountContractClient::new(&env, &contract_id);

    let kp_owner = generate_keypair(&env);
    let kp_backup = generate_keypair(&env);
    let kp_guardian = generate_keypair(&env);

    client.initialize(
        &kp_owner.public_key_bytes,
        &1,
        &String::from_str(&env, "Owner Passkey"),
    );

    // Add backup and guardian keys using self-authorized call
    env.mock_all_auths();
    client.add_signer(&Signer {
        public_key: kp_backup.public_key_bytes.clone(),
        weight: 1,
        role: SignerRole::Backup,
        name: String::from_str(&env, "Backup Key"),
    });
    client.add_signer(&Signer {
        public_key: kp_guardian.public_key_bytes.clone(),
        weight: 1,
        role: SignerRole::Guardian,
        name: String::from_str(&env, "Guardian Key"),
    });

    // Elevate threshold to 2 (2-of-3 setup)
    client.update_threshold(&2);
    assert_eq!(client.get_threshold(), 2);
    assert_eq!(client.get_signers().len(), 3);

    let (_hash_obj, payload_hash, payload_bytes) = make_payload_hash(&env, b"tx_authorization_payload_t50");
    let empty_contexts: Vec<Context> = vec![&env];

    let sig_owner = sign_payload(&env, &kp_owner, &payload_bytes);
    let sig_backup = sign_payload(&env, &kp_backup, &payload_bytes);
    let sig_guardian = sign_payload(&env, &kp_guardian, &payload_bytes);

    // 1-of-3 submitted: fails threshold check (weight 1 < 2)
    let auth_1_sig = PasskeyAuth {
        signatures: vec![
            &env,
            PasskeySignature {
                public_key: kp_owner.public_key_bytes.clone(),
                signature: sig_owner.clone(),
            },
        ],
    };
    let res_1 = env.try_invoke_contract_check_auth::<AccError>(
        &contract_id,
        &payload_hash,
        auth_1_sig.into_val(&env),
        &empty_contexts,
    );
    assert_eq!(res_1, Err(Ok(AccError::ThresholdNotMet)));

    // 2-of-3 (Owner + Backup): succeeds (weight 2 >= 2)
    let auth_owner_backup = PasskeyAuth {
        signatures: vec![
            &env,
            PasskeySignature {
                public_key: kp_owner.public_key_bytes.clone(),
                signature: sig_owner.clone(),
            },
            PasskeySignature {
                public_key: kp_backup.public_key_bytes.clone(),
                signature: sig_backup.clone(),
            },
        ],
    };
    let res_2 = env.try_invoke_contract_check_auth::<AccError>(
        &contract_id,
        &payload_hash,
        auth_owner_backup.into_val(&env),
        &empty_contexts,
    );
    assert_eq!(res_2, Ok(()));

    // 2-of-3 (Backup + Guardian): succeeds without primary owner!
    let auth_backup_guardian = PasskeyAuth {
        signatures: vec![
            &env,
            PasskeySignature {
                public_key: kp_backup.public_key_bytes.clone(),
                signature: sig_backup.clone(),
            },
            PasskeySignature {
                public_key: kp_guardian.public_key_bytes.clone(),
                signature: sig_guardian.clone(),
            },
        ],
    };
    let res_social_recovery = env.try_invoke_contract_check_auth::<AccError>(
        &contract_id,
        &payload_hash,
        auth_backup_guardian.into_val(&env),
        &empty_contexts,
    );
    assert_eq!(res_social_recovery, Ok(()));

    // 3-of-3: succeeds
    let auth_all_3 = PasskeyAuth {
        signatures: vec![
            &env,
            PasskeySignature {
                public_key: kp_owner.public_key_bytes.clone(),
                signature: sig_owner,
            },
            PasskeySignature {
                public_key: kp_backup.public_key_bytes.clone(),
                signature: sig_backup,
            },
            PasskeySignature {
                public_key: kp_guardian.public_key_bytes.clone(),
                signature: sig_guardian,
            },
        ],
    };
    let res_all = env.try_invoke_contract_check_auth::<AccError>(
        &contract_id,
        &payload_hash,
        auth_all_3.into_val(&env),
        &empty_contexts,
    );
    assert_eq!(res_all, Ok(()));
}

#[test]
fn test_t51_signer_rotation_and_safety() {
    let env = Env::default();
    let contract_id = env.register(SmartAccountContract, ());
    let client = SmartAccountContractClient::new(&env, &contract_id);

    let kp_old = generate_keypair(&env);
    let kp_new = generate_keypair(&env);

    client.initialize(
        &kp_old.public_key_bytes,
        &1,
        &String::from_str(&env, "Old Passkey"),
    );

    env.mock_all_auths();

    // Anti-bricking check 1: Cannot remove the only signer when threshold is 1
    let remove_sole_res = client.try_remove_signer(&kp_old.public_key_bytes);
    assert_eq!(
        remove_sole_res,
        Err(Ok(AccError::CannotRemoveBelowThreshold))
    );

    // Rotate: Add new passkey first
    client.add_signer(&Signer {
        public_key: kp_new.public_key_bytes.clone(),
        weight: 1,
        role: SignerRole::Backup,
        name: String::from_str(&env, "New Replacement Passkey"),
    });

    // Now removing old key succeeds (remaining weight 1 >= threshold 1)
    let remove_old_res = client.try_remove_signer(&kp_old.public_key_bytes);
    assert!(remove_old_res.is_ok());

    // Storage hygiene check: old key is completely wiped
    assert!(client.get_signer(&kp_old.public_key_bytes).is_none());
    assert_eq!(client.get_signers().len(), 1);

    // Anti-bricking check 2: Cannot increase threshold beyond total signer weight
    let invalid_threshold_res = client.try_update_threshold(&2);
    assert_eq!(
        invalid_threshold_res,
        Err(Ok(AccError::InvalidThreshold))
    );
    let zero_threshold_res = client.try_update_threshold(&0);
    assert_eq!(zero_threshold_res, Err(Ok(AccError::InvalidThreshold)));

    // Duplicate signer addition rejected
    let dup_add_res = client.try_add_signer(&Signer {
        public_key: kp_new.public_key_bytes.clone(),
        weight: 1,
        role: SignerRole::Owner,
        name: String::from_str(&env, "Duplicate Key"),
    });
    assert_eq!(dup_add_res, Err(Ok(AccError::SignerAlreadyExists)));

    // Zero weight signer rejected
    let kp_zero = generate_keypair(&env);
    let zero_weight_res = client.try_add_signer(&Signer {
        public_key: kp_zero.public_key_bytes,
        weight: 0,
        role: SignerRole::Owner,
        name: String::from_str(&env, "Zero Weight"),
    });
    assert_eq!(zero_weight_res, Err(Ok(AccError::InvalidSignerWeight)));
}
