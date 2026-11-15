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
use constella_policy::{
    PolicyContract, PolicyContractClient, RebalanceProposal, SleeveTarget, TradeOperation, TradeRoute,
};
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
/// 0-fee strategy vault. With `vault_nav_feeds`, the oracle also carries two
/// $1.00 feeds under the key "VAULT", used by the oracle-focused tests. Since
/// audit V5 the vault never reads that key: share prices always come from
/// holdings.
fn deploy_stack_with(vault_nav_feeds: bool) -> Stack<'static> {
    let env = Env::default();
    env.mock_all_auths(); // setup only
    env.ledger().set_timestamp(T0);
    env.ledger().set_sequence_number(100);
    let admin = Address::generate(&env);
    let treasury = Address::generate(&env);

    let sac = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let usdc_addr = sac.address();

    let registry_id = env.register(AssetRegistry, (admin.clone(),));
    let registry = AssetRegistryClient::new(&env, &registry_id);
    registry.add_asset(&AssetEntry {
        contract_address: usdc_addr.clone(),
        asset_id: Symbol::new(&env, "USDC"),
        issuer: Symbol::new(&env, "CIRCLE"),
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

    let oracle_id = env.register(NavOracle, ());
    let oracle = NavOracleClient::new(&env, &oracle_id);
    oracle.initialize(&admin, &usdc_addr, &7, &300);
    policy.initialize(&admin, &oracle_id);
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
            tvl_cap: 1_000_000 * 10_000_000, // FIXTURE: ROADMAP 3.2 $1M beta cap
            per_user_cap: 100_000 * 10_000_000, // FIXTURE: design-partner ticket size (GTM)
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
    // Audit P4: the amendment applies only after the notice period.
    s.env.ledger().set_timestamp(T0 + constella_policy::PROTOCOL_MIN_NOTICE_SECONDS);
    s.policy.activate_policy_update(&s.vault_id);
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
        issuer: Symbol::new(&s.env, "SPIKO"),
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

// ---------------------------------------------------------------------------
// P1 — slippage was checked as min_amount_out >= amount_in * (1 - cap) in RAW
// token units, ignoring the exchange rate. With EUR/USD = 1.08 a fair
// USDC->EURC trade was rejected and a 7.4%-loss EURC->USDC trade accepted.
// EXECUTION.md: "estimated slippage <= cap per route" (value-based).
// ---------------------------------------------------------------------------
fn usdc_eurc_policy(s: &Stack) -> Address {
    let registry = AssetRegistryClient::new(&s.env, &s.registry_id);
    let eurc = s.env.register_stellar_asset_contract_v2(Address::generate(&s.env)).address();
    registry.add_asset(&AssetEntry {
        contract_address: eurc.clone(),
        asset_id: Symbol::new(&s.env, "EURC"),
        issuer: Symbol::new(&s.env, "CIRCLE"),
        sleeve_type: RegSleeve::Cash,
        liquidity_tier: 0,
        permissionless: true,
        oracle_sources: Vec::from_array(&s.env, [Symbol::new(&s.env, "EURC_A"), Symbol::new(&s.env, "EURC_B")]),
        eligible_jurisdictions: Vec::new(&s.env),
        blocked_jurisdictions: Vec::new(&s.env),
        investor_classes_required: Vec::new(&s.env),
        is_active: true,
        added_at: T0,
    });
    // EUR priced in USDC: two independent feeds at 1.08 under its own key.
    let eurc_key = Symbol::new(&s.env, "EURC");
    s.oracle.configure_sleeve(&eurc_key, &200, &2);
    for id in ["EURC_A", "EURC_B"] {
        let mut src = oracle_source(&s.env, id, &eurc, ONE * 108 / 100);
        src.sleeve = eurc_key.clone();
        s.oracle.register_source(&src);
    }
    let mut targets = Vec::new(&s.env);
    for id in ["USDC", "EURC"] {
        targets.push_back(SleeveTarget {
            asset_id: Symbol::new(&s.env, id),
            target_weight_bps: 5000,
            min_band_bps: 0,
            max_band_bps: 10_000,
            sleeve_type: Symbol::new(&s.env, "cash"),
        });
    }
    let owner = Address::generate(&s.env);
    s.policy.register_policy(
        &s.vault_id, &owner, &soroban_sdk::BytesN::from_array(&s.env, &[0x77; 32]), &s.registry_id,
        &1000, &10_000, &10_000, &50, &500, &targets,
    );
    eurc
}

fn one_trade(s: &Stack, a_in: &Address, a_out: &Address, amount_in: i128, min_out: i128) -> RebalanceProposal {
    let mut w = Vec::new(&s.env);
    w.push_back((Symbol::new(&s.env, "USDC"), 5000u32));
    w.push_back((Symbol::new(&s.env, "EURC"), 5000u32));
    RebalanceProposal {
        vault: s.vault_id.clone(),
        policy_hash: s.policy.get_policy_hash(&s.vault_id),
        operations: Vec::from_array(&s.env, [TradeOperation {
            asset_in: a_in.clone(),
            asset_out: a_out.clone(),
            amount_in,
            min_amount_out: min_out,
            route: TradeRoute::Soroswap,
        }]),
        weights_after: w,
        deadline: T0 + 600,
        proposer: s.admin.clone(),
    }
}

#[test]
fn p1_slippage_is_measured_in_value_not_token_units() {
    let s = deploy_stack();
    let eurc = usdc_eurc_policy(&s);

    // 1000 USDC is worth 925.93 EURC at 1.08; min_out 925 = ~0.1% slippage (cap 0.5%).
    let fair = s.policy.validate_rebalance(&s.vault_id, &one_trade(&s, &s.usdc_addr, &eurc, 1000 * ONE, 925 * ONE));
    assert!(fair.is_valid, "fair USDC->EURC trade must be accepted (code {})", fair.code);

    // 1000 EURC is worth 1080 USDC; accepting 1000 USDC = 7.4% loss.
    let bad = s.policy.validate_rebalance(&s.vault_id, &one_trade(&s, &eurc, &s.usdc_addr, 1000 * ONE, 1000 * ONE));
    assert!(!bad.is_valid, "7.4%-loss EURC->USDC trade must be rejected");
}

// ---------------------------------------------------------------------------
// P2 — the issuer cap inferred the issuer from the asset_id prefix, so USDC
// and EURC (both Circle) counted as different issuers and Circle could exceed
// its cap. REGISTRY.md models the issuer per asset; the cap must use it.
// ---------------------------------------------------------------------------
#[test]
fn p2_issuer_cap_counts_all_assets_of_the_same_issuer() {
    let s = deploy_stack();
    let eurc = usdc_eurc_policy_with_caps(&s, 4000); // issuer cap 40%
    let _ = eurc;
    let mut w = Vec::new(&s.env);
    w.push_back((Symbol::new(&s.env, "USDC"), 3000u32)); // Circle
    w.push_back((Symbol::new(&s.env, "EURC"), 3000u32)); // Circle
    w.push_back((Symbol::new(&s.env, "BLEND_USDC_SUPPLY"), 4000u32)); // Blend
    let proposal = RebalanceProposal {
        vault: s.vault_id.clone(),
        policy_hash: s.policy.get_policy_hash(&s.vault_id),
        operations: Vec::new(&s.env),
        weights_after: w,
        deadline: T0 + 600,
        proposer: s.admin.clone(),
    };
    let r = s.policy.validate_rebalance(&s.vault_id, &proposal);
    assert!(!r.is_valid, "Circle at 60% must breach a 40% issuer cap");
    assert_eq!(r.message, Symbol::new(&s.env, "ISSUER_CAP_EXCEEDED"));
}

fn usdc_eurc_policy_with_caps(s: &Stack, max_issuer_bps: u32) -> Address {
    let registry = AssetRegistryClient::new(&s.env, &s.registry_id);
    let mut eurc = Address::generate(&s.env);
    for (id, st) in [("EURC", RegSleeve::Cash), ("BLEND_USDC_SUPPLY", RegSleeve::Lending)] {
        let addr = s.env.register_stellar_asset_contract_v2(Address::generate(&s.env)).address();
        if id == "EURC" {
            eurc = addr.clone();
        }
        registry.add_asset(&AssetEntry {
            contract_address: addr,
            asset_id: Symbol::new(&s.env, id),
            issuer: Symbol::new(&s.env, if id == "EURC" { "CIRCLE" } else { "BLEND" }),
            sleeve_type: st,
            liquidity_tier: 0,
            permissionless: true,
            oracle_sources: Vec::from_array(&s.env, [Symbol::new(&s.env, "SRC_A"), Symbol::new(&s.env, "SRC_B")]),
            eligible_jurisdictions: Vec::new(&s.env),
            blocked_jurisdictions: Vec::new(&s.env),
            investor_classes_required: Vec::new(&s.env),
            is_active: true,
            added_at: T0,
        });
    }
    let mut targets = Vec::new(&s.env);
    for (id, st) in [("USDC", "cash"), ("EURC", "cash"), ("BLEND_USDC_SUPPLY", "lending")] {
        targets.push_back(SleeveTarget {
            asset_id: Symbol::new(&s.env, id),
            target_weight_bps: 3333,
            min_band_bps: 0,
            max_band_bps: 10_000,
            sleeve_type: Symbol::new(&s.env, st),
        });
    }
    s.policy.register_policy(
        &s.vault_id, &Address::generate(&s.env), &soroban_sdk::BytesN::from_array(&s.env, &[0x22; 32]),
        &s.registry_id, &1000, &10_000, &max_issuer_bps, &50, &500, &targets,
    );
    eurc
}

// ---------------------------------------------------------------------------
// M1a — a rebalance could BUY an asset that is not in the signed policy, as
// long as the claimed weights_after omitted it. EXECUTION.md: "no excluded
// asset is bought". The signed sleeve list is the allow-list.
// ---------------------------------------------------------------------------
#[test]
fn m1_rebalance_cannot_buy_an_asset_outside_the_signed_policy() {
    let s = deploy_stack();
    let _eurc = usdc_eurc_policy(&s); // signed sleeves: USDC, EURC
    // A registered, properly priced asset the owner did NOT sign up for.
    let registry = AssetRegistryClient::new(&s.env, &s.registry_id);
    let jqqq = s.env.register_stellar_asset_contract_v2(Address::generate(&s.env)).address();
    registry.add_asset(&AssetEntry {
        contract_address: jqqq.clone(),
        asset_id: Symbol::new(&s.env, "JH_JQQQ"),
        issuer: Symbol::new(&s.env, "JANUS_HENDERSON"),
        sleeve_type: RegSleeve::Equity,
        liquidity_tier: 2,
        permissionless: true,
        oracle_sources: Vec::from_array(&s.env, [Symbol::new(&s.env, "JQQQ_A"), Symbol::new(&s.env, "JQQQ_B")]),
        eligible_jurisdictions: Vec::new(&s.env),
        blocked_jurisdictions: Vec::new(&s.env),
        investor_classes_required: Vec::new(&s.env),
        is_active: true,
        added_at: T0,
    });
    let key = Symbol::new(&s.env, "JQQQ");
    s.oracle.configure_sleeve(&key, &200, &2);
    for id in ["JQQQ_A", "JQQQ_B"] {
        let mut src = oracle_source(&s.env, id, &jqqq, 50 * ONE);
        src.sleeve = key.clone();
        s.oracle.register_source(&src);
    }

    // Fairly priced trade into JQQQ (1000 USDC -> 19.95 JQQQ at $50, 0.25% slippage).
    let r = s.policy.validate_rebalance(&s.vault_id, &one_trade(&s, &s.usdc_addr, &jqqq, 1000 * ONE, 1995 * ONE / 100));
    assert!(!r.is_valid, "buying an asset outside the signed policy must be rejected");
    assert_eq!(r.message, Symbol::new(&s.env, "ASSET_NOT_IN_POLICY"));
}

// ---------------------------------------------------------------------------
// P4 (owner decision D-E) — an owner's amendment took effect immediately for
// every depositor of a shared vault. Amendments must wait a notice period so
// holders can exit under the terms they signed up for.
// ---------------------------------------------------------------------------
#[test]
fn p4_amendment_does_not_take_effect_before_the_notice_period() {
    let s = deploy_stack();
    let owner = Address::generate(&s.env);
    let signed = soroban_sdk::BytesN::from_array(&s.env, &[0x41; 32]);
    let amended = soroban_sdk::BytesN::from_array(&s.env, &[0x42; 32]);
    let targets = cash_only_targets(&s.env);
    s.env.mock_all_auths();
    s.policy.register_policy(
        &s.vault_id, &owner, &signed, &s.registry_id, &1000, &10_000, &10_000, &50, &500, &targets,
    );

    // Owner signs an amendment that drops the liquidity floor to 0.
    s.policy.update_policy(&s.vault_id, &amended, &0, &10_000, &10_000, &50, &500, &targets);

    // Depositors are still protected by the terms they joined under.
    assert_eq!(s.policy.get_policy_hash(&s.vault_id), signed, "amendment must not apply immediately");
    assert_eq!(s.policy.get_policy_config(&s.vault_id).unwrap().min_liquidity_sleeve_bps, 1000);

    // Holders can still exit under the old terms during the notice period.
    let holder = Address::generate(&s.env);
    s.usdc_admin.mint(&holder, &(100 * ONE));
    s.vault.deposit(&(100 * ONE), &holder);
    s.env.ledger().set_timestamp(T0 + constella_policy::PROTOCOL_MIN_NOTICE_SECONDS - 1);
    assert!(s.policy.try_activate_policy_update(&s.vault_id).is_err(), "one second early");
    let out = s.vault.redeem(&(100 * ONE), &holder, &holder);
    assert_eq!(out, 100 * ONE, "holder exits in full under the old terms during the notice period");

    // After the notice period anyone may finalize it - no signatures needed.
    s.env.ledger().set_timestamp(T0 + constella_policy::PROTOCOL_MIN_NOTICE_SECONDS);
    s.env.set_auths(&[]);
    s.policy.activate_policy_update(&s.vault_id);
    assert_eq!(s.policy.get_policy_hash(&s.vault_id), amended);
    assert_eq!(s.policy.get_policy_config(&s.vault_id).unwrap().min_liquidity_sleeve_bps, 0);
}

// ---------------------------------------------------------------------------
// W3 (caps) — the vault accepted unlimited deposits. AGENT-RULES Rule 6: all
// new contracts start with TVL caps and per-user caps; ROADMAP 3.2: $1M TVL cap.
// ---------------------------------------------------------------------------
#[test]
fn w3_deposits_are_capped() {
    let s = deploy_stack();
    let whale = Address::generate(&s.env);
    let huge = 10_000_000 * ONE; // $10M
    s.usdc_admin.mint(&whale, &huge);

    assert!(s.vault.max_deposit(&whale) < i128::MAX, "max_deposit must report a finite cap");
    assert!(s.vault.try_deposit(&huge, &whale).is_err(), "a $10M deposit must exceed the caps");
    assert_eq!(s.usdc.balance(&whale), huge, "rejected deposit moves no funds");
}

// ---------------------------------------------------------------------------
// W7 — the Collateral Passport hard-coded a 2% haircut (98% LTV) for every
// basket and reported every address as an eligible liquidator.
// COLLATERAL-PASSPORT Pillar 3 (haircut from composition) and Pillar 4 /
// VAULT-DESIGN (permissioned constituents need compliant liquidators).
// ---------------------------------------------------------------------------
#[test]
fn w7_passport_is_derived_from_the_basket_not_hard_coded() {
    let s = deploy_stack_with(false);
    let alice = Address::generate(&s.env);
    s.usdc_admin.mint(&alice, &(1000 * ONE));
    s.vault.deposit(&(1000 * ONE), &alice);

    // A 100% USDC basket is a "High-Cash Basket": 10% haircut, 90% LTV.
    let sheet = s.vault.get_risk_sheet();
    assert_eq!(sheet.recommended_haircut_bps, 1000, "cash basket haircut per Pillar 3");
    assert_eq!(sheet.recommended_ltv_bps, 9000);

    // Add a permissioned constituent (KYC'd fund token): random addresses are
    // no longer eligible liquidators.
    let registry = AssetRegistryClient::new(&s.env, &s.registry_id);
    let benji = s.env.register_stellar_asset_contract_v2(Address::generate(&s.env)).address();
    registry.add_asset(&AssetEntry {
        contract_address: benji.clone(),
        asset_id: Symbol::new(&s.env, "FRANKLIN_BENJI"),
        issuer: Symbol::new(&s.env, "FRANKLIN_TEMPLETON"),
        sleeve_type: RegSleeve::UsdGovFund,
        liquidity_tier: 1,
        permissionless: false,
        oracle_sources: Vec::from_array(&s.env, [Symbol::new(&s.env, "BENJI_A"), Symbol::new(&s.env, "BENJI_B")]),
        eligible_jurisdictions: Vec::new(&s.env),
        blocked_jurisdictions: Vec::new(&s.env),
        investor_classes_required: Vec::new(&s.env),
        is_active: true,
        added_at: T0,
    });
    let mut sleeves = Vec::new(&s.env);
    sleeves.push_back(SleeveState {
        asset_id: String::from_str(&s.env, "USDC"),
        contract_address: s.usdc_addr.clone(),
        target_weight_bps: 5000,
        band_bps: 500,
        current_balance: 1000 * ONE,
        sleeve_type: SleeveType::CashStable,
        liquidity_tier: 0,
    });
    sleeves.push_back(SleeveState {
        asset_id: String::from_str(&s.env, "FRANKLIN_BENJI"),
        contract_address: benji,
        target_weight_bps: 5000,
        band_bps: 500,
        current_balance: 0,
        sleeve_type: SleeveType::UsdGovFunds,
        liquidity_tier: 1,
    });
    s.vault.set_sleeves(&sleeves);
    assert!(!s.vault.is_eligible_liquidator(&Address::generate(&s.env)), "gated basket: no unverified liquidators");
}

// ---------------------------------------------------------------------------
// V5 — if the oracle had sources under the pseudo-sleeve "VAULT", the vault
// used them AS its share price and ignored what it actually holds, so the
// oracle admin could set NAV per share. VAULT-DESIGN: shares are priced from
// the vault's holdings; NAV-ORACLE: the vault PUBLISHES its NAV (output only).
// ---------------------------------------------------------------------------
#[test]
fn v5_share_price_comes_from_holdings_not_an_oracle_override() {
    let s = deploy_stack_with(false);
    let vault_key = Symbol::new(&s.env, "VAULT");
    // Oracle admin publishes a $2.00 "share price" with two consistent feeds.
    s.oracle.configure_sleeve(&vault_key, &200, &2);
    let dummy = Address::generate(&s.env);
    for id in ["OVERRIDE_A", "OVERRIDE_B"] {
        let mut src = oracle_source(&s.env, id, &dummy, 2 * ONE);
        src.sleeve = vault_key.clone();
        s.oracle.register_source(&src);
    }

    // The vault holds only USDC: a share is worth exactly $1.00.
    assert_eq!(s.vault.get_nav_deposit().unwrap().price, ONE, "deposit NAV must come from holdings");
    assert_eq!(s.vault.get_nav_conservative().unwrap().price, ONE);
    let alice = Address::generate(&s.env);
    s.usdc_admin.mint(&alice, &(100 * ONE));
    assert_eq!(s.vault.deposit(&(100 * ONE), &alice), 100 * ONE, "100 USDC buys 100 shares, not 50");
    assert_eq!(s.vault.redeem(&(100 * ONE), &alice, &alice), 100 * ONE);
}

// ---------------------------------------------------------------------------
// M3 — every contract had a public, unauthenticated `initialize`. Between
// deployment and initialization anyone could call it first and become admin.
// Fix: Soroban constructors (`__constructor`) run inside the deployment itself.
// ---------------------------------------------------------------------------
fn stranger_initializes(env: &Env, contract: &Address, args: soroban_sdk::Vec<soroban_sdk::Val>) -> bool {
    matches!(
        env.try_invoke_contract::<(), soroban_sdk::Error>(contract, &Symbol::new(env, "initialize"), args),
        Ok(Ok(()))
    )
}

#[test]
fn m3_fresh_deployments_cannot_be_taken_over() {
    let env = Env::default();
    env.ledger().set_timestamp(T0);
    let attacker = Address::generate(&env);
    let usdc = env.register_stellar_asset_contract_v2(Address::generate(&env)).address();

    let owner = Address::generate(&env);
    let registry = env.register(AssetRegistry, (owner.clone(),));
    assert!(!stranger_initializes(&env, &registry, (attacker.clone(),).into_val(&env)), "registry hijack");

    let oracle = env.register(NavOracle, ());
    assert!(!stranger_initializes(&env, &oracle, (attacker.clone(), usdc.clone(), 7u32, 300u32).into_val(&env)), "oracle hijack");

    let policy = env.register(PolicyContract, ());
    assert!(!stranger_initializes(&env, &policy, (attacker.clone(), oracle.clone()).into_val(&env)), "policy hijack");

    let vault = env.register(StrategyVault, ());
    let cfg = VaultConfig {
        admin: attacker.clone(),
        pause_authority: attacker.clone(),
        numeraire: usdc,
        policy_contract: policy,
        oracle_contract: oracle,
        registry_contract: registry,
        treasury: attacker.clone(),
        management_fee_bps: 10_000,
        tvl_cap: i128::MAX,
        per_user_cap: i128::MAX,
    };
    let args = (cfg, String::from_str(&env, "Evil"), String::from_str(&env, "EVIL"), 7u32).into_val(&env);
    assert!(!stranger_initializes(&env, &vault, args), "vault hijack");
}
