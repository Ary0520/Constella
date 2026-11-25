//! Step 8 (D-F; OPEN-ISSUES #2) on the REAL smart-account, verifier and
//! threshold-policy WASMs via the real account factory. Authorization is mocked
//! here (registry logic); the guardian passkey-signature path itself is covered
//! with real WebAuthn signatures in smart-account (s4::*, device_recovery::*).
extern crate std;
use super::*;
use constella_account_factory::{AccountFactory, AccountFactoryClient as FactoryClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    vec, IntoVal, Symbol, Val,
};
use stellar_accounts::smart_account::{ContextRule, ContextRuleType, Signer};

const ACCOUNT_WASM: &[u8] = include_bytes!("../../target/wasm32v1-none/release/constella_smart_account.wasm");
const VERIFIER_WASM: &[u8] = include_bytes!("../../target/wasm32v1-none/release/constella_webauthn_verifier.wasm");
const POLICY_WASM: &[u8] = include_bytes!("../../target/wasm32v1-none/release/constella_threshold_policy.wasm");
// P-256 multiples of G (uncompressed, valid points): G, 2G, 3G, 4G.
const OWNER: &str = "046b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c2964fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5";
const G1: &str = "047cf27b188d034f7e8a52380304b51ac3c08969e277f21b35a60b48fc4766997807775510db8ed040293d9ac69f7430dbba7dade63ce982299e04b79d227873d1";
const G2: &str = "045ecbe4d1a6330a44c8f7ef951d4bf165e6c6b721efada985fb41661bc6e7fd6c8734640c4998ff7e374b06ce1a64a2ecd82ab036384fb83d9a79b127a27d5032";
const NEW: &str = "04e2534a3532d08fbba02dde659ee62bd0031fe2db785596ef509302446b030852e0f1575a4c633cc719dfee5fda862d764efc96c3f30ee0055c42c23f184ed8c6";
const T0: u64 = 1_731_600_000;
const DELAY: u64 = 48 * 3600;

