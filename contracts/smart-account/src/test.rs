#![cfg(test)]

extern crate std;

use super::*;
use p256::ecdsa::{signature::hazmat::PrehashSigner, Signature, SigningKey, VerifyingKey};
use soroban_sdk::{
    auth::Context,
    testutils::{Address as _, Ledger as _},
    vec, Address, Bytes, BytesN, Env, IntoVal, String, Symbol, Vec,
};

/// SDK 28 returns events as XDR (`ContractEvents`). Convert them back to the
/// `(contract, topics, data)` tuples these assertions use - the same mapping
/// the SDK applies in its own `PartialEq` for `ContractEvents` (W4.1).
fn all_events(env: &soroban_sdk::Env) -> soroban_sdk::Vec<(soroban_sdk::Address, soroban_sdk::Vec<soroban_sdk::Val>, soroban_sdk::Val)> {
    use soroban_sdk::testutils::Events as _;
    use soroban_sdk::{xdr, TryFromVal};
    let mut out = soroban_sdk::Vec::new(env);
    for e in env.events().all().events() {
        let Some(id) = e.contract_id.clone() else { continue };
        let addr = soroban_sdk::Address::try_from_val(env, &xdr::ScAddress::Contract(id)).unwrap();
        let xdr::ContractEventBody::V0(body) = &e.body;
        let mut topics = soroban_sdk::Vec::new(env);
        for t in body.topics.iter() {
            topics.push_back(soroban_sdk::Val::try_from_val(env, t).unwrap());
        }
        out.push_back((addr, topics, soroban_sdk::Val::try_from_val(env, &body.data).unwrap()));
    }
    out
}


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
    let admin_kp = generate_keypair(&env);

    // Audit M3: the owner passkey is installed by the constructor, atomically
    // with deployment, so there is no window for anyone else to claim it.
    let contract_id = env.register(
        SmartAccountContract,
        (admin_kp.public_key_bytes.clone(), 1u32, String::from_str(&env, "Treasury Admin Passkey")),
    );
    let client = SmartAccountContractClient::new(&env, &contract_id);

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

    // There is no `initialize` entrypoint to install an attacker key.
    let second_kp = generate_keypair(&env);
    let r = env.try_invoke_contract::<(), soroban_sdk::Error>(
        &contract_id,
        &Symbol::new(&env, "initialize"),
        (second_kp.public_key_bytes.clone(), 1u32, String::from_str(&env, "Attacker Key")).into_val(&env),
    );
    assert!(!matches!(r, Ok(Ok(()))));
    assert_eq!(client.get_signers().len(), 1);

    // Zero threshold makes the deployment itself fail.
    let env2 = Env::default();
    let key = second_kp.public_key_bytes.to_array();
    let deployed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        env2.register(SmartAccountContract, (BytesN::from_array(&env2, &key), 0u32, String::from_str(&env2, "Zero Thresh")))
    }));
    assert!(deployed.is_err());
}

