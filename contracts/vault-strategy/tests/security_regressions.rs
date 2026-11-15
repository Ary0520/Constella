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
use constella_policy::{PolicyContract, PolicyContractClient, SleeveTarget};
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
    policy: PolicyContractClient<'a>,
    registry_id: Address,
    oracle: NavOracleClient<'a>,
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
    let policy = PolicyContractClient::new(&env, &policy_id);
    policy.initialize(&admin);

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
            oracle_contract: oracle_id.clone(),
            registry_contract: registry_id.clone(),
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
        policy,
        registry_id,
        oracle,
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

// ---------------------------------------------------------------------------
// P3 — the "signed" policy could be registered and replaced by the protocol
// admin alone; the user never signed. AGENT-RULES Rule 5 / THESIS-ENGINE
// Step 6: only the policy owner (user / treasury multisig) may sign or amend.
// ---------------------------------------------------------------------------
fn cash_only_targets(env: &Env) -> Vec<SleeveTarget> {
    Vec::from_array(env, [SleeveTarget {
        asset_id: Symbol::new(env, "USDC"),
        target_weight_bps: 10_000,
        min_band_bps: 9_000,
        max_band_bps: 10_000,
        sleeve_type: Symbol::new(env, "cash"),
    }])
}

#[test]
fn p3_admin_alone_cannot_replace_a_signed_policy() {
    let s = deploy_stack();
    let signed_hash = soroban_sdk::BytesN::from_array(&s.env, &[0x51; 32]);
    let forged_hash = soroban_sdk::BytesN::from_array(&s.env, &[0xBA; 32]);
    let empty: Vec<SleeveTarget> = Vec::new(&s.env);

    let owner = Address::generate(&s.env); // the user's / treasury's signing account

    s.env.mock_all_auths(); // setup: owner + vault registered the signed policy
    s.policy.register_policy(
        &s.vault_id, &owner, &signed_hash, &s.registry_id,
        &1000, &10_000, &10_000, &50, &500, &cash_only_targets(&s.env),
    );

    // Attack: ONLY the protocol admin authorizes a replacement that wipes all
    // bands and drops the liquidity floor to 0.
    s.env.mock_auths(&[MockAuth {
        address: &s.admin,
        invoke: &MockAuthInvoke {
            contract: &s.policy.address,
            fn_name: "update_policy",
            args: (s.vault_id.clone(), forged_hash.clone(), 0u32, 10_000u32, 10_000u32, 10_000u32, 10_000u32, empty.clone())
                .into_val(&s.env),
            sub_invokes: &[],
        },
    }]);
    let res = s.policy.try_update_policy(&s.vault_id, &forged_hash, &0, &10_000, &10_000, &10_000, &10_000, &empty);

    assert!(res.is_err(), "admin alone must not be able to amend a signed policy");
    assert_eq!(s.policy.get_policy_hash(&s.vault_id), signed_hash);
    assert_eq!(s.policy.get_sleeve_targets(&s.vault_id).len(), 1);
    assert_eq!(s.policy.get_policy_config(&s.vault_id).unwrap().min_liquidity_sleeve_bps, 1000);

    // Attack 2: admin tries to register a fresh policy over the existing one.
    s.env.mock_all_auths();
    let squat = s.policy.try_register_policy(
        &s.vault_id, &s.admin, &forged_hash, &s.registry_id,
        &0, &10_000, &10_000, &10_000, &10_000, &empty,
    );
    assert!(squat.is_err(), "a signed policy cannot be re-registered");
    assert_eq!(s.policy.get_policy_config(&s.vault_id).unwrap().owner, owner);

    // The legitimate owner can still amend by re-signing.
    let amended = soroban_sdk::BytesN::from_array(&s.env, &[0x52; 32]);
    let targets = cash_only_targets(&s.env);
    s.env.mock_auths(&[MockAuth {
        address: &owner,
        invoke: &MockAuthInvoke {
            contract: &s.policy.address,
            fn_name: "update_policy",
            args: (s.vault_id.clone(), amended.clone(), 1500u32, 10_000u32, 10_000u32, 50u32, 500u32, targets.clone())
                .into_val(&s.env),
            sub_invokes: &[],
        },
    }]);
    s.policy.update_policy(&s.vault_id, &amended, &1500, &10_000, &10_000, &50, &500, &targets);
    assert_eq!(s.env.auths()[0].0, owner);
    assert_eq!(s.policy.get_policy_hash(&s.vault_id), amended);
}