fn key(env: &Env, hex: &str) -> Bytes {
    let raw: std::vec::Vec<u8> = (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect();
    Bytes::from_slice(env, &raw)
}

struct F { env: Env, reg: RecoveryRegistryClient<'static>, account: Address, verifier: Address, policy: Address }

fn setup() -> F {
    let env = Env::default();
    env.cost_estimate().budget().reset_unlimited();
    env.ledger().set_timestamp(T0);
    let verifier = env.register(VERIFIER_WASM, ());
    let policy = env.register(POLICY_WASM, ());
    let hash = env.deployer().upload_contract_wasm(ACCOUNT_WASM);
    let factory = FactoryClient::new(&env, &env.register(AccountFactory, (Address::generate(&env), verifier.clone(), hash)));
    let account = factory.create_account(&key(&env, OWNER));
    let reg = RecoveryRegistryClient::new(&env, &env.register(RecoveryRegistry, (factory.address.clone(), verifier.clone(), policy.clone())));
    F { env, reg, account, verifier, policy }
}

fn guardians(f: &F) -> Vec<Bytes> {
    vec![&f.env, key(&f.env, G1), key(&f.env, G2)]
}

fn rule(f: &F, id: u32) -> ContextRule {
    f.env.invoke_contract(&f.account, &Symbol::new(&f.env, "get_context_rule"), vec![&f.env, id.into_val(&f.env)])
}

#[test]
fn guardians_become_an_account_scoped_threshold_rule() {
    let f = setup();
    f.env.mock_all_auths();
    let id = f.reg.set_guardians(&f.account, &guardians(&f), &2);
    let r = rule(&f, id);
    assert_eq!(r.context_type, ContextRuleType::CallContract(f.account.clone()), "scoped to the account: propose only (S4)");
    assert_eq!(r.signers, vec![&f.env, Signer::External(f.verifier.clone(), key(&f.env, G1)),
                                Signer::External(f.verifier.clone(), key(&f.env, G2))]);
    let threshold: u32 = f.env.invoke_contract(&f.policy, &Symbol::new(&f.env, "get_threshold"),
                                               vec![&f.env, id.into_val(&f.env), f.account.clone().into_val(&f.env)]);
    assert_eq!(threshold, 2);
    assert_eq!(f.reg.guardians_of(&f.account), Some(GuardianSet { rule_id: id, threshold: 2, guardians: guardians(&f) }));
    assert_eq!(f.reg.accounts_guarded_by(&key(&f.env, G1)), vec![&f.env, f.account.clone()]);
}

#[test]
fn only_the_account_itself_can_set_its_guardians() {
    let f = setup();
    assert!(f.reg.try_set_guardians(&f.account, &guardians(&f), &1).is_err());
    assert!(f.reg.guardians_of(&f.account).is_none());
}

#[test]
fn bad_guardian_sets_are_refused() {
    let f = setup();
    f.env.mock_all_auths();
    let g = guardians(&f);
    assert_eq!(f.reg.try_set_guardians(&f.account, &g, &0), Err(Ok(RecoveryError::InvalidThreshold.into())));
    assert_eq!(f.reg.try_set_guardians(&f.account, &g, &3), Err(Ok(RecoveryError::InvalidThreshold.into())));
    let dup = vec![&f.env, key(&f.env, G1), key(&f.env, G1)];
    assert_eq!(f.reg.try_set_guardians(&f.account, &dup, &1), Err(Ok(RecoveryError::DuplicateGuardian.into())));
    let bad = vec![&f.env, Bytes::from_slice(&f.env, &[4u8; 33])];
    assert_eq!(f.reg.try_set_guardians(&f.account, &bad, &1), Err(Ok(RecoveryError::InvalidKey.into())));
    let mut many = Vec::new(&f.env);
    for _ in 0..=MAX_GUARDIANS {
        many.push_back(key(&f.env, G1));
    }
    assert_eq!(f.reg.try_set_guardians(&f.account, &many, &1), Err(Ok(RecoveryError::TooManyGuardians.into())));
    let own = vec![&f.env, key(&f.env, OWNER)];
    assert_eq!(f.reg.try_set_guardians(&f.account, &own, &1), Err(Ok(RecoveryError::InvalidKey.into())), "the owner key cannot guard itself");
    let stranger = Address::generate(&f.env);
    assert_eq!(f.reg.try_set_guardians(&stranger, &g, &1), Err(Ok(RecoveryError::UnknownAccount.into())));
    f.reg.set_guardians(&f.account, &g, &1);
    assert_eq!(f.reg.try_set_guardians(&f.account, &g, &1), Err(Ok(RecoveryError::GuardiansAlreadySet.into())));
}

#[test]
fn after_recovery_the_new_key_finds_the_account() {
    let f = setup();
    f.env.mock_all_auths();
    f.reg.set_guardians(&f.account, &guardians(&f), &1);
    let new_owner = Signer::External(f.verifier.clone(), key(&f.env, NEW));
    let _: Val = f.env.invoke_contract(&f.account, &Symbol::new(&f.env, "propose_recovery"), vec![&f.env, new_owner.into_val(&f.env)]);
    // Before the switch the new key is not the owner: recording it is refused.
    assert_eq!(f.reg.try_record_owner(&f.account, &key(&f.env, NEW)), Err(Ok(RecoveryError::NotOwnerKey.into())));
    f.env.ledger().set_timestamp(T0 + DELAY);
    let _: Val = f.env.invoke_contract(&f.account, &Symbol::new(&f.env, "execute_recovery"), vec![&f.env]);
    assert_eq!(f.reg.owner_account(&key(&f.env, NEW)), None);
    f.reg.record_owner(&f.account, &key(&f.env, NEW)); // permissionless and verifiable
    assert_eq!(f.reg.owner_account(&key(&f.env, NEW)), Some(f.account.clone()));
    // The old key no longer owns it: it cannot be recorded.
    assert_eq!(f.reg.try_record_owner(&f.account, &key(&f.env, OWNER)), Err(Ok(RecoveryError::NotOwnerKey.into())));
}

#[test]
fn finish_executes_a_due_recovery_and_records_the_new_key_in_one_call() {
    let f = setup();
    f.env.mock_all_auths();
    f.reg.set_guardians(&f.account, &guardians(&f), &1);
    let new_owner = Signer::External(f.verifier.clone(), key(&f.env, NEW));
    let _: Val = f.env.invoke_contract(&f.account, &Symbol::new(&f.env, "propose_recovery"), vec![&f.env, new_owner.into_val(&f.env)]);
    // Not before the 48-hour delay (the account refuses), and nothing is recorded.
    assert!(f.reg.try_finish(&f.account).is_err());
    assert_eq!(f.reg.owner_account(&key(&f.env, NEW)), None);
    f.env.ledger().set_timestamp(T0 + DELAY);
    f.env.set_auths(&[]); // permissionless: no signature at all
    assert_eq!(f.reg.finish(&f.account), key(&f.env, NEW));
    assert_eq!(f.reg.owner_account(&key(&f.env, NEW)), Some(f.account.clone()));
    // Nothing pending any more.
    assert_eq!(f.reg.try_finish(&f.account), Err(Ok(RecoveryError::NoPendingRecovery.into())));
}
