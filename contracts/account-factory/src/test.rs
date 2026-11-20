//! Tests on the REAL compiled smart-account and WebAuthn-verifier WASMs
//! (run `stellar contract build` first).
extern crate std;
use super::*;
use soroban_sdk::testutils::{storage::Persistent as _, Address as _, Events as _, Ledger};

const ACCOUNT_WASM: &[u8] = include_bytes!("../../target/wasm32v1-none/release/constella_smart_account.wasm");
const VERIFIER_WASM: &[u8] = include_bytes!("../../target/wasm32v1-none/release/constella_webauthn_verifier.wasm");
/// P-256 generator G and 2G, uncompressed: valid public keys.
const KEY_A: &str = "046b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c2964fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5";
const KEY_B: &str = "047cf27b188d034f7e8a52380304b51ac3c08969e277f21b35a60b48fc4766997807775510db8ed040293d9ac69f7430dbba7dade63ce982299e04b79d227873d1";

fn key(env: &Env, hex: &str) -> Bytes {
    let raw: std::vec::Vec<u8> = (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect();
    Bytes::from_slice(env, &raw)
}

struct F {
    env: Env,
    factory: AccountFactoryClient<'static>,
    admin: Address,
}

fn setup() -> F {
    let env = Env::default();
    env.cost_estimate().budget().reset_unlimited();
    env.ledger().set_timestamp(1_731_600_000);
    let verifier = env.register(VERIFIER_WASM, ());
    let hash = env.deployer().upload_contract_wasm(ACCOUNT_WASM);
    let admin = Address::generate(&env);
    let id = env.register(AccountFactory, (admin.clone(), verifier, hash));
    F { factory: AccountFactoryClient::new(&env, &id), env, admin }
}

#[test]
fn anyone_creates_the_account_at_the_address_its_key_determines() {
    let f = setup();
    let k = key(&f.env, KEY_A);
    let predicted = f.factory.address_for(&k);
    // No auth at all: the relayer (or anyone) can pay for it.
    let account = f.factory.create_account(&k);
    assert!(f.env.auths().is_empty(), "creation needs nobody's signature");
    assert_eq!(account, predicted);
    assert!(f.factory.is_account(&account));
    assert_eq!(f.factory.account_of(&k), Some(account.clone()));
    // A real smart account with its protected owner rule.
    let rule: u32 = f.env.invoke_contract(&account, &soroban_sdk::Symbol::new(&f.env, "get_owner_rule"), soroban_sdk::vec![&f.env]);
    let _ = rule;
}

#[test]
fn a_repeat_returns_the_same_account_and_deploys_nothing() {
    let f = setup();
    let k = key(&f.env, KEY_A);
    let first = f.factory.create_account(&k);
    let again = f.factory.create_account(&k);
    assert_eq!(first, again);
    assert!(f.env.events().all().events().is_empty(), "no second ACCOUNT_CREATED");
}

#[test]
fn different_keys_get_different_accounts_and_bad_keys_are_refused() {
    let f = setup();
    let a = f.factory.create_account(&key(&f.env, KEY_A));
    let b = f.factory.create_account(&key(&f.env, KEY_B));
    assert_ne!(a, b);
    assert!(!f.factory.is_account(&Address::generate(&f.env)));
    let short = Bytes::from_slice(&f.env, &[4u8; 33]);
    assert_eq!(f.factory.try_create_account(&short), Err(Ok(AccountFactoryError::InvalidOwnerKey.into())));
    let mut not_uncompressed = key(&f.env, KEY_A);
    not_uncompressed.set(0, 0x02);
    assert_eq!(f.factory.try_create_account(&not_uncompressed), Err(Ok(AccountFactoryError::InvalidOwnerKey.into())));
}

#[test]
fn records_are_persistent_and_renewed() {
    let f = setup();
    let k = key(&f.env, KEY_A);
    let account = f.factory.create_account(&k);
    f.env.as_contract(&f.factory.address, || {
        let salt: BytesN<32> = f.env.crypto().sha256(&k).into();
        assert!(f.env.storage().persistent().get_ttl(&DataKey::AccountOf(salt)) >= 29 * 17_280);
        assert!(f.env.storage().persistent().get_ttl(&DataKey::IsAccount(account.clone())) >= 29 * 17_280);
    });
}

#[test]
fn account_code_changes_wait_48_hours_and_only_the_admin_proposes() {
    let f = setup();
    let new_hash = BytesN::from_array(&f.env, &[7; 32]);
    assert!(f.factory.try_propose_account_wasm(&new_hash).is_err(), "admin auth required");
    f.env.mock_all_auths();
    f.factory.propose_account_wasm(&new_hash);
    assert_eq!(f.factory.try_activate_account_wasm(), Err(Ok(AccountFactoryError::WasmChangeNotReady.into())));
    f.env.ledger().set_timestamp(1_731_600_000 + WASM_CHANGE_DELAY_SECONDS);
    f.factory.activate_account_wasm();
    assert_eq!(f.factory.get_settings().account_wasm_hash, new_hash);
    let _ = f.admin;
}