#[test]
fn test_t48_secp256r1_valid_auth() {
    let env = Env::default();
    let admin_kp = generate_keypair(&env);
    let contract_id = env.register(SmartAccountContract, (admin_kp.public_key_bytes.clone(), 1u32, String::from_str(&env, "Alice iPhone Passkey")));

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

    let admin_kp = generate_keypair(&env);
    let contract_id = env.register(SmartAccountContract, (admin_kp.public_key_bytes.clone(), 1u32, String::from_str(&env, "Alice Key")));

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

    // Case 6: Public key with invalid SEC-1 prefix (e.g. 0x02 compressed instead of 0x04 uncompressed)
    let mut bad_prefix_bytes = [0u8; 65];
    bad_prefix_bytes[0] = 0x02;
    bad_prefix_bytes[1] = 0x01;
    let bad_pk = BytesN::<65>::from_array(&env, &bad_prefix_bytes);
    let bad_prefix_auth = PasskeyAuth {
        signatures: vec![
            &env,
            PasskeySignature {
                public_key: bad_pk,
                signature: signature_bytes.clone(),
            },
        ],
    };
    let bad_prefix_res = env.try_invoke_contract_check_auth::<AccError>(
        &contract_id,
        &payload_hash,
        bad_prefix_auth.into_val(&env),
        &empty_contexts,
    );
    assert_eq!(bad_prefix_res, Err(Ok(AccError::InvalidSignature)));

    // Case 7: Public key with all-zero coordinate point [0x04, 0, ..., 0]
    let mut zero_pt_bytes = [0u8; 65];
    zero_pt_bytes[0] = 0x04;
    let zero_pt_pk = BytesN::<65>::from_array(&env, &zero_pt_bytes);
    let zero_pt_auth = PasskeyAuth {
        signatures: vec![
            &env,
            PasskeySignature {
                public_key: zero_pt_pk,
                signature: signature_bytes.clone(),
            },
        ],
    };
    let zero_pt_res = env.try_invoke_contract_check_auth::<AccError>(
        &contract_id,
        &payload_hash,
        zero_pt_auth.into_val(&env),
        &empty_contexts,
    );
    assert_eq!(zero_pt_res, Err(Ok(AccError::InvalidSignature)));

    // Case 8: Signature where r = 0 or s = 0 (scalar component zero)
    let mut zero_r_bytes = [1u8; 64];
    for b in zero_r_bytes[..32].iter_mut() {
        *b = 0;
    }
    let zero_r_sig = BytesN::<64>::from_array(&env, &zero_r_bytes);
    let zero_r_auth = PasskeyAuth {
        signatures: vec![
            &env,
            PasskeySignature {
                public_key: admin_kp.public_key_bytes.clone(),
                signature: zero_r_sig,
            },
        ],
    };
    let zero_r_res = env.try_invoke_contract_check_auth::<AccError>(
        &contract_id,
        &payload_hash,
        zero_r_auth.into_val(&env),
        &empty_contexts,
    );
    assert_eq!(zero_r_res, Err(Ok(AccError::InvalidSignature)));

    // (Audit M3: an uninitialized account can no longer exist; the owner key is
    // installed by the constructor, atomically with deployment.)
}

#[test]
fn test_t50_multi_signer_threshold() {
    let env = Env::default();

    let kp_owner = generate_keypair(&env);
    let kp_backup = generate_keypair(&env);
    let kp_guardian = generate_keypair(&env);

    let contract_id = env.register(SmartAccountContract, (kp_owner.public_key_bytes.clone(), 1u32, String::from_str(&env, "Owner Passkey")));
    let client = SmartAccountContractClient::new(&env, &contract_id);

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

    let kp_old = generate_keypair(&env);
    let kp_new = generate_keypair(&env);

    let contract_id = env.register(SmartAccountContract, (kp_old.public_key_bytes.clone(), 1u32, String::from_str(&env, "Old Passkey")));
    let client = SmartAccountContractClient::new(&env, &contract_id);

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
        public_key: kp_zero.public_key_bytes.clone(),
        weight: 0,
        role: SignerRole::Owner,
        name: String::from_str(&env, "Zero Weight"),
    });
    assert_eq!(zero_weight_res, Err(Ok(AccError::InvalidSignerWeight)));

    // Non-existent signer removal rejected with SignerNotFound
    let non_existent_kp = generate_keypair(&env);
    let rem_non_existent = client.try_remove_signer(&non_existent_kp.public_key_bytes);
    assert_eq!(rem_non_existent, Err(Ok(AccError::SignerNotFound)));

    // Signer addition with invalid public key format rejected
    let mut bad_pk_bytes = [0u8; 65];
    bad_pk_bytes[0] = 0x02; // Compressed prefix
    bad_pk_bytes[1] = 0x05;
    let bad_pk = BytesN::<65>::from_array(&env, &bad_pk_bytes);
    let bad_pk_res = client.try_add_signer(&Signer {
        public_key: bad_pk,
        weight: 1,
        role: SignerRole::Owner,
        name: String::from_str(&env, "Bad PK"),
    });
    assert_eq!(bad_pk_res, Err(Ok(AccError::InvalidSignature)));

    // Update signer: successfully adjust role, name, and weight
    let updated_signer = Signer {
        public_key: kp_new.public_key_bytes.clone(),
        weight: 2,
        role: SignerRole::Owner,
        name: String::from_str(&env, "Promoted Owner Key"),
    };
    let update_res = client.try_update_signer(&updated_signer);
    assert!(update_res.is_ok());

    let fetched = client.get_signer(&kp_new.public_key_bytes).unwrap();
    assert_eq!(fetched.weight, 2);
    assert_eq!(fetched.role, SignerRole::Owner);
    assert_eq!(fetched.name, String::from_str(&env, "Promoted Owner Key"));

    // Now threshold can be safely increased to 2
    client.update_threshold(&2);
    assert_eq!(client.get_threshold(), 2);

    // Update signer: cannot reduce weight below threshold (anti-bricking)
    let underweight_signer = Signer {
        public_key: kp_new.public_key_bytes.clone(),
        weight: 1, // Below threshold 2
        role: SignerRole::Owner,
        name: String::from_str(&env, "Underweight"),
    };
    let underweight_res = client.try_update_signer(&underweight_signer);
    assert_eq!(
        underweight_res,
        Err(Ok(AccError::CannotRemoveBelowThreshold))
    );

    // Update signer: non-existent signer fails with SignerNotFound
    let non_existent_update = Signer {
        public_key: non_existent_kp.public_key_bytes.clone(),
        weight: 1,
        role: SignerRole::Guardian,
        name: String::from_str(&env, "Ghost"),
    };
    assert_eq!(
        client.try_update_signer(&non_existent_update),
        Err(Ok(AccError::SignerNotFound))
    );

    // (Audit M3: an uninitialized account can no longer exist.)
}

