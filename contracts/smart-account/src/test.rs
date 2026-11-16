#![cfg(test)]

extern crate std;

use super::*;
use core::cell::Cell;
use p256::ecdsa::{signature::hazmat::PrehashSigner, Signature, SigningKey, VerifyingKey};
use soroban_sdk::{
    auth::ContractContext,
    testutils::{Address as _, Ledger as _},
    xdr::{
        Hash as XdrHash, HashIdPreimage, HashIdPreimageSorobanAuthorization, InvokeContractArgs, Limits, ScVal,
        SorobanAddressCredentials, SorobanAuthorizationEntry, SorobanAuthorizedFunction, SorobanAuthorizedInvocation,
        SorobanCredentials, FromXdr, ToXdr, VecM, WriteXdr,
    },
    IntoVal, Symbol, TryFromVal,
};
use stellar_accounts::{policies::simple_threshold::SimpleThresholdAccountParams, smart_account::AuthDigestPreimage};

/// SDK 28 returns events as XDR (`ContractEvents`). Convert them back to the
/// `(contract, topics, data)` tuples these assertions use (W4.1).
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


const UP: u8 = 0x01; // WebAuthn "user present"
const UV: u8 = 0x04; // WebAuthn "user verified" (biometric / PIN)

fn key_bytes(env: &Env, kp: &KeyPair) -> Bytes {
    Bytes::from_array(env, &kp.public_key_bytes.to_array())
}

fn ext(env: &Env, verifier: &Address, kp: &KeyPair) -> Signer {
    Signer::External(verifier.clone(), key_bytes(env, kp))
}

/// A deployed account with the real verifier and threshold-policy contracts.
struct Acct {
    env: Env,
    id: Address,
    client: SmartAccountContractClient<'static>,
    verifier: Address,
    policy: Address,
    owner: KeyPair,
    owner_rule: u32,
    nonce: Cell<i64>,
}

fn deploy_on(env: Env) -> Acct {
    let verifier = env.register(constella_webauthn_verifier::WebAuthnVerifier, ());
    let policy = env.register(constella_threshold_policy::ThresholdPolicy, ());
    let owner = generate_keypair(&env);
    let id = env.register(SmartAccountContract, (verifier.clone(), key_bytes(&env, &owner), String::from_str(&env, "owner")));
    let client = SmartAccountContractClient::new(&env, &id);
    let owner_rule = client.get_owner_rule();
    Acct { env, id, client, verifier, policy, owner, owner_rule, nonce: Cell::new(1) }
}

fn deploy() -> Acct {
    deploy_on(Env::default())
}

/// Each passkey signs a WebAuthn assertion over the OZ auth digest:
/// SHA-256(XDR(account, signature_payload, context_rule_ids)).
fn auth_payload(a: &Acct, payload: &BytesN<32>, rule_ids: &[u32], signers: &[&KeyPair]) -> AuthPayload {
    let env = &a.env;
    let ids = Vec::from_slice(env, rule_ids);
    let digest = AuthDigestPreimage { account: a.id.clone(), signature_payload: payload.clone(), context_rule_ids: ids.clone() }.digest(env);
    let mut map = Map::new(env);
    for kp in signers {
        let sig = s1::assertion(env, kp, &digest.to_array(), UP | UV, "webauthn.get");
        map.set(ext(env, &a.verifier, kp), sig.to_xdr(env));
    }
    AuthPayload { signers: map, context_rule_ids: ids }
}

fn call_ctx(env: &Env, contract: &Address, fn_name: &str) -> Context {
    Context::Contract(ContractContext { contract: contract.clone(), fn_name: Symbol::new(env, fn_name), args: vec![env] })
}

/// Runs the account's real `__check_auth` (no mocking) for `contexts`.
fn check(a: &Acct, payload: &BytesN<32>, ap: AuthPayload, contexts: Vec<Context>) -> Result<(), Result<SmartAccountError, soroban_sdk::InvokeError>> {
    a.env.try_invoke_contract_check_auth::<SmartAccountError>(&a.id, payload, ap.into_val(&a.env), &contexts)
}

fn invocation(env: &Env, contract: &Address, fn_name: &str, args: Vec<Val>, subs: std::vec::Vec<SorobanAuthorizedInvocation>) -> SorobanAuthorizedInvocation {
    let args: std::vec::Vec<ScVal> = args.iter().map(|v| ScVal::try_from_val(env, &v).unwrap()).collect();
    SorobanAuthorizedInvocation {
        function: SorobanAuthorizedFunction::ContractFn(InvokeContractArgs {
            contract_address: contract.into(),
            function_name: fn_name.try_into().unwrap(),
            args: args.try_into().unwrap(),
        }),
        sub_invocations: subs.try_into().unwrap_or(VecM::default()),
    }
}