// ---------------------------------------------------------------------------
// O1 — trigger_circuit_breaker had no authorization: anyone could pause any
// sleeve (griefing / denial of service). CONTRACTS-SPEC "Security Properties":
// pause is single-sig *pause authority*; unpause requires the admin multisig.
// ---------------------------------------------------------------------------
#[test]
fn o1_stranger_cannot_trip_a_circuit_breaker() {
    let s = deploy_stack(); // vault NAV priced by the VAULT oracle sleeve
    let vault_sleeve = Symbol::new(&s.env, "VAULT");
    let user = Address::generate(&s.env);
    s.usdc_admin.mint(&user, &(100 * ONE));

    let griefer = Address::generate(&s.env);
    let reason = Symbol::new(&s.env, "GRIEF");

    // Attack 1: no signatures at all, impersonating the admin as caller.
    s.env.set_auths(&[]);
    let res = s.oracle.try_trigger_circuit_breaker(&s.admin, &vault_sleeve, &reason);
    assert!(res.is_err(), "an unsigned call must not be able to pause a sleeve");

    // Attack 2: the griefer signs, but holds no pause role.
    s.env.mock_auths(&[MockAuth {
        address: &griefer,
        invoke: &MockAuthInvoke {
            contract: &s.oracle.address,
            fn_name: "trigger_circuit_breaker",
            args: (griefer.clone(), vault_sleeve.clone(), reason.clone()).into_val(&s.env),
            sub_invokes: &[],
        },
    }]);
    let res = s.oracle.try_trigger_circuit_breaker(&griefer, &vault_sleeve, &reason);
    assert!(res.is_err(), "an unauthorized caller must not be able to pause a sleeve");
    assert!(!s.oracle.is_sleeve_paused(&vault_sleeve));

    // The vault keeps working for honest users.
    s.env.mock_all_auths();
    assert_eq!(s.vault.deposit(&(100 * ONE), &user), 100 * ONE);
}

// ---------------------------------------------------------------------------
// V3 — pausing one sleeve froze the WHOLE vault (deposits and cash-only
// redemptions). NAV-ORACLE Rules 4/5: unaffected sleeves continue normally.
// ---------------------------------------------------------------------------
#[test]
fn v3_pausing_an_empty_sleeve_does_not_freeze_the_vault() {
    let s = deploy_stack_with(false); // NAV from sleeve composition
    let registry = AssetRegistryClient::new(&s.env, &s.registry_id);
    let gov = s.env.register_stellar_asset_contract_v2(Address::generate(&s.env)).address();
    let gov_sleeve = Symbol::new(&s.env, "USD_GOV_FUNDS");
    registry.add_asset(&AssetEntry {
        contract_address: gov.clone(),
        asset_id: Symbol::new(&s.env, "SPIKO_USD"),
        sleeve_type: RegSleeve::UsdGovFund,
        liquidity_tier: 1,
        permissionless: true,
        oracle_sources: Vec::from_array(&s.env, [Symbol::new(&s.env, "SPIKO_NAV"), Symbol::new(&s.env, "SPIKO_RFL")]),
        eligible_jurisdictions: Vec::new(&s.env),
        blocked_jurisdictions: Vec::new(&s.env),
        investor_classes_required: Vec::new(&s.env),
        is_active: true,
        added_at: T0,
    });
    s.oracle.configure_sleeve(&gov_sleeve, &200, &2);
    for id in ["SPIKO_NAV", "SPIKO_RFL"] {
        let mut src = oracle_source(&s.env, id, &gov, ONE);
        src.sleeve = gov_sleeve.clone();
        s.oracle.register_source(&src);
    }

    let alice = Address::generate(&s.env);
    s.usdc_admin.mint(&alice, &(1000 * ONE));
    s.vault.deposit(&(500 * ONE), &alice);
    let mut sleeves = Vec::new(&s.env);
    sleeves.push_back(SleeveState {
        asset_id: String::from_str(&s.env, "USDC"),
        contract_address: s.usdc_addr.clone(),
        target_weight_bps: 5000,
        band_bps: 500,
        current_balance: 500 * ONE,
        sleeve_type: SleeveType::CashStable,
        liquidity_tier: 0,
    });
    sleeves.push_back(SleeveState {
        asset_id: String::from_str(&s.env, "SPIKO_USD"),
        contract_address: gov,
        target_weight_bps: 5000,
        band_bps: 500,
        current_balance: 0,
        sleeve_type: SleeveType::UsdGovFunds,
        liquidity_tier: 1,
    });
    s.vault.set_sleeves(&sleeves);

    // The authorized pause authority (admin by default) halts the gov sleeve.
    s.oracle.trigger_circuit_breaker(&s.admin, &gov_sleeve, &Symbol::new(&s.env, "ISSUER_FREEZE"));
    assert!(s.oracle.is_sleeve_paused(&gov_sleeve));

    // Original PoC: both of these failed. The cash sleeve must keep working.
    assert!(s.vault.try_deposit(&(100 * ONE), &alice).is_ok(), "deposit blocked by an unrelated empty sleeve");
    assert!(s.vault.try_redeem(&(10 * ONE), &alice, &alice).is_ok(), "redemption blocked by an unrelated empty sleeve");
    assert_eq!(s.usdc.balance(&alice), 410 * ONE);
}

