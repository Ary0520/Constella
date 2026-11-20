#![cfg(test)]
extern crate std;

use super::*;
use soroban_sdk::{
    auth::ContractContext,
    testutils::{Address as _, MockAuth, MockAuthInvoke},
    vec, Bytes, IntoVal, String, Symbol,
};
use stellar_accounts::smart_account::ContextRuleType;

fn signer(env: &Env, verifier: &Address, b: u8) -> Signer {
    Signer::External(verifier.clone(), Bytes::from_array(env, &[b; 65]))
}

fn rule(env: &Env, id: u32, signers: Vec<Signer>) -> ContextRule {
    ContextRule {
        id,
        context_type: ContextRuleType::Default,
        name: String::from_str(env, "recovery"),
        signer_ids: Vec::from_iter(env, 0..signers.len()),
        signers,
        policies: Vec::new(env),
        policy_ids: Vec::new(env),
        valid_until: None,
    }
}

fn ctx(env: &Env) -> Context {
    Context::Contract(ContractContext {
        contract: Address::generate(env),
        fn_name: Symbol::new(env, "add_signer"),
        args: vec![env],
    })
}

struct T {
    env: Env,
    policy: Address,
    client: ThresholdPolicyClient<'static>,
    account: Address,
    signers: Vec<Signer>,
}

fn setup() -> T {
    let env = Env::default();
    let policy = env.register(ThresholdPolicy, ());
    let client = ThresholdPolicyClient::new(&env, &policy);
    let account = Address::generate(&env);
    let verifier = Address::generate(&env);
    let signers = vec![&env, signer(&env, &verifier, 1), signer(&env, &verifier, 2), signer(&env, &verifier, 3)];
    T { env, policy, client, account, signers }
}

/// The account (and only the account) authorizes `fn_name` on the policy.
fn as_account(t: &T, fn_name: &'static str, args: Vec<soroban_sdk::Val>) {
    t.env.mock_auths(&[MockAuth {
        address: &t.account,
        invoke: &MockAuthInvoke { contract: &t.policy, fn_name, args, sub_invokes: &[] },
    }]);
}

fn install(t: &T, threshold: u32) -> ContextRule {
    let r = rule(&t.env, 1, t.signers.clone());
    let params = SimpleThresholdAccountParams { threshold };
    as_account(t, "install", (params.clone(), r.clone(), t.account.clone()).into_val(&t.env));
    t.client.install(&params, &r, &t.account);
    assert_eq!(t.env.auths()[0].0, t.account);
    r
}

#[test]
fn two_of_three_needs_two_authenticated_signers() {
    let t = setup();
    let r = install(&t, 2);
    assert_eq!(t.client.get_threshold(&1, &t.account), 2);
    let c = ctx(&t.env);

    let one = vec![&t.env, t.signers.get(0).unwrap()];
    as_account(&t, "enforce", (c.clone(), one.clone(), r.clone(), t.account.clone()).into_val(&t.env));
    assert!(t.client.try_enforce(&c, &one, &r, &t.account).is_err());

    let two = vec![&t.env, t.signers.get(0).unwrap(), t.signers.get(2).unwrap()];
    as_account(&t, "enforce", (c.clone(), two.clone(), r.clone(), t.account.clone()).into_val(&t.env));
    t.client.enforce(&c, &two, &r, &t.account);
    assert_eq!(t.env.auths()[0].0, t.account);
}

#[test]
fn threshold_must_be_between_one_and_the_rule_signers() {
    let t = setup();
    let r = rule(&t.env, 1, t.signers.clone());
    for bad in [0u32, 4] {
        let params = SimpleThresholdAccountParams { threshold: bad };
        as_account(&t, "install", (params.clone(), r.clone(), t.account.clone()).into_val(&t.env));
        assert!(t.client.try_install(&params, &r, &t.account).is_err(), "threshold {bad} accepted");
    }
    install(&t, 3);
    as_account(&t, "set_threshold", (4u32, r.clone(), t.account.clone()).into_val(&t.env));
    assert!(t.client.try_set_threshold(&4, &r, &t.account).is_err());
    as_account(&t, "set_threshold", (1u32, r.clone(), t.account.clone()).into_val(&t.env));
    t.client.set_threshold(&1, &r, &t.account);
    assert_eq!(t.client.get_threshold(&1, &t.account), 1);
}

#[test]
fn only_the_account_can_change_its_policy() {
    let t = setup();
    let r = install(&t, 2);
    let stranger = Address::generate(&t.env);
    t.env.mock_auths(&[MockAuth {
        address: &stranger,
        invoke: &MockAuthInvoke {
            contract: &t.policy,
            fn_name: "set_threshold",
            args: (1u32, r.clone(), t.account.clone()).into_val(&t.env),
            sub_invokes: &[],
        },
    }]);
    assert!(t.client.try_set_threshold(&1, &r, &t.account).is_err());
    assert_eq!(t.client.get_threshold(&1, &t.account), 2);
}

#[test]
fn state_is_isolated_per_account() {
    let t = setup();
    install(&t, 2);
    let other = Address::generate(&t.env);
    assert!(t.client.try_get_threshold(&1, &other).is_err(), "another account must not inherit a threshold");
}