/// Installs a real, passkey-signed Soroban authorization entry for the next
/// call: the host computes the payload and runs `__check_auth` itself.
fn sign_entry(a: &Acct, signers: &[&KeyPair], rule_ids: &[u32], root: SorobanAuthorizedInvocation) {
    let env = &a.env;
    let nonce = a.nonce.get();
    a.nonce.set(nonce + 1);
    let exp = env.ledger().sequence() + 100;
    let preimage = HashIdPreimage::SorobanAuthorization(HashIdPreimageSorobanAuthorization {
        network_id: XdrHash(env.ledger().get().network_id),
        nonce,
        signature_expiration_ledger: exp,
        invocation: root.clone(),
    });
    let xdr = preimage.to_xdr(Limits::none()).unwrap();
    let payload = env.crypto().sha256(&Bytes::from_slice(env, &xdr)).to_bytes();
    let ap = auth_payload(a, &payload, rule_ids, signers);
    let ap_val: Val = ap.into_val(env);
    let signature = ScVal::try_from_val(env, &ap_val).unwrap();
    env.set_auths(&[SorobanAuthorizationEntry {
        credentials: SorobanCredentials::Address(SorobanAddressCredentials {
            address: (&a.id).into(),
            nonce,
            signature_expiration_ledger: exp,
            signature,
        }),
        root_invocation: root,
    }]);
}

/// Signs a call to one of the account's own management functions.
fn sign_self_call(a: &Acct, signers: &[&KeyPair], rule_id: u32, fn_name: &str, args: Vec<Val>) {
    sign_entry(a, signers, &[rule_id], invocation(&a.env, &a.id, fn_name, args, std::vec![]));
}

/// Owner adds a recovery rule scoped to the account itself, M-of-N guardians.
fn add_recovery(a: &Acct, guardians: &[&KeyPair], threshold: u32) -> u32 {
    let env = &a.env;
    let signers = Vec::from_iter(env, guardians.iter().map(|g| ext(env, &a.verifier, g)));
    let mut policies: Map<Address, Val> = Map::new(env);
    policies.set(a.policy.clone(), SimpleThresholdAccountParams { threshold }.into_val(env));
    let ctype = ContextRuleType::CallContract(a.id.clone());
    let name = String::from_str(env, "recovery");
    sign_self_call(a, &[&a.owner], a.owner_rule, "add_context_rule",
        (ctype.clone(), name.clone(), None::<u32>, signers.clone(), policies.clone()).into_val(env));
    a.client.add_context_rule(&ctype, &name, &None, &signers, &policies).id
}

fn contract_err(code: u32) -> soroban_sdk::Error {
    soroban_sdk::Error::from_contract_error(code)
}

fn payload(env: &Env, seed: &[u8]) -> BytesN<32> {
    env.crypto().sha256(&Bytes::from_slice(env, seed)).to_bytes()
}

// -------------------------------------------------------------------------
// T47 / audit M3: the constructor installs the protected owner rule.
// -------------------------------------------------------------------------
#[test]
fn test_t47_constructor_installs_the_owner_rule() {
    let a = deploy();
    assert_eq!(a.client.get_context_rules_count(), 1);
    let rule = a.client.get_context_rule(&a.owner_rule);
    assert_eq!(rule.context_type, ContextRuleType::Default);
    assert_eq!(rule.signers, vec![&a.env, ext(&a.env, &a.verifier, &a.owner)]);
    assert!(rule.policies.is_empty());
    assert_eq!(rule.valid_until, None);

    // Invalid owner keys make the deployment itself fail.
    for bad in [{ let mut k = [5u8; 65]; k[0] = 0x02; k.to_vec() }, { let mut k = [0u8; 65]; k[0] = 0x04; k.to_vec() }, std::vec![4u8; 64]] {
        let env = Env::default();
        let v = env.register(constella_webauthn_verifier::WebAuthnVerifier, ());
        let deployed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            env.register(SmartAccountContract, (v.clone(), Bytes::from_slice(&env, &bad), String::from_str(&env, "x")))
        }));
        assert!(deployed.is_err(), "owner key {:?}.. accepted", &bad[..2]);
    }
}

// -------------------------------------------------------------------------
// T48: the owner passkey authorizes any operation via the Default rule.
// -------------------------------------------------------------------------
#[test]
fn test_t48_owner_passkey_authorizes() {
    let a = deploy();
    let vault = Address::generate(&a.env);
    let usdc = Address::generate(&a.env);
    let p = payload(&a.env, b"t48");
    let ctxs = vec![&a.env, call_ctx(&a.env, &vault, "deposit"), call_ctx(&a.env, &usdc, "transfer")];
    let ap = auth_payload(&a, &p, &[a.owner_rule, a.owner_rule], &[&a.owner]);
    assert_eq!(check(&a, &p, ap, ctxs), Ok(()));
}

