//! Security regression suite for findings in docs/audit/AUDIT-2026-11-15.md.
//!
//! Each test wires the REAL registry, policy, NAV-oracle and vault contracts
//! together and replays the original exploit. A test here must fail on the
//! pre-fix code and pass after the fix. Setup steps may use `mock_all_auths`;
//! the call under test always uses an explicit `mock_auths` tree and asserts
//! who authorized it (AGENT-RULES Rule 13).
#![allow(clippy::inconsistent_digit_grouping)]
extern crate std;

use constella_nav_oracle::{NavOracle, NavOracleClient, OracleSource};
use constella_policy::{PolicyContract, PolicyContractClient};
use constella_registry::{AssetEntry, AssetRegistry, AssetRegistryClient, SleeveType as RegSleeve};
use constella_vault_strategy::{SleeveState, SleeveType, StrategyVault, StrategyVaultClient, VaultConfig};
use soroban_sdk::{
    testutils::{Address as _, Ledger, MockAuth, MockAuthInvoke},
    token, Address, Env, IntoVal, String, Symbol, Vec,
};

const ONE: i128 = 10_000_000; // 7 decimals
const T0: u64 = 1_000_000;

struct Stack<'a> {
    env: Env,
    usdc_admin: token::StellarAssetClient<'a>,
    usdc: token::Client<'a>,
    vault: StrategyVaultClient<'a>,
    vault_id: Address,
    treasury: Address,
    admin: Address,
    usdc_addr: Address,
}

fn oracle_source(env: &Env, id: &str, asset: &Address, price: i128) -> OracleSource {
    OracleSource {
        source_id: Symbol::new(env, id),
        asset: asset.clone(),
        sleeve: Symbol::new(env, "VAULT"),
        price,
        timestamp: T0,
        staleness_limit_seconds: 3600,
        min_liquidity_depth_usd: 100_000,
        current_liquidity_depth_usd: 200_000,
        is_active: true,
    }
}

/// Full contract stack: registry (USDC registered), policy, oracle and a
/// 0-fee strategy vault. With `vault_nav_feeds`, the oracle carries two
/// independent $1.00 feeds for the vault NAV; without, share price is derived
/// from sleeve composition.
fn deploy_stack_with(vault_nav_feeds: bool) -> Stack<'static> {
    let env = Env::default();
    env.mock_all_auths(); // setup only
    env.ledger().set_timestamp(T0);
    env.ledger().set_sequence_number(100);
    let admin = Address::generate(&env);
    let treasury = Address::generate(&env);

    let sac = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let usdc_addr = sac.address();

    let registry_id = env.register(AssetRegistry, ());
    let registry = AssetRegistryClient::new(&env, &registry_id);
    registry.initialize(&admin);
    registry.add_asset(&AssetEntry {
        contract_address: usdc_addr.clone(),
        asset_id: Symbol::new(&env, "USDC"),
        sleeve_type: RegSleeve::Cash,
        liquidity_tier: 0,
        permissionless: true,
        oracle_sources: Vec::from_array(&env, [Symbol::new(&env, "USDC_A"), Symbol::new(&env, "USDC_B")]),
        eligible_jurisdictions: Vec::new(&env),
        blocked_jurisdictions: Vec::new(&env),
        investor_classes_required: Vec::new(&env),
        is_active: true,
        added_at: T0,
    });

    let policy_id = env.register(PolicyContract, ());
    PolicyContractClient::new(&env, &policy_id).initialize(&admin);

    let oracle_id = env.register(NavOracle, ());
    let oracle = NavOracleClient::new(&env, &oracle_id);
    oracle.initialize(&admin, &usdc_addr, &7, &300);
    if vault_nav_feeds {
        oracle.configure_sleeve(&Symbol::new(&env, "VAULT"), &200, &2);
        oracle.register_source(&oracle_source(&env, "USDC_A", &usdc_addr, ONE));
        oracle.register_source(&oracle_source(&env, "USDC_B", &usdc_addr, ONE));
    }

    let vault_id = env.register(StrategyVault, ());
    let vault = StrategyVaultClient::new(&env, &vault_id);
    vault.initialize(
        &VaultConfig {
            admin: admin.clone(),
            pause_authority: Address::generate(&env),
            numeraire: usdc_addr.clone(),
            policy_contract: policy_id,
            oracle_contract: oracle_id,
            registry_contract: registry_id,
            treasury: treasury.clone(),
            management_fee_bps: 0,
        },
        &String::from_str(&env, "Constella Strategy Vault"),
        &String::from_str(&env, "cVAULT"),
        &7,
    );

    Stack {
        usdc_admin: token::StellarAssetClient::new(&env, &usdc_addr),
        usdc: token::Client::new(&env, &usdc_addr),
        env,
        vault,
        vault_id,
        treasury,
        admin,
        usdc_addr,
    }
}

fn deploy_stack() -> Stack<'static> {
    deploy_stack_with(true)
}

fn authorize(s: &Stack, who: &Address, fn_name: &'static str, args: soroban_sdk::Vec<soroban_sdk::Val>) {
    s.env.mock_auths(&[MockAuth {
        address: who,
        invoke: &MockAuthInvoke { contract: &s.vault_id, fn_name, args, sub_invokes: &[] },
    }]);
}