#[test]
fn test_smart_account_unauthorized_self_auth_enforcement() {
    let env = Env::default();

    let admin_kp = generate_keypair(&env);
    let contract_id = env.register(SmartAccountContract, (admin_kp.public_key_bytes.clone(), 1u32, String::from_str(&env, "Owner Passkey")));
    let client = SmartAccountContractClient::new(&env, &contract_id);

    let new_kp = generate_keypair(&env);
    let new_signer = Signer {
        public_key: new_kp.public_key_bytes.clone(),
        weight: 1,
        role: SignerRole::Backup,
        name: String::from_str(&env, "Backup Key"),
    };

    // Attempting management calls without self-authorization fails
    assert!(client.try_add_signer(&new_signer).is_err());
    assert!(client.try_remove_signer(&admin_kp.public_key_bytes).is_err());
    assert!(client.try_update_threshold(&2).is_err());
    assert!(client.try_update_signer(&new_signer).is_err());
}

#[test]
fn test_t55_sponsored_passkey_vault_deposit() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000_000);
    env.ledger().set_sequence_number(100);

    let admin = Address::generate(&env);
    let pause_authority = Address::generate(&env);
    let treasury = Address::generate(&env);

    // 1. Deploy Numéraire (USDC)
    let token_admin = Address::generate(&env);
    let numeraire_sac = env.register_stellar_asset_contract_v2(token_admin);
    let numeraire_address = numeraire_sac.address();
    let numeraire_admin = soroban_sdk::token::StellarAssetClient::new(&env, &numeraire_address);
    let numeraire_client = soroban_sdk::token::Client::new(&env, &numeraire_address);

    // 2. Deploy Registry
    let registry_id = env.register(constella_registry::AssetRegistry, (admin.clone(),));
    let registry_client = constella_registry::AssetRegistryClient::new(&env, &registry_id);

    let mut usdc_sources = Vec::new(&env);
    usdc_sources.push_back(Symbol::new(&env, "USDC_SRC"));
    registry_client.add_asset(&constella_registry::AssetEntry {
        contract_address: numeraire_address.clone(),
        asset_id: Symbol::new(&env, "USDC"),
        issuer: Symbol::new(&env, "CIRCLE"),
        sleeve_type: constella_registry::SleeveType::Cash,
        liquidity_tier: 0,
        permissionless: true,
        oracle_sources: usdc_sources,
        eligible_jurisdictions: Vec::new(&env),
        blocked_jurisdictions: Vec::new(&env),
        investor_classes_required: Vec::new(&env),
        is_active: true,
        added_at: 1_000_000,
    });

    // 3. Deploy Policy

    // 4. Deploy Oracle
    let oracle_id = env.register(constella_nav_oracle::NavOracle, (admin.clone(), numeraire_address.clone(), 7u32, 300u32));
    let oracle_client = constella_nav_oracle::NavOracleClient::new(&env, &oracle_id);
    let policy_id = env.register(constella_policy::PolicyContract, (admin.clone(), oracle_id.clone()));

    let sleeve_sym = Symbol::new(&env, "VAULT");
    oracle_client.configure_sleeve(&sleeve_sym, &300, &2);
    let dummy_asset = Address::generate(&env);
    let s1 = constella_nav_oracle::OracleSource {
        source_id: Symbol::new(&env, "MEDIAN_SRC"),
        asset: dummy_asset.clone(),
        sleeve: sleeve_sym.clone(),
        price: 10_000_000,
        timestamp: 1_000_000,
        staleness_limit_seconds: 14400,
        min_liquidity_depth_usd: 100_000,
        current_liquidity_depth_usd: 200_000,
        is_active: true,
    };
    let s2 = constella_nav_oracle::OracleSource {
        source_id: Symbol::new(&env, "CONS_SRC"),
        asset: dummy_asset.clone(),
        sleeve: sleeve_sym.clone(),
        price: 9_900_000,
        timestamp: 1_000_000,
        staleness_limit_seconds: 14400,
        min_liquidity_depth_usd: 100_000,
        current_liquidity_depth_usd: 200_000,
        is_active: true,
    };
    let s3 = constella_nav_oracle::OracleSource {
        source_id: Symbol::new(&env, "HIGH_SRC"),
        asset: dummy_asset.clone(),
        sleeve: sleeve_sym.clone(),
        price: 10_100_000,
        timestamp: 1_000_000,
        staleness_limit_seconds: 14400,
        min_liquidity_depth_usd: 100_000,
        current_liquidity_depth_usd: 200_000,
        is_active: true,
    };
    oracle_client.register_source(&s1);
    oracle_client.register_source(&s2);
    oracle_client.register_source(&s3);

    // 5. Deploy Strategy Vault
    let vault_config = constella_vault_strategy::VaultConfig {
        admin: admin.clone(),
        pause_authority: pause_authority.clone(),
        numeraire: numeraire_address.clone(),
        policy_contract: policy_id.clone(),
        oracle_contract: oracle_id.clone(),
        registry_contract: registry_id.clone(),
        treasury: treasury.clone(),
        management_fee_bps: 100,
        tvl_cap: 1_000_000 * 10_000_000, // FIXTURE: ROADMAP 3.2 $1M beta cap
        per_user_cap: 100_000 * 10_000_000, // FIXTURE: design-partner ticket size (GTM)
    };
    let vault_id = env.register(
        constella_vault_strategy::StrategyVault,
        (
            vault_config,
            String::from_str(&env, "Constella Strategy Vault"),
            String::from_str(&env, "CONST"),
            7u32,
        ),
    );
    let vault_client = constella_vault_strategy::StrategyVaultClient::new(&env, &vault_id);

    // 6. Deploy Fee Sponsor Engine
    let sponsor_id = env.register(constella_fee_sponsor::FeeSponsorContract, (admin.clone(), 10u32, 50u32, 500_000u64));
    let sponsor_client = constella_fee_sponsor::FeeSponsorContractClient::new(&env, &sponsor_id);
    // Whitelist vault for zero-gas sponsorship
    sponsor_client.whitelist_contract(&vault_id);

    // 7. Deploy & Initialize User Passkey Smart Account
    let user_passkey = generate_keypair(&env);
    let user_account_id = env.register(SmartAccountContract, (user_passkey.public_key_bytes.clone(), 1u32, String::from_str(&env, "Bob iPhone Passkey")));

    // Mint USDC deposit funds to the user smart account (e.g. 1000 USDC)
    numeraire_admin.mint(&user_account_id, &1000_0000000);
    assert_eq!(numeraire_client.balance(&user_account_id), 1000_0000000);

    // Zero XLM user flow:
    // A) Sponsor verifies transaction eligibility before signing fee-bump
    let fee_bid_stroops = 10_000; // 0.001 XLM gas
    let sponsor_val = sponsor_client.try_validate_sponsorship(&user_account_id, &vault_id, &fee_bid_stroops);
    assert_eq!(sponsor_val, Ok(Ok(())));

    // B) User executes deposit of 500 USDC into Strategy Vault via smart account
    let shares_minted = vault_client.deposit(&500_0000000, &user_account_id, &user_account_id, &user_account_id);
    assert_eq!(shares_minted, 500_0000000);

    // C) Sponsor records the sponsored transaction
    sponsor_client.record_sponsored_op(&user_account_id, &vault_id, &fee_bid_stroops);

    // D) Verification of balances & zero-gas sponsorship accounting
    // Smart account holds 500 vault shares
    assert_eq!(vault_client.balance(&user_account_id), 500_0000000);
    // Smart account still has remaining 500 USDC
    assert_eq!(numeraire_client.balance(&user_account_id), 500_0000000);
    // User paid 0 XLM for gas; Fee Sponsor tracked the operation & budget
    let user_stats = sponsor_client.get_user_stats(&user_account_id);
    assert_eq!(user_stats.hourly_ops, 1);
    assert_eq!(user_stats.daily_ops, 1);
    let global_stats = sponsor_client.get_global_stats();
    assert_eq!(global_stats.total_ops_today, 1);
    assert_eq!(global_stats.total_fees_today_stroops, fee_bid_stroops);
}