// ---------------------------------------------------------------------------
// O4 — a deviation breaker tripped only when someone READ a price. If that
// read happened inside a vault call that then reverted, the pause and its
// NAV_PAUSE event were rolled back with it. NAV-ORACLE: every circuit-breaker
// event MUST be emitted on-chain. Deviation must latch when prices change.
// ---------------------------------------------------------------------------
#[test]
fn o4_deviation_pause_latches_when_the_price_is_pushed() {
    let s = deploy_stack(); // VAULT sleeve priced by USDC_A and USDC_B at $1.00
    let vault_sleeve = Symbol::new(&s.env, "VAULT");
    let alice = Address::generate(&s.env);
    s.usdc_admin.mint(&alice, &(100 * ONE));
    s.vault.deposit(&(100 * ONE), &alice);

    // A feed diverges 10% (threshold 2%). Nobody reads a price yet.
    s.oracle.update_price(&Symbol::new(&s.env, "USDC_B"), &(ONE * 11 / 10), &200_000, &T0);
    assert!(
        s.oracle.is_sleeve_paused(&vault_sleeve),
        "deviation must latch in the same successful call that pushed the price"
    );
    assert_eq!(s.oracle.get_sleeve_config(&vault_sleeve).unwrap().pause_reason, Symbol::new(&s.env, "DEVIATION"));

    // A vault call that reverts afterwards cannot undo the pause.
    assert!(s.vault.try_redeem(&(1_000 * ONE), &alice, &alice).is_err());
    assert!(s.oracle.is_sleeve_paused(&vault_sleeve));
    assert!(s.oracle.get_deposit_nav(&vault_sleeve).is_none());
}

// ---------------------------------------------------------------------------
// O2 — update_price accepted any timestamp; a feed stamped in the future
// never became stale (NAV-ORACLE Rule 5: "a 24-hour-old price is not a price").
// ---------------------------------------------------------------------------
#[test]
fn o2_future_dated_price_is_rejected_and_cannot_dodge_staleness() {
    let s = deploy_stack(); // VAULT sleeve: USDC_A, USDC_B, 1h staleness limit
    let vault_sleeve = Symbol::new(&s.env, "VAULT");
    let far_future = T0 + 10_000_000;

    for id in ["USDC_A", "USDC_B"] {
        let res = s.oracle.try_update_price(&Symbol::new(&s.env, id), &ONE, &200_000, &far_future);
        assert!(res.is_err(), "future-dated price must be rejected");
    }

    // ~58 days later the original T0 prices are stale: pricing must hard-stop.
    s.env.ledger().set_timestamp(T0 + 5_000_000);
    assert!(s.oracle.get_deposit_nav(&vault_sleeve).is_none(), "stale feeds must not price the vault");
    assert!(s.oracle.get_collateral_nav(&vault_sleeve).is_none());
}

// ---------------------------------------------------------------------------
// O3 — a sleeve could be configured with min_valid_sources = 1, so a single
// thin feed (the YieldBlox pattern: $1 -> $107) could price the vault.
// NAV-ORACLE Rule 5 / AGENT-RULES Rule 3: fewer than 2 valid sources = hard stop.
// ---------------------------------------------------------------------------
#[test]
fn o3_single_source_configuration_is_refused() {
    let s = deploy_stack();
    let xlm_sleeve = Symbol::new(&s.env, "XLM");
    let res = s.oracle.try_configure_sleeve(&xlm_sleeve, &200, &1);
    assert!(res.is_err(), "min_valid_sources = 1 must be refused");

    // Even with only one feed registered, that feed alone can never price.
    let asset = Address::generate(&s.env);
    let mut src = oracle_source(&s.env, "ONLY_FEED", &asset, 107 * ONE);
    src.sleeve = xlm_sleeve.clone();
    s.oracle.register_source(&src);
    assert!(s.oracle.price(&asset).is_none(), "a single source must not produce a price");
    assert!(s.oracle.get_collateral_nav(&xlm_sleeve).is_none());
}