// -------------------------------------------------------------------------
// T49: every way a signature can be wrong is rejected.
// -------------------------------------------------------------------------
#[test]
fn test_t49_invalid_authorizations_are_rejected() {
    let a = deploy();
    let env = &a.env;
    let vault = Address::generate(env);
    let ctx = || vec![env, call_ctx(env, &vault, "deposit")];
    let p = payload(env, b"t49");
    let stranger = generate_keypair(env);

    // A key that is not in the rule.
    let r = check(&a, &p, auth_payload(&a, &p, &[a.owner_rule], &[&stranger]), ctx());
    assert_eq!(r, Err(Ok(SmartAccountError::UnvalidatedContext)));
    // Owner plus an extra, unknown signer: the extra signer is refused.
    let r = check(&a, &p, auth_payload(&a, &p, &[a.owner_rule], &[&a.owner, &stranger]), ctx());
    assert_eq!(r, Err(Ok(SmartAccountError::UnauthorizedSigner)));
    // No signatures at all.
    let r = check(&a, &p, AuthPayload { signers: Map::new(env), context_rule_ids: vec![env, a.owner_rule] }, ctx());
    assert_eq!(r, Err(Ok(SmartAccountError::UnvalidatedContext)));
    // A rule that does not exist / one rule id per context.
    let r = check(&a, &p, auth_payload(&a, &p, &[99], &[&a.owner]), ctx());
    assert_eq!(r, Err(Ok(SmartAccountError::ContextRuleNotFound)));
    let r = check(&a, &p, auth_payload(&a, &p, &[a.owner_rule, a.owner_rule], &[&a.owner]), ctx());
    assert_eq!(r, Err(Ok(SmartAccountError::ContextRuleIdsLengthMismatch)));

    // Signed for another transaction.
    let other = payload(env, b"another transaction");
    assert!(check(&a, &p, auth_payload(&a, &other, &[a.owner_rule], &[&a.owner]), ctx()).is_err());
    // Zeroed signature.
    let mut ap = auth_payload(&a, &p, &[a.owner_rule], &[&a.owner]);
    let key = ext(env, &a.verifier, &a.owner);
    let mut sig = stellar_accounts::verifiers::webauthn::WebAuthnSigData::from_xdr(env, &ap.signers.get(key.clone()).unwrap()).unwrap();
    sig.signature = BytesN::from_array(env, &[0u8; 64]);
    ap.signers.set(key.clone(), sig.to_xdr(env));
    assert!(check(&a, &p, ap, ctx()).is_err());
    // The rule choice is part of what was signed: re-labelling it breaks the signature.
    let mut ap = auth_payload(&a, &p, &[a.owner_rule], &[&a.owner]);
    let rec = add_recovery(&a, &[&generate_keypair(env), &generate_keypair(env)], 1);
    let mut ids = Vec::new(env);
    ids.push_back(rec);
    ap.context_rule_ids = ids;
    assert!(check(&a, &p, ap, ctx()).is_err());
}

// -------------------------------------------------------------------------
// T50: social recovery is M-of-N and scoped to account management only.
// -------------------------------------------------------------------------
#[test]
fn test_t50_recovery_is_threshold_gated_and_cannot_move_funds() {
    let a = deploy();
    let env = &a.env;
    let (g1, g2, g3) = (generate_keypair(env), generate_keypair(env), generate_keypair(env));
    let rec = add_recovery(&a, &[&g1, &g2, &g3], 2);
    let p = payload(env, b"t50");
    let manage = || vec![env, call_ctx(env, &a.id, "propose_recovery")];

    // 1 of 3 is not enough.
    assert!(check(&a, &p, auth_payload(&a, &p, &[rec], &[&g1]), manage()).is_err());
    // 2 of 3 may propose a recovery (it then waits 48 h - see s4).
    assert_eq!(check(&a, &p, auth_payload(&a, &p, &[rec], &[&g1, &g3]), manage()), Ok(()));
    // ... but can NOT authorize anything else: a vault deposit, a token transfer.
    let vault = Address::generate(env);
    let r = check(&a, &p, auth_payload(&a, &p, &[rec], &[&g1, &g2, &g3]), vec![env, call_ctx(env, &vault, "deposit")]);
    assert_eq!(r, Err(Ok(SmartAccountError::UnvalidatedContext)));
    // The owner is not a recovery signer.
    let r = check(&a, &p, auth_payload(&a, &p, &[rec], &[&a.owner, &g1]), manage());
    assert_eq!(r, Err(Ok(SmartAccountError::UnauthorizedSigner)));
}