// -------------------------------------------------------------------------
// Audit M3 (2026-11-15): a freshly deployed passkey account must not be
// claimable by whoever calls `initialize` first.
// -------------------------------------------------------------------------
#[test]
fn m3_fresh_account_cannot_be_claimed_by_a_stranger() {
    use soroban_sdk::IntoVal;
    let env = Env::default();
    let owner = generate_keypair(&env);
    let account = env.register(SmartAccountContract, (owner.public_key_bytes.clone(), 1u32, String::from_str(&env, "owner")));
    let mut attacker_key = [7u8; 65];
    attacker_key[0] = 0x04;
    let args = (BytesN::from_array(&env, &attacker_key), 1u32, String::from_str(&env, "attacker")).into_val(&env);
    let r = env.try_invoke_contract::<(), soroban_sdk::Error>(&account, &Symbol::new(&env, "initialize"), args);
    assert!(!matches!(r, Ok(Ok(()))), "a stranger must not be able to install their key");
    let signers = SmartAccountContractClient::new(&env, &account).get_signers();
    assert_eq!(signers.len(), 1);
    assert_eq!(signers.get(0).unwrap().public_key, owner.public_key_bytes);
}

// -------------------------------------------------------------------------
// Audit M2 (2026-11-15): custom events use SCREAMING_SNAKE_CASE (Rule 8).
// -------------------------------------------------------------------------
#[test]
fn m2_every_account_event_follows_rule_8() {
    use soroban_sdk::{FromVal, SymbolStr, TryFromVal};
    let env = Env::default();
    env.mock_all_auths();
    let owner = generate_keypair(&env);
    let id = env.register(SmartAccountContract, (owner.public_key_bytes.clone(), 1u32, String::from_str(&env, "owner")));
    let client = SmartAccountContractClient::new(&env, &id);
    let check = |env: &Env| -> usize {
        let mut n = 0;
        for (c, topics, _) in all_events(env).iter() {
            if c != id { continue; }
            let sym = Symbol::try_from_val(env, &topics.get(0).unwrap()).unwrap();
            let s = SymbolStr::from_val(env, &sym.to_symbol_val());
            let bytes: &[u8] = s.as_ref();
            assert!(bytes.iter().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || *b == b'_'),
                "event `{}` violates Rule 8", std::str::from_utf8(bytes).unwrap());
            n += 1;
        }
        n
    };
    let backup = generate_keypair(&env);
    let mut n = 0;
    client.add_signer(&Signer { public_key: backup.public_key_bytes.clone(), weight: 1, role: SignerRole::Backup, name: String::from_str(&env, "b") });
    n += check(&env);
    client.update_signer(&Signer { public_key: backup.public_key_bytes.clone(), weight: 2, role: SignerRole::Backup, name: String::from_str(&env, "b") });
    n += check(&env);
    client.update_threshold(&2);
    n += check(&env);
    client.update_threshold(&1);
    client.remove_signer(&backup.public_key_bytes);
    n += check(&env);
    assert!(n >= 4, "saw {n} events");
}

