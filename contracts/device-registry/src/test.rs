//! On the REAL smart-account + verifier WASMs via the real account factory.
extern crate std;
use super::*;
use constella_account_factory::{AccountFactory, AccountFactoryClient as FactoryClient};
use soroban_sdk::testutils::{Address as _, Ledger};

const ACCOUNT_WASM: &[u8] = include_bytes!("../../target/wasm32v1-none/release/constella_smart_account.wasm");
const VERIFIER_WASM: &[u8] = include_bytes!("../../target/wasm32v1-none/release/constella_webauthn_verifier.wasm");
const KEY_A: &str = "046b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c2964fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5";
const KEY_B: &str = "047cf27b188d034f7e8a52380304b51ac3c08969e277f21b35a60b48fc4766997807775510db8ed040293d9ac69f7430dbba7dade63ce982299e04b79d227873d1";

fn key(env: &Env, hex: &str) -> Bytes {
    let raw: std::vec::Vec<u8> = (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect();
    Bytes::from_slice(env, &raw)
}

struct F { env: Env, factory: FactoryClient<'static>, reg: DeviceRegistryClient<'static>, account: Address }

fn setup() -> F {
    let env = Env::default();
    env.cost_estimate().budget().reset_unlimited();
    env.ledger().set_timestamp(1_731_600_000);
    let verifier = env.register(VERIFIER_WASM, ());
    let hash = env.deployer().upload_contract_wasm(ACCOUNT_WASM);
    let factory = FactoryClient::new(&env, &env.register(AccountFactory, (Address::generate(&env), verifier.clone(), hash)));
    let account = factory.create_account(&key(&env, KEY_A));
    let reg = DeviceRegistryClient::new(&env, &env.register(DeviceRegistry, (factory.address.clone(), verifier)));
    F { env, factory, reg, account }
}

#[test]
fn a_device_gets_its_own_rule_on_the_account_and_can_be_found_by_its_key() {
    let f = setup();
    f.env.mock_all_auths();
    let rule = f.reg.add_device(&f.account, &key(&f.env, KEY_B), &String::from_str(&f.env, "phone"));
    let owner_rule: u32 = f.env.invoke_contract(&f.account, &soroban_sdk::Symbol::new(&f.env, "get_owner_rule"), vec![&f.env]);
    assert_ne!(rule, owner_rule, "a SEPARATE rule: the device signs alone");
    let link = f.reg.device_of(&key(&f.env, KEY_B)).unwrap();
    assert_eq!((link.account.clone(), link.rule_id), (f.account.clone(), rule));
    assert_eq!(f.reg.devices_of(&f.account).len(), 1);
    // The rule really exists on the account with the device key as its only signer.
    let r: ContextRule = f.env.invoke_contract(&f.account, &soroban_sdk::Symbol::new(&f.env, "get_context_rule"),
        vec![&f.env, soroban_sdk::IntoVal::into_val(&rule, &f.env)]);
    assert_eq!(r.signers.len(), 1);
}

#[test]
fn only_the_account_itself_can_add_its_devices() {
    let f = setup();
    // No authorization from the account -> refused.
    assert!(f.reg.try_add_device(&f.account, &key(&f.env, KEY_B), &String::from_str(&f.env, "x")).is_err());
    assert!(f.reg.device_of(&key(&f.env, KEY_B)).is_none());
}

#[test]
fn unknown_accounts_reused_keys_and_bad_keys_are_refused() {
    let f = setup();
    f.env.mock_all_auths();
    let name = String::from_str(&f.env, "d");
    let stranger = Address::generate(&f.env);
    assert_eq!(f.reg.try_add_device(&stranger, &key(&f.env, KEY_B), &name), Err(Ok(DeviceError::UnknownAccount.into())));
    // KEY_A already OWNS the account: it cannot also be a device.
    assert_eq!(f.reg.try_add_device(&f.account, &key(&f.env, KEY_A), &name), Err(Ok(DeviceError::KeyInUse.into())));
    f.reg.add_device(&f.account, &key(&f.env, KEY_B), &name);
    assert_eq!(f.reg.try_add_device(&f.account, &key(&f.env, KEY_B), &name), Err(Ok(DeviceError::KeyInUse.into())));
    assert_eq!(f.reg.try_add_device(&f.account, &Bytes::from_slice(&f.env, &[4u8; 33]), &name), Err(Ok(DeviceError::InvalidKey.into())));
    let _ = f.factory;
}