// -------------------------------------------------------------------------
// T51: owner rotation through recovery (real signed entries) and every
// anti-bricking guard.
// -------------------------------------------------------------------------
#[test]
fn test_t51_owner_rotation_by_recovery_and_anti_bricking() {
    let a = deploy();
    let env = &a.env;
    let (g1, g2, g3) = (generate_keypair(env), generate_keypair(env), generate_keypair(env));
    let rec = add_recovery(&a, &[&g1, &g2, &g3], 2);
    let vault = Address::generate(env);
    let spend = || vec![env, call_ctx(env, &vault, "deposit")];

    // Lost phone: 2 guardians propose a new owner passkey; after the 48 h
    // recovery delay (decision D-F) anyone executes it.
    let new_owner = generate_keypair(env);
    let new_signer = ext(env, &a.verifier, &new_owner);
    sign_self_call(&a, &[&g1, &g2], rec, "propose_recovery", (new_signer.clone(),).into_val(env));
    a.client.propose_recovery(&new_signer);
    env.ledger().set_timestamp(env.ledger().timestamp() + s4::DELAY);
    a.client.execute_recovery();

    let p = payload(env, b"after rotation");
    assert!(check(&a, &p, auth_payload(&a, &p, &[a.owner_rule], &[&a.owner]), spend()).is_err(), "old key still works");
    assert_eq!(check(&a, &p, auth_payload(&a, &p, &[a.owner_rule], &[&new_owner]), spend()), Ok(()));

    // Anti-bricking, each attempted with a valid owner signature:
    let owner = &new_owner;
    sign_self_call(&a, &[owner], a.owner_rule, "remove_context_rule", (a.owner_rule,).into_val(env));
    assert_eq!(a.client.try_remove_context_rule(&a.owner_rule).err().unwrap().unwrap(), contract_err(AccError::OwnerRuleProtected as u32));
    sign_self_call(&a, &[owner], a.owner_rule, "update_context_rule_valid_until", (a.owner_rule, Some(500u32)).into_val(env));
    assert_eq!(a.client.try_update_context_rule_valid_until(&a.owner_rule, &Some(500)).err().unwrap().unwrap(), contract_err(AccError::OwnerRuleProtected as u32));
    let only = a.client.get_signer_id(&new_signer);
    sign_self_call(&a, &[owner], a.owner_rule, "remove_signer", (a.owner_rule, only).into_val(env));
    assert_eq!(a.client.try_remove_signer(&a.owner_rule, &only).err().unwrap().unwrap(), contract_err(SmartAccountError::NoSignersAndPolicies as u32));

    // Recovery 2-of-3: removing one guardian is fine, a second would strand it.
    let g1_id = a.client.get_signer_id(&ext(env, &a.verifier, &g1));
    sign_self_call(&a, &[owner], a.owner_rule, "remove_signer", (rec, g1_id).into_val(env));
    a.client.remove_signer(&rec, &g1_id);
    let g2_id = a.client.get_signer_id(&ext(env, &a.verifier, &g2));
    sign_self_call(&a, &[owner], a.owner_rule, "remove_signer", (rec, g2_id).into_val(env));
    assert_eq!(a.client.try_remove_signer(&rec, &g2_id).err().unwrap().unwrap(), contract_err(AccError::ThresholdUnreachable as u32));
    assert_eq!(a.client.get_context_rule(&rec).signers.len(), 2);

    // A non-owner rule can still be retired by the owner.
    sign_self_call(&a, &[owner], a.owner_rule, "remove_context_rule", (rec,).into_val(env));
    a.client.remove_context_rule(&rec);
    assert_eq!(a.client.get_context_rules_count(), 1);
}

#[test]
fn test_management_requires_the_accounts_own_authorization() {
    let a = deploy();
    let env = &a.env;
    let k = ext(env, &a.verifier, &generate_keypair(env));
    // No authorization entry at all.
    assert!(a.client.try_add_signer(&a.owner_rule, &k).is_err());
    // A stranger's passkey signature.
    let stranger = generate_keypair(env);
    sign_self_call(&a, &[&stranger], a.owner_rule, "add_signer", (a.owner_rule, k.clone()).into_val(env));
    assert!(a.client.try_add_signer(&a.owner_rule, &k).is_err());
    assert_eq!(a.client.get_context_rule(&a.owner_rule).signers.len(), 1);
}