// -------------------------------------------------------------------------
// Audit W4.1b (2026-11-15): golden event encodings.
// -------------------------------------------------------------------------
/// Golden event recorder (audit W4.1b). Captures the exact XDR of every event
/// the contract under test emits during a scenario and compares it with the
/// committed `golden/events.txt`, so the `Events::publish` -> `#[contractevent]`
/// migration is proven byte-for-byte. Regenerate ONLY deliberately with
/// `UPDATE_GOLDEN=1 cargo test golden_events`.
struct GoldenEvents {
    contract: soroban_sdk::Address,
    lines: std::vec::Vec<std::string::String>,
    names: std::collections::BTreeSet<std::string::String>,
}

impl GoldenEvents {
    fn new(contract: &soroban_sdk::Address) -> Self {
        Self { contract: contract.clone(), lines: std::vec::Vec::new(), names: Default::default() }
    }

    fn capture(&mut self, env: &soroban_sdk::Env, step: &str) {
        use soroban_sdk::testutils::Events as _;
        use soroban_sdk::xdr::{ContractEventBody, Limits, ScVal, WriteXdr};
        for e in env.events().all().filter_by_contract(&self.contract).events() {
            let ContractEventBody::V0(body) = &e.body;
            let name = match body.topics.first() {
                Some(ScVal::Symbol(s)) => std::string::String::from_utf8_lossy(s.0.as_slice()).into_owned(),
                _ => std::string::String::from("?"),
            };
            let b64 = e.body.to_xdr_base64(Limits::none()).unwrap();
            self.lines.push(std::format!("{step} | {name} | {b64}"));
            self.names.insert(name);
        }
    }