// ---------------------------------------------------------------------------
// V1 — self-transfer minted shares (attacker drained other depositors)
// ---------------------------------------------------------------------------
#[test]
fn v1_self_transfer_cannot_inflate_shares_or_drain_vault() {
    let s = deploy_stack();
    let attacker = Address::generate(&s.env);
    let victim = Address::generate(&s.env);
    s.usdc_admin.mint(&attacker, &(100 * ONE));
    s.usdc_admin.mint(&victim, &(900 * ONE));
    s.vault.deposit(&(100 * ONE), &attacker);
    s.vault.deposit(&(900 * ONE), &victim);

    // Original exploit: repeatedly transfer the full balance to oneself.
    for _ in 0..3 {
        let bal = s.vault.balance(&attacker);
        authorize(&s, &attacker, "transfer", (attacker.clone(), attacker.clone(), bal).into_val(&s.env));
        s.vault.transfer(&attacker, &attacker, &bal);
        assert_eq!(s.env.auths()[0].0, attacker);
    }

    assert_eq!(s.vault.balance(&attacker), 100 * ONE);
    assert_eq!(
        s.vault.balance(&attacker) + s.vault.balance(&victim) + s.vault.balance(&s.treasury),
        s.vault.total_supply(),
        "sum of balances must equal total supply"
    );

    // Attacker can only get back what they put in; the victim is whole.
    s.env.mock_all_auths();
    let out = s.vault.redeem(&s.vault.balance(&attacker), &attacker, &attacker);
    assert!(out <= 100 * ONE, "attacker redeemed {out}, deposited {}", 100 * ONE);
    assert_eq!(s.vault.balance(&victim), 900 * ONE);
    assert!(s.usdc.balance(&s.vault_id) >= 900 * ONE * 99 / 100);
}

#[test]
fn v1_self_transfer_from_cannot_inflate_shares() {
    let s = deploy_stack();
    let owner = Address::generate(&s.env);
    let spender = Address::generate(&s.env);
    s.usdc_admin.mint(&owner, &(50 * ONE));
    s.vault.deposit(&(50 * ONE), &owner);
    s.vault.approve(&owner, &spender, &(50 * ONE), &10_000);

    authorize(&s, &spender, "transfer_from", (spender.clone(), owner.clone(), owner.clone(), 50 * ONE).into_val(&s.env));
    s.vault.transfer_from(&spender, &owner, &owner, &(50 * ONE));
    assert_eq!(s.env.auths()[0].0, spender);

    assert_eq!(s.vault.balance(&owner), 50 * ONE);
    assert_eq!(s.vault.total_supply(), 50 * ONE);
}

// ---------------------------------------------------------------------------
// V2 — admin overwrote the cash sleeve's book balance, inflating NAV, then
// redeemed a sliver of its own shares for depositors' real USDC.
// ---------------------------------------------------------------------------
#[test]
fn v2_admin_cannot_inject_sleeve_balance_to_drain_depositors() {
    let s = deploy_stack_with(false); // NAV from sleeve composition
    let alice = Address::generate(&s.env);
    s.usdc_admin.mint(&alice, &(1000 * ONE));
    s.usdc_admin.mint(&s.admin, &ONE);
    s.vault.deposit(&(1000 * ONE), &alice);
    s.vault.deposit(&ONE, &s.admin);
    let admin_usdc_before = s.usdc.balance(&s.admin);

    // Attack step 1: admin rewrites the cash sleeve's recorded balance.
    let mut forged = Vec::new(&s.env);
    forged.push_back(SleeveState {
        asset_id: String::from_str(&s.env, "USDC"),
        contract_address: s.usdc_addr.clone(),
        target_weight_bps: 10_000,
        band_bps: 0,
        current_balance: 1_001_000_000 * ONE,
        sleeve_type: SleeveType::CashStable,
        liquidity_tier: 0,
    });
    s.env.mock_auths(&[MockAuth {
        address: &s.admin,
        invoke: &MockAuthInvoke {
            contract: &s.vault_id,
            fn_name: "set_sleeves",
            args: (forged.clone(),).into_val(&s.env),
            sub_invokes: &[],
        },
    }]);
    assert!(s.vault.try_set_sleeves(&forged).is_err(), "forged balance must be rejected");

    // Attack step 2: redeem a sliver of shares for 1000 USDC.
    s.env.mock_all_auths();
    let sliver = s.vault.convert_to_shares(&(1000 * ONE));
    let _ = s.vault.try_redeem(&sliver.min(s.vault.balance(&s.admin)), &s.admin, &s.admin);
    let _ = s.vault.try_redeem(&s.vault.balance(&s.admin), &s.admin, &s.admin);

    let admin_gain = s.usdc.balance(&s.admin) - admin_usdc_before;
    assert!(admin_gain <= ONE, "admin extracted {admin_gain} after depositing {ONE}");
    let alice_out = s.vault.redeem(&s.vault.balance(&alice), &alice, &alice);
    assert_eq!(alice_out, 1000 * ONE);
}