#[test]
fn test_t55_sponsored_passkey_vault_deposit() {
    // Setup steps use mock_all_auths; the deposit itself runs under the
    // user's real passkey-signed authorization entry (W4.3b).
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
        feeder: Address::generate(&env),
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
        feeder: Address::generate(&env),
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
        feeder: Address::generate(&env),
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
    let acct = deploy_on(env.clone());
    let user_account_id = acct.id.clone();

    // Mint USDC deposit funds to the user smart account (e.g. 1000 USDC)
    numeraire_admin.mint(&user_account_id, &1000_0000000);
    assert_eq!(numeraire_client.balance(&user_account_id), 1000_0000000);

    // Zero XLM user flow:
    // A) Sponsor verifies transaction eligibility before signing fee-bump
    let fee_bid_stroops = 10_000; // 0.001 XLM gas
    let sponsor_val = sponsor_client.try_validate_sponsorship(&user_account_id, &vault_id, &fee_bid_stroops);
    assert_eq!(sponsor_val, Ok(Ok(())));

    // B) User executes deposit of 500 USDC into Strategy Vault via smart account
    // The deposit authorizes two operations for the account: the vault call
    // (operator) and the USDC transfer it triggers (from). Both use the owner rule.
    let amount: i128 = 500_0000000;
    let tree = || invocation(&env, &vault_id, "deposit",
        (amount, user_account_id.clone(), user_account_id.clone(), user_account_id.clone()).into_val(&env),
        std::vec![invocation(&env, &numeraire_address, "transfer", (user_account_id.clone(), vault_id.clone(), amount).into_val(&env), std::vec![])]);
    // A stranger's passkey cannot spend the account's USDC.
    let stranger = generate_keypair(&env);
    sign_entry(&acct, &[&stranger], &[acct.owner_rule, acct.owner_rule], tree());
    assert!(vault_client.try_deposit(&amount, &user_account_id, &user_account_id, &user_account_id).is_err());
    assert_eq!(numeraire_client.balance(&user_account_id), 1000_0000000);
    // The owner's Face ID signature does.
    sign_entry(&acct, &[&acct.owner], &[acct.owner_rule, acct.owner_rule], tree());
    let shares_minted = vault_client.deposit(&amount, &user_account_id, &user_account_id, &user_account_id);
    env.mock_all_auths(); // remaining steps are sponsor/admin bookkeeping
    assert_eq!(shares_minted, 500_0000000);

    // C) The designated relayer (audit F1b) records the sponsored transaction
    let relayer = Address::generate(&env);
    sponsor_client.add_relayer(&relayer);
    sponsor_client.record_sponsored_op(&relayer, &user_account_id, &vault_id, &fee_bid_stroops);

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
    let a = deploy();
    let env = &a.env;
    let attacker = generate_keypair(env);
    let args = (a.verifier.clone(), key_bytes(env, &attacker), String::from_str(env, "attacker")).into_val(env);
    let r = env.try_invoke_contract::<(), soroban_sdk::Error>(&a.id, &Symbol::new(env, "initialize"), args);
    assert!(!matches!(r, Ok(Ok(()))), "a stranger must not be able to install their key");
    assert_eq!(a.client.get_context_rules_count(), 1);
    assert_eq!(a.client.get_context_rule(&a.owner_rule).signers, vec![env, ext(env, &a.verifier, &a.owner)]);
}

/// Events emitted by the audited OpenZeppelin smart-account library itself
/// (AGENT-RULES Rule 8 exception for audited library events).
const OZ_ACCOUNT_EVENTS: [&str; 11] = [
    "context_rule_added", "context_rule_meta_updated", "context_rule_removed", "signer_added", "signer_removed",
    "policy_added", "policy_removed", "signer_registered", "signer_deregistered", "policy_registered", "policy_deregistered",
];

fn event_name(env: &Env, topic: &Val) -> std::string::String {
    use soroban_sdk::{FromVal, SymbolStr};
    let sym = Symbol::try_from_val(env, topic).unwrap();
    let s = SymbolStr::from_val(env, &sym.to_symbol_val());
    let b: &[u8] = s.as_ref();
    std::string::String::from_utf8(b.to_vec()).unwrap()
}

// -------------------------------------------------------------------------
// Audit M2: Constella events are SCREAMING_SNAKE_CASE (Rule 8); the only
// other events are the OpenZeppelin library's own.
// -------------------------------------------------------------------------
#[test]
fn m2_every_account_event_follows_rule_8() {
    let env = Env::default();
    env.mock_all_auths(); // event naming only; auth is covered by T49-T51
    let a = deploy_on(env.clone());
    let mut seen = std::collections::BTreeSet::new();
    let mut record = |env: &Env| {
        for (c, topics, _) in all_events(env).iter() {
            if c != a.id { continue; }
            let n = event_name(env, &topics.get(0).unwrap());
            let screaming = n.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_');
            assert!(screaming || OZ_ACCOUNT_EVENTS.contains(&n.as_str()), "event `{n}` violates Rule 8");
            seen.insert(n);
        }
    };
    let g = generate_keypair(&env);
    let mut policies: Map<Address, Val> = Map::new(&env);
    policies.set(a.policy.clone(), SimpleThresholdAccountParams { threshold: 1 }.into_val(&env));
    let rule = a.client.add_context_rule(&ContextRuleType::CallContract(a.id.clone()), &String::from_str(&env, "recovery"), &None, &vec![&env, ext(&env, &a.verifier, &g)], &policies);
    record(&env);
    a.client.add_signer(&rule.id, &ext(&env, &a.verifier, &generate_keypair(&env)));
    record(&env);
    a.client.remove_context_rule(&rule.id);
    record(&env);
    assert!(seen.len() >= 4, "saw {seen:?}");
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
    let fixed = |env: &Env, b: u8| {
        let sk = SigningKey::from_slice(&[b; 32]).unwrap();
        let pt = VerifyingKey::from(&sk).to_encoded_point(false);
        Bytes::from_slice(env, pt.as_bytes())
    };
    let env = Env::default();
    env.mock_all_auths(); // event encodings only; auth is covered by T49-T51
    let verifier = env.register(constella_webauthn_verifier::WebAuthnVerifier, ());
    let policy = env.register(constella_threshold_policy::ThresholdPolicy, ());
    let id = env.register(SmartAccountContract, (verifier.clone(), fixed(&env, 1), String::from_str(&env, "owner")));
    let mut g = GoldenEvents::new(&id);
    g.capture(&env, "constructor");
    let client = SmartAccountContractClient::new(&env, &id);
    let guardians = vec![&env, Signer::External(verifier.clone(), fixed(&env, 2)), Signer::External(verifier.clone(), fixed(&env, 3))];
    let mut policies: Map<Address, Val> = Map::new(&env);
    policies.set(policy.clone(), SimpleThresholdAccountParams { threshold: 1 }.into_val(&env));
    let rule = client.add_context_rule(&ContextRuleType::CallContract(id.clone()), &String::from_str(&env, "recovery"), &None, &guardians, &policies);
    g.capture(&env, "add_context_rule");
    let sid = client.add_signer(&rule.id, &Signer::External(verifier.clone(), fixed(&env, 4)));
    g.capture(&env, "add_signer");
    client.remove_signer(&rule.id, &sid);
    g.capture(&env, "remove_signer");
    client.update_context_rule_name(&rule.id, &String::from_str(&env, "guardians"));
    g.capture(&env, "update_context_rule_name");
    client.remove_context_rule(&rule.id);
    g.capture(&env, "remove_context_rule");
    // Audit S4: delayed recovery events.
    let new_owner = Signer::External(verifier.clone(), fixed(&env, 5));
    client.propose_recovery(&new_owner);
    g.capture(&env, "propose_recovery");
    client.cancel_recovery();
    g.capture(&env, "cancel_recovery");
    client.propose_recovery(&new_owner);
    env.ledger().set_timestamp(env.ledger().timestamp() + RECOVERY_DELAY_SECONDS);
    client.execute_recovery();
    g.capture(&env, "execute_recovery");
    g.finish(&["ACCOUNT_INITIALIZED", "context_rule_added", "signer_registered", "policy_registered", "signer_added",
        "signer_removed", "signer_deregistered", "context_rule_meta_updated", "context_rule_removed",
        "RECOVERY_PROPOSED", "RECOVERY_CANCELLED", "RECOVERY_EXECUTED"]);
}