    fn finish(self, expected_names: &[&str]) {
        for n in expected_names {
            assert!(self.names.contains(*n), "scenario never emitted `{n}`; coverage incomplete");
        }
        let dir = std::format!("{}/golden", env!("CARGO_MANIFEST_DIR"));
        let path = std::format!("{dir}/events.txt");
        let text = self.lines.join("\n") + "\n";
        if std::env::var("UPDATE_GOLDEN").as_deref() == Ok("1") {
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(&path, &text).unwrap();
        } else {
            let expected = std::fs::read_to_string(&path)
                .unwrap_or_else(|_| panic!("missing golden file {path}"));
            assert_eq!(expected.replace("\r\n", "\n"), text, "event encoding changed vs {path}");
        }
    }
}

#[test]
fn golden_events_smart_account() {
    // Fixed keys: golden bytes must not depend on OsRng.
    let fixed = |b: u8| {
        let sk = SigningKey::from_slice(&[b; 32]).unwrap();
        let pt = VerifyingKey::from(&sk).to_encoded_point(false);
        pt.as_bytes().to_vec()
    };
    let env = Env::default();
    env.mock_all_auths();
    let owner = BytesN::<65>::from_array(&env, fixed(1).as_slice().try_into().unwrap());
    let backup = BytesN::<65>::from_array(&env, fixed(2).as_slice().try_into().unwrap());
    let id = env.register(SmartAccountContract, (owner.clone(), 1u32, String::from_str(&env, "owner")));
    let mut g = GoldenEvents::new(&id);
    g.capture(&env, "constructor");
    let client = SmartAccountContractClient::new(&env, &id);
    client.add_signer(&Signer { public_key: backup.clone(), weight: 1, role: SignerRole::Backup, name: String::from_str(&env, "b") });
    g.capture(&env, "add_signer");
    client.update_signer(&Signer { public_key: backup.clone(), weight: 2, role: SignerRole::Backup, name: String::from_str(&env, "b") });
    g.capture(&env, "update_signer");
    client.update_threshold(&2);
    g.capture(&env, "update_threshold");
    client.update_threshold(&1);
    client.remove_signer(&backup);
    g.capture(&env, "remove_signer");
    g.finish(&["ACCOUNT_INITIALIZED", "SIGNER_ADDED", "SIGNER_UPDATED", "THRESHOLD_UPDATED", "SIGNER_REMOVED"]);
}