// -------------------------------------------------------------------------
// Audit S1 (2026-11-15): real passkeys sign a WebAuthn assertion. W3C WebAuthn
// L2 §7.2: the authenticator signs SHA-256(authenticatorData ||
// SHA-256(clientDataJSON)), clientDataJSON carrying type "webauthn.get" and
// challenge = base64url(the digest the account asks for - here the OZ auth
// digest). Assertions are built byte-for-byte in that format with real keys.
// -------------------------------------------------------------------------
mod s1 {
    use super::*;
    use stellar_accounts::verifiers::webauthn::WebAuthnSigData;

    /// RFC 4648 §5 base64url without padding (independent of the contract code).
    fn b64url(bytes: &[u8]) -> std::string::String {
        const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let mut out = std::string::String::new();
        for chunk in bytes.chunks(3) {
            let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
            let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
            for i in 0..(chunk.len() + 1) {
                out.push(A[((n >> (18 - 6 * i)) & 63) as usize] as char);
            }
        }
        out
    }

    fn sha(env: &Env, data: &[u8]) -> [u8; 32] {
        env.crypto().sha256(&Bytes::from_slice(env, data)).to_array()
    }

    /// An assertion as a platform authenticator (Face ID / Touch ID) returns it.
    pub(super) fn assertion(env: &Env, kp: &KeyPair, challenge: &[u8; 32], flags: u8, typ: &str) -> WebAuthnSigData {
        let mut auth_data = sha(env, b"constella.app").to_vec(); // rpIdHash
        auth_data.push(flags);
        auth_data.extend_from_slice(&[0, 0, 0, 7]); // signCount
        let client_data = std::format!(
            r#"{{"type":"{typ}","challenge":"{}","origin":"https://constella.app","crossOrigin":false}}"#,
            b64url(challenge)
        );
        let mut msg = auth_data.clone();
        msg.extend_from_slice(&sha(env, client_data.as_bytes()));
        WebAuthnSigData {
            signature: sign_payload(env, kp, &sha(env, &msg)),
            authenticator_data: Bytes::from_slice(env, &auth_data),
            client_data: Bytes::from_slice(env, client_data.as_bytes()),
        }
    }

    fn digest(a: &Acct, p: &BytesN<32>) -> [u8; 32] {
        AuthDigestPreimage { account: a.id.clone(), signature_payload: p.clone(), context_rule_ids: vec![&a.env, a.owner_rule] }
            .digest(&a.env)
            .to_array()
    }

    fn check_sig(a: &Acct, p: &BytesN<32>, sig: WebAuthnSigData) -> Result<(), Result<SmartAccountError, soroban_sdk::InvokeError>> {
        let mut signers = Map::new(&a.env);
        signers.set(ext(&a.env, &a.verifier, &a.owner), sig.to_xdr(&a.env));
        let ap = AuthPayload { signers, context_rule_ids: vec![&a.env, a.owner_rule] };
        check(a, p, ap, vec![&a.env, call_ctx(&a.env, &Address::generate(&a.env), "deposit")])
    }

    fn setup() -> (Acct, BytesN<32>) {
        let a = deploy();
        let p = payload(&a.env, b"s1-transaction");
        (a, p)
    }

    #[test]
    fn a_real_webauthn_assertion_authorizes() {
        let (a, p) = setup();
        let d = digest(&a, &p);
        assert_eq!(check_sig(&a, &p, assertion(&a.env, &a.owner, &d, UP | UV, "webauthn.get")), Ok(()));
    }

    #[test]
    fn a_signature_over_the_raw_payload_is_rejected() {
        // The pre-S1 scheme: P-256 directly over the 32-byte payload.
        let (a, p) = setup();
        let d = digest(&a, &p);
        let mut sig = assertion(&a.env, &a.owner, &d, UP | UV, "webauthn.get");
        sig.signature = sign_payload(&a.env, &a.owner, &p.to_array());
        assert!(check_sig(&a, &p, sig).is_err());
    }

    #[test]
    fn an_assertion_for_another_transaction_is_rejected() {
        let (a, p) = setup();
        let other = digest(&a, &payload(&a.env, b"some-other-transaction"));
        assert!(check_sig(&a, &p, assertion(&a.env, &a.owner, &other, UP | UV, "webauthn.get")).is_err());
    }

    #[test]
    fn user_presence_and_verification_are_required() {
        let (a, p) = setup();
        let d = digest(&a, &p);
        assert!(check_sig(&a, &p, assertion(&a.env, &a.owner, &d, UP, "webauthn.get")).is_err());
        assert!(check_sig(&a, &p, assertion(&a.env, &a.owner, &d, UV, "webauthn.get")).is_err());
    }

    #[test]
    fn a_registration_ceremony_is_not_an_authorization() {
        let (a, p) = setup();
        let d = digest(&a, &p);
        assert!(check_sig(&a, &p, assertion(&a.env, &a.owner, &d, UP | UV, "webauthn.create")).is_err());
    }

    #[test]
    fn tampered_client_data_breaks_the_signature() {
        let (a, p) = setup();
        let d = digest(&a, &p);
        let mut sig = assertion(&a.env, &a.owner, &d, UP | UV, "webauthn.get");
        let cd: std::vec::Vec<u8> = sig.client_data.iter().collect();
        let s = std::string::String::from_utf8(cd).unwrap().replace("constella.app", "evil.example");
        sig.client_data = Bytes::from_slice(&a.env, s.as_bytes());
        assert!(check_sig(&a, &p, sig).is_err());
    }
}

// -------------------------------------------------------------------------
// Audit S4 / decision D-F (2026-11-15): social recovery waits 48 hours and
// the owner can cancel it; guardians can never change the account directly.
// -------------------------------------------------------------------------
mod s4 {
    use super::*;

    pub(super) const DELAY: u64 = 48 * 3600;
    const T0: u64 = 1_000_000;

    fn setup() -> (Acct, KeyPair, KeyPair, KeyPair, u32) {
        let a = deploy();
        a.env.ledger().set_timestamp(T0);
        let (g1, g2, g3) = (generate_keypair(&a.env), generate_keypair(&a.env), generate_keypair(&a.env));
        let rec = add_recovery(&a, &[&g1, &g2, &g3], 2);
        (a, g1, g2, g3, rec)
    }

    fn propose(a: &Acct, signers: &[&KeyPair], rec: u32, new_owner: &Signer) {
        sign_self_call(a, signers, rec, "propose_recovery", (new_owner.clone(),).into_val(&a.env));
        a.client.propose_recovery(new_owner);
    }

    #[test]
    fn guardians_cannot_change_the_account_directly() {
        let (a, g1, g2, _g3, rec) = setup();
        let env = &a.env;
        let k = ext(env, &a.verifier, &generate_keypair(env));
        // Real __check_auth: a management call selected under the recovery rule.
        let p = payload(env, b"s4");
        for f in ["add_signer", "remove_signer", "add_context_rule", "remove_context_rule", "cancel_recovery"] {
            let r = check(&a, &p, auth_payload(&a, &p, &[rec], &[&g1, &g2]), vec![env, call_ctx(env, &a.id, f)]);
            assert!(r.is_err(), "guardians could authorize `{f}`");
        }
        // Real signed entry, end to end.
        sign_self_call(&a, &[&g1, &g2], rec, "add_signer", (a.owner_rule, k.clone()).into_val(env));
        assert!(a.client.try_add_signer(&a.owner_rule, &k).is_err());
        assert_eq!(a.client.get_context_rule(&a.owner_rule).signers.len(), 1);
    }

    #[test]
    fn recovery_executes_only_after_48_hours_and_then_by_anyone() {
        let (a, g1, _g2, g3, rec) = setup();
        let env = &a.env;
        let new_owner = generate_keypair(env);
        let ns = ext(env, &a.verifier, &new_owner);
        propose(&a, &[&g1, &g3], rec, &ns);
        let pending = a.client.get_pending_recovery().unwrap();
        assert_eq!((pending.new_owner.clone(), pending.executable_at), (ns.clone(), T0 + DELAY));

        env.ledger().set_timestamp(T0 + DELAY - 1);
        assert_eq!(a.client.try_execute_recovery().err().unwrap().unwrap(), contract_err(AccError::RecoveryNotReady as u32));
        // The old owner keeps full control during the window.
        let vault = Address::generate(env);
        let p = payload(env, b"during window");
        assert_eq!(check(&a, &p, auth_payload(&a, &p, &[a.owner_rule], &[&a.owner]), vec![env, call_ctx(env, &vault, "deposit")]), Ok(()));

        env.ledger().set_timestamp(T0 + DELAY);
        a.client.execute_recovery(); // no authorization needed
        assert_eq!(a.client.get_context_rule(&a.owner_rule).signers, vec![env, ns.clone()]);
        assert_eq!(a.client.get_pending_recovery(), None);
        let p = payload(env, b"after");
        let spend = || vec![env, call_ctx(env, &vault, "deposit")];
        assert!(check(&a, &p, auth_payload(&a, &p, &[a.owner_rule], &[&a.owner]), spend()).is_err());
        assert_eq!(check(&a, &p, auth_payload(&a, &p, &[a.owner_rule], &[&new_owner]), spend()), Ok(()));
    }

    #[test]
    fn the_owner_can_cancel_and_guardians_cannot() {
        let (a, g1, g2, _g3, rec) = setup();
        let env = &a.env;
        let attacker_key = ext(env, &a.verifier, &generate_keypair(env));
        propose(&a, &[&g1, &g2], rec, &attacker_key);

        sign_self_call(&a, &[&g1, &g2], rec, "cancel_recovery", ().into_val(env));
        assert!(a.client.try_cancel_recovery().is_err());
        assert!(a.client.get_pending_recovery().is_some());

        sign_self_call(&a, &[&a.owner], a.owner_rule, "cancel_recovery", ().into_val(env));
        a.client.cancel_recovery();
        assert_eq!(a.client.get_pending_recovery(), None);
        env.ledger().set_timestamp(T0 + DELAY);
        assert_eq!(a.client.try_execute_recovery().err().unwrap().unwrap(), contract_err(AccError::NoPendingRecovery as u32));
        assert_eq!(a.client.get_context_rule(&a.owner_rule).signers, vec![env, ext(env, &a.verifier, &a.owner)]);
    }

    #[test]
    fn a_new_proposal_restarts_the_clock() {
        let (a, g1, g2, g3, rec) = setup();
        let env = &a.env;
        let first = ext(env, &a.verifier, &generate_keypair(env));
        let second = ext(env, &a.verifier, &generate_keypair(env));
        propose(&a, &[&g1, &g2], rec, &first);
        env.ledger().set_timestamp(T0 + 10 * 3600);
        propose(&a, &[&g2, &g3], rec, &second);
        let pending = a.client.get_pending_recovery().unwrap();
        assert_eq!((pending.new_owner, pending.executable_at), (second.clone(), T0 + 10 * 3600 + DELAY));
        env.ledger().set_timestamp(T0 + DELAY);
        assert!(a.client.try_execute_recovery().is_err());
    }

    #[test]
    fn the_current_owner_key_cannot_be_proposed() {
        let (a, g1, g2, _g3, rec) = setup();
        let env = &a.env;
        let current = ext(env, &a.verifier, &a.owner);
        sign_self_call(&a, &[&g1, &g2], rec, "propose_recovery", (current.clone(),).into_val(env));
        assert_eq!(a.client.try_propose_recovery(&current).err().unwrap().unwrap(), contract_err(AccError::AlreadyOwner as u32));
    }
}

/// Audit F1: cross-implementation vectors. The relayer's Python client
/// (`services/relayer/.../passkey.py::auth_digest`) must encode OpenZeppelin's
/// `AuthDigestPreimage` byte-for-byte; the same constants are asserted in
/// `services/relayer/tests/test_passkey.py`.
#[test]
fn f1_auth_digest_vectors_match_the_python_client() {
    let env = Env::default();
    let account = Address::from_str(&env, "CAIRCEIRCEIRCEIRCEIRCEIRCEIRCEIRCEIRCEIRCEIRCEIRCEIRDB3V");
    let mut p = [0u8; 32];
    for (i, b) in p.iter_mut().enumerate() {
        *b = i as u8;
    }
    let payload = BytesN::from_array(&env, &p);
    let hex = |d: BytesN<32>| d.to_array().iter().map(|b| std::format!("{b:02x}")).collect::<std::string::String>();
    let d1 = AuthDigestPreimage { account: account.clone(), signature_payload: payload.clone(), context_rule_ids: vec![&env, 0u32, 0u32] }.digest(&env);
    assert_eq!(hex(d1), "9b19bf4175973da39fd001b8251d481ef104e9672369c0ee611e68450611210a");
    let d2 = AuthDigestPreimage { account, signature_payload: payload, context_rule_ids: vec![&env, 3u32] }.digest(&env);
    assert_eq!(hex(d2), "d506511b29a2196dba8d9145cfa66841c45e54ba8a9298e5683a608cb008859b");
}
