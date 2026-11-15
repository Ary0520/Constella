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
    token, Address, Env, IntoVal, String, Symbol, TryFromVal, Vec,
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
        feeder: Address::generate(env),
    }
}

/// Full contract stack: registry (USDC registered), policy, oracle and a
/// 0-fee strategy vault. With `vault_nav_feeds`, the oracle also carries two
/// $1.00 feeds under the key "VAULT", used by the oracle-focused tests. Since
/// audit V5 the vault never reads that key: share prices always come from
/// holdings.
fn deploy_stack_with(vault_nav_feeds: bool) -> Stack<'static> {
    deploy_stack_cfg(vault_nav_feeds, 0)
}

fn deploy_stack_cfg(vault_nav_feeds: bool, management_fee_bps: u32) -> Stack<'static> {
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


    let oracle_id = env.register(NavOracle, (admin.clone(), usdc_addr.clone(), 7u32, 300u32));
    let oracle = NavOracleClient::new(&env, &oracle_id);
    let policy_id = env.register(PolicyContract, (admin.clone(), oracle_id.clone()));
    let policy = PolicyContractClient::new(&env, &policy_id);
    if vault_nav_feeds {
        oracle.configure_sleeve(&Symbol::new(&env, "VAULT"), &200, &2);
        oracle.register_source(&oracle_source(&env, "USDC_A", &usdc_addr, ONE));
        oracle.register_source(&oracle_source(&env, "USDC_B", &usdc_addr, ONE));
    }

    let vault_id = env.register(
        StrategyVault,
        (
        VaultConfig {
            admin: admin.clone(),
            pause_authority: Address::generate(&env),
            numeraire: usdc_addr.clone(),
            policy_contract: policy_id,
            oracle_contract: oracle_id.clone(),
            registry_contract: registry_id.clone(),
            treasury: treasury.clone(),
            management_fee_bps,
            tvl_cap: 1_000_000 * 10_000_000, // FIXTURE: ROADMAP 3.2 $1M beta cap
            per_user_cap: 100_000 * 10_000_000, // FIXTURE: design-partner ticket size (GTM)
        },
        String::from_str(&env, "Constella Strategy Vault"),
        String::from_str(&env, "cVAULT"),
        7u32,
        ),
    );
    let vault = StrategyVaultClient::new(&env, &vault_id);

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
    s.vault.deposit(&(100 * ONE), &attacker, &attacker, &attacker);
    s.vault.deposit(&(900 * ONE), &victim, &victim, &victim);

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
    let out = s.vault.redeem(&s.vault.balance(&attacker), &attacker, &attacker, &attacker);
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
    s.vault.deposit(&(50 * ONE), &owner, &owner, &owner);
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
    s.vault.deposit(&(1000 * ONE), &alice, &alice, &alice);
    s.vault.deposit(&ONE, &s.admin, &s.admin, &s.admin);
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
    let _ = s.vault.try_redeem(&sliver.min(s.vault.balance(&s.admin)), &s.admin, &s.admin, &s.admin);
    let _ = s.vault.try_redeem(&s.vault.balance(&s.admin), &s.admin, &s.admin, &s.admin);

    let admin_gain = s.usdc.balance(&s.admin) - admin_usdc_before;
    assert!(admin_gain <= ONE, "admin extracted {admin_gain} after depositing {ONE}");
    let alice_out = s.vault.redeem(&s.vault.balance(&alice), &alice, &alice, &alice);
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
    assert_eq!(s.vault.deposit(&(100 * ONE), &user, &user, &user), 100 * ONE);
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
    s.vault.deposit(&(500 * ONE), &alice, &alice, &alice);
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
    assert!(s.vault.try_deposit(&(100 * ONE), &alice, &alice, &alice).is_ok(), "deposit blocked by an unrelated empty sleeve");
    assert!(s.vault.try_redeem(&(10 * ONE), &alice, &alice, &alice).is_ok(), "redemption blocked by an unrelated empty sleeve");
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
    s.vault.deposit(&(100 * ONE), &alice, &alice, &alice);

    // A feed diverges 10% (threshold 2%). Nobody reads a price yet.
    s.oracle.update_price(&Symbol::new(&s.env, "USDC_B"), &(ONE * 11 / 10), &200_000, &T0);
    assert!(
        s.oracle.is_sleeve_paused(&vault_sleeve),
        "deviation must latch in the same successful call that pushed the price"
    );
    assert_eq!(s.oracle.get_sleeve_config(&vault_sleeve).unwrap().pause_reason, Symbol::new(&s.env, "DEVIATION"));

    // A vault call that reverts afterwards cannot undo the pause.
    assert!(s.vault.try_redeem(&(1_000 * ONE), &alice, &alice, &alice).is_err());
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
    assert!(s.oracle.collateral_price(&asset).is_none(), "a single source must not produce a price");
    assert!(s.oracle.get_collateral_nav(&xlm_sleeve).is_none());
}

// ---------------------------------------------------------------------------
// P1 — slippage was checked as min_amount_out >= amount_in * (1 - cap) in RAW
// token units, ignoring the exchange rate. With EUR/USD = 1.08 a fair
// USDC->EURC trade was rejected and a 7.4%-loss EURC->USDC trade accepted.
// EXECUTION.md: "estimated slippage <= cap per route" (value-based).
// ---------------------------------------------------------------------------
fn usdc_eurc_policy(s: &Stack) -> Address {
    usdc_eurc_policy_with(s, (0, 10_000), (0, 10_000), "cash", 1000)
}

/// USDC + EURC policy with signed bands `(min, max)` per sleeve, EURC's policy
/// sleeve type and the liquidity floor (bps).
fn usdc_eurc_policy_with(s: &Stack, usdc_band: (u32, u32), eurc_band: (u32, u32), eurc_type: &str, floor_bps: u32) -> Address {
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
    for (id, (lo, hi), kind) in [("USDC", usdc_band, "cash"), ("EURC", eurc_band, eurc_type)] {
        targets.push_back(SleeveTarget {
            asset_id: Symbol::new(&s.env, id),
            target_weight_bps: 5000,
            min_band_bps: lo,
            max_band_bps: hi,
            sleeve_type: Symbol::new(&s.env, kind),
        });
    }
    let owner = Address::generate(&s.env);
    s.policy.register_policy(
        &s.vault_id, &owner, &soroban_sdk::BytesN::from_array(&s.env, &[0x77; 32]), &s.registry_id,
        &floor_bps, &10_000, &10_000, &50, &500, &targets,
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
    s.vault.deposit(&(100 * ONE), &holder, &holder, &holder);
    s.env.ledger().set_timestamp(T0 + constella_policy::PROTOCOL_MIN_NOTICE_SECONDS - 1);
    assert!(s.policy.try_activate_policy_update(&s.vault_id).is_err(), "one second early");
    let out = s.vault.redeem(&(100 * ONE), &holder, &holder, &holder);
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
    assert!(s.vault.try_deposit(&huge, &whale, &whale, &whale).is_err(), "a $10M deposit must exceed the caps");
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
    s.vault.deposit(&(1000 * ONE), &alice, &alice, &alice);

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
    assert_eq!(s.vault.deposit(&(100 * ONE), &alice, &alice, &alice), 100 * ONE, "100 USDC buys 100 shares, not 50");
    assert_eq!(s.vault.redeem(&(100 * ONE), &alice, &alice, &alice), 100 * ONE);
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
    // The numéraire is in the closed universe before any vault uses it (audit M4).
    env.mock_all_auths();
    AssetRegistryClient::new(&env, &registry).add_asset(&AssetEntry {
        contract_address: usdc.clone(),
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
    env.set_auths(&[]);

    let oracle = env.register(NavOracle, (owner.clone(), usdc.clone(), 7u32, 300u32));
    assert!(!stranger_initializes(&env, &oracle, (attacker.clone(), usdc.clone(), 7u32, 300u32).into_val(&env)), "oracle hijack");

    let policy = env.register(PolicyContract, (owner.clone(), oracle.clone()));
    assert!(!stranger_initializes(&env, &policy, (attacker.clone(), oracle.clone()).into_val(&env)), "policy hijack");

    let legit = VaultConfig {
        admin: owner.clone(),
        pause_authority: owner.clone(),
        numeraire: usdc,
        policy_contract: policy,
        oracle_contract: oracle,
        registry_contract: registry,
        treasury: owner.clone(),
        management_fee_bps: 10,
        tvl_cap: 1_000_000 * ONE,
        per_user_cap: 100_000 * ONE,
    };
    let vault = env.register(StrategyVault, (legit.clone(), String::from_str(&env, "V"), String::from_str(&env, "V"), 7u32));
    let mut hostile = legit.clone();
    hostile.admin = attacker.clone();
    hostile.treasury = attacker.clone();
    hostile.management_fee_bps = 10_000;
    let args = (hostile, String::from_str(&env, "Evil"), String::from_str(&env, "EVIL"), 7u32).into_val(&env);
    assert!(!stranger_initializes(&env, &vault, args), "vault hijack");
    assert_eq!(StrategyVaultClient::new(&env, &vault).get_config(), legit, "configuration unchanged");
}

// ---------------------------------------------------------------------------
// M4 — the vault always labelled its cash sleeve "USDC", even for a EURC
// vault, and accepted any token as numéraire. VAULT-DESIGN: numéraire is USDC
// or EURC; README Closed Universe: only registered, active assets are used.
// ---------------------------------------------------------------------------
fn eur_vault_config(env: &Env, admin: &Address, numeraire: &Address, registry: &Address) -> VaultConfig {
    let oracle = env.register(NavOracle, (admin.clone(), numeraire.clone(), 7u32, 300u32));
    let policy = env.register(PolicyContract, (admin.clone(), oracle.clone()));
    VaultConfig {
        admin: admin.clone(),
        pause_authority: admin.clone(),
        numeraire: numeraire.clone(),
        policy_contract: policy,
        oracle_contract: oracle,
        registry_contract: registry.clone(),
        treasury: admin.clone(),
        management_fee_bps: 0,
        tvl_cap: 1_000_000 * ONE,
        per_user_cap: 100_000 * ONE,
    }
}

#[test]
fn m4_cash_sleeve_is_named_from_the_registry_and_numeraire_must_be_registered() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(T0);
    let admin = Address::generate(&env);
    let registry_id = env.register(AssetRegistry, (admin.clone(),));
    let eurc = env.register_stellar_asset_contract_v2(Address::generate(&env)).address();
    AssetRegistryClient::new(&env, &registry_id).add_asset(&AssetEntry {
        contract_address: eurc.clone(),
        asset_id: Symbol::new(&env, "EURC"),
        issuer: Symbol::new(&env, "CIRCLE"),
        sleeve_type: RegSleeve::Cash,
        liquidity_tier: 0,
        permissionless: true,
        oracle_sources: Vec::from_array(&env, [Symbol::new(&env, "EURC_A"), Symbol::new(&env, "EURC_B")]),
        eligible_jurisdictions: Vec::new(&env),
        blocked_jurisdictions: Vec::new(&env),
        investor_classes_required: Vec::new(&env),
        is_active: true,
        added_at: T0,
    });

    let cfg = eur_vault_config(&env, &admin, &eurc, &registry_id);
    let vault = env.register(StrategyVault, (cfg, String::from_str(&env, "Euro Vault"), String::from_str(&env, "cEUR"), 7u32));
    let cash = StrategyVaultClient::new(&env, &vault).get_sleeves().get(0).unwrap();
    assert_eq!(cash.asset_id, String::from_str(&env, "EURC"), "a EURC vault's cash sleeve must be labelled EURC");

    // A token that is not in the closed universe cannot be the numéraire.
    let rogue = env.register_stellar_asset_contract_v2(Address::generate(&env)).address();
    let cfg = eur_vault_config(&env, &admin, &rogue, &registry_id);
    let deployed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        env.register(StrategyVault, (cfg, String::from_str(&env, "Rogue"), String::from_str(&env, "R"), 7u32))
    }));
    assert!(deployed.is_err(), "an unregistered numéraire must make deployment fail");
}

// ---------------------------------------------------------------------------
// W2 — share ledger lived in instance storage with no TTL management: one
// ever-growing entry loaded on every call, archived after the default TTL.
// W6 — share token was not SEP-41 complete.
// ---------------------------------------------------------------------------
/// (topics, data) of every event the vault emitted in the last invocation.
fn vault_events(s: &Stack) -> std::vec::Vec<(std::vec::Vec<soroban_sdk::xdr::ScVal>, soroban_sdk::xdr::ScVal)> {
    use soroban_sdk::testutils::Events as _;
    use soroban_sdk::xdr::ContractEventBody;
    let mut out = std::vec::Vec::new();
    for e in s.env.events().all().filter_by_contract(&s.vault_id).events() {
        let ContractEventBody::V0(body) = &e.body;
        out.push((body.topics.iter().cloned().collect(), body.data.clone()));
    }
    out
}

fn sym(name: &str) -> soroban_sdk::xdr::ScVal {
    soroban_sdk::xdr::ScVal::Symbol(name.try_into().unwrap())
}

fn sc(env: &Env, a: &Address) -> soroban_sdk::xdr::ScVal {
    soroban_sdk::xdr::ScVal::try_from_val(env, &a.to_val()).unwrap()
}

fn i128_val(v: i128) -> soroban_sdk::xdr::ScVal {
    soroban_sdk::xdr::ScVal::I128(soroban_sdk::xdr::Int128Parts { hi: (v >> 64) as i64, lo: v as u64 })
}

fn contract_error(code: u32) -> soroban_sdk::Error {
    soroban_sdk::Error::from_contract_error(code)
}

#[test]
fn w2_share_ledger_is_persistent_per_holder_with_ttl() {
    use stellar_tokens::fungible::{total_supply::TotalSupplyStorageKey, FungibleStorageKey, BALANCE_TTL_THRESHOLD};
    use soroban_sdk::testutils::storage::Persistent as _;
    let s = deploy_stack();
    let alice = Address::generate(&s.env);
    s.usdc_admin.mint(&alice, &(100 * ONE));
    s.vault.deposit(&(100 * ONE), &alice, &alice, &alice);
    // OZ extends a balance entry's TTL whenever it is accessed (a new entry
    // starts at the network minimum: 4096 ledgers here, ~120 days on mainnet).
    assert_eq!(s.vault.balance(&alice), 100 * ONE);

    s.env.as_contract(&s.vault_id, || {
        let bal_key = FungibleStorageKey::Balance(alice.clone());
        let p = s.env.storage().persistent();
        assert_eq!(p.get::<_, i128>(&bal_key), Some(100 * ONE), "balance must be a persistent per-holder entry");
        assert!(p.get_ttl(&bal_key) >= BALANCE_TTL_THRESHOLD, "balance TTL not extended on access");
        assert_eq!(p.get::<_, i128>(&TotalSupplyStorageKey::TotalSupply), Some(100 * ONE));
    });
}

#[test]
fn w2_instance_storage_does_not_grow_with_the_number_of_holders() {
    use soroban_sdk::testutils::storage::{Instance as _, Persistent as _};
    let s = deploy_stack();
    let instance_len = |s: &Stack| s.env.as_contract(&s.vault_id, || s.env.storage().instance().all().len());
    let first = Address::generate(&s.env);
    s.usdc_admin.mint(&first, &(10 * ONE));
    s.vault.deposit(&(10 * ONE), &first, &first, &first);
    let baseline = instance_len(&s);

    // Every call loads the whole instance entry, and entries are size-capped:
    // per-holder state must not live there.
    for _ in 0..20 {
        let holder = Address::generate(&s.env);
        s.usdc_admin.mint(&holder, &(10 * ONE));
        s.vault.deposit(&(10 * ONE), &holder, &holder, &holder);
        let until = s.env.ledger().sequence() + 1_000;
        s.vault.approve(&holder, &first, &ONE, &until);
    }
    assert_eq!(instance_len(&s), baseline, "per-holder state leaked into instance storage");
    assert!(s.env.as_contract(&s.vault_id, || s.env.storage().instance().get_ttl()) > 0);
    let persisted = s.env.as_contract(&s.vault_id, || s.env.storage().persistent().all().len());
    assert!(persisted >= 21, "each holder balance must be its own persistent entry");
}

#[test]
fn w6_approve_validates_live_until_ledger_and_reports_it() {
    let s = deploy_stack();
    let owner = Address::generate(&s.env);
    let spender = Address::generate(&s.env);
    let seq = s.env.ledger().sequence();

    // Already expired.
    authorize(&s, &owner, "approve", (owner.clone(), spender.clone(), 10 * ONE, seq - 1).into_val(&s.env));
    assert!(s.vault.try_approve(&owner, &spender, &(10 * ONE), &(seq - 1)).is_err());
    // Beyond the network's maximum entry lifetime.
    let too_far = s.env.ledger().max_live_until_ledger() + 1;
    authorize(&s, &owner, "approve", (owner.clone(), spender.clone(), 10 * ONE, too_far).into_val(&s.env));
    assert!(s.vault.try_approve(&owner, &spender, &(10 * ONE), &too_far).is_err());
    assert_eq!(s.vault.allowance(&owner, &spender), 0);

    // Valid approval: SEP-41 event data is [amount, live_until_ledger].
    let until = seq + 1_000;
    authorize(&s, &owner, "approve", (owner.clone(), spender.clone(), 10 * ONE, until).into_val(&s.env));
    s.vault.approve(&owner, &spender, &(10 * ONE), &until);
    assert_eq!(s.env.auths()[0].0, owner);
    let data = soroban_sdk::xdr::ScVal::Vec(Some(
        std::vec![i128_val(10 * ONE), soroban_sdk::xdr::ScVal::U32(until)].try_into().unwrap(),
    ));
    assert_eq!(
        vault_events(&s),
        std::vec![(std::vec![sym("approve"), sc(&s.env, &owner), sc(&s.env, &spender)], data)]
    );
    assert_eq!(s.vault.allowance(&owner, &spender), 10 * ONE);

    // The allowance lapses after live_until_ledger.
    s.env.ledger().set_sequence_number(until + 1);
    assert_eq!(s.vault.allowance(&owner, &spender), 0);
}

#[test]
fn w6_holder_can_burn_and_spender_can_burn_from_within_allowance() {
    let s = deploy_stack();
    let alice = Address::generate(&s.env);
    let spender = Address::generate(&s.env);
    let stranger = Address::generate(&s.env);
    s.usdc_admin.mint(&alice, &(100 * ONE));
    s.vault.deposit(&(100 * ONE), &alice, &alice, &alice);

    // A stranger cannot burn someone else's shares.
    authorize(&s, &stranger, "burn", (alice.clone(), ONE).into_val(&s.env));
    assert!(s.vault.try_burn(&alice, &ONE).is_err());

    authorize(&s, &alice, "burn", (alice.clone(), 10 * ONE).into_val(&s.env));
    s.vault.burn(&alice, &(10 * ONE));
    assert_eq!(s.env.auths()[0].0, alice);
    assert_eq!(
        vault_events(&s),
        std::vec![(std::vec![sym("burn"), sc(&s.env, &alice)], i128_val(10 * ONE))]
    );
    assert_eq!(s.vault.balance(&alice), 90 * ONE);
    assert_eq!(s.vault.total_supply(), 90 * ONE);

    // Cannot burn more than held.
    authorize(&s, &alice, "burn", (alice.clone(), 91 * ONE).into_val(&s.env));
    assert_eq!(s.vault.try_burn(&alice, &(91 * ONE)).err().unwrap().unwrap(), contract_error(6));

    let until = s.env.ledger().sequence() + 1_000;
    s.env.mock_all_auths();
    s.vault.approve(&alice, &spender, &(20 * ONE), &until);

    authorize(&s, &spender, "burn_from", (spender.clone(), alice.clone(), 15 * ONE).into_val(&s.env));
    s.vault.burn_from(&spender, &alice, &(15 * ONE));
    assert_eq!(s.env.auths()[0].0, spender);
    assert_eq!(
        vault_events(&s),
        std::vec![(std::vec![sym("burn"), sc(&s.env, &alice)], i128_val(15 * ONE))]
    );
    assert_eq!(s.vault.allowance(&alice, &spender), 5 * ONE);
    assert_eq!(s.vault.balance(&alice), 75 * ONE);
    assert_eq!(s.vault.total_supply(), 75 * ONE);

    // Cannot exceed the remaining allowance.
    authorize(&s, &spender, "burn_from", (spender.clone(), alice.clone(), 6 * ONE).into_val(&s.env));
    assert_eq!(
        s.vault.try_burn_from(&spender, &alice, &(6 * ONE)).err().unwrap().unwrap(),
        contract_error(7)
    );
}

#[test]
fn w6_fee_shares_are_announced_with_a_mint_event() {
    // FIXTURE: 25 bps/year, the ceiling in docs/build-spec/README.md ("0.05%-0.25%/year").
    let s = deploy_stack_cfg(true, 25);
    let alice = Address::generate(&s.env);
    let bob = Address::generate(&s.env);
    s.usdc_admin.mint(&alice, &(1_000 * ONE));
    s.vault.deposit(&(1_000 * ONE), &alice, &alice, &alice);
    assert_eq!(s.vault.balance(&s.treasury), 0);

    s.env.ledger().set_timestamp(T0 + 30 * 86_400);
    authorize(&s, &alice, "transfer", (alice.clone(), bob.clone(), ONE).into_val(&s.env));
    s.vault.transfer(&alice, &bob, &ONE);
    let events = vault_events(&s); // events of the transfer call only

    let fee = s.vault.balance(&s.treasury);
    assert!(fee > 0, "fee must have accrued");
    let mints: std::vec::Vec<_> = events.into_iter().filter(|(t, _)| t[0] == sym("mint")).collect();
    assert_eq!(mints, std::vec![(std::vec![sym("mint"), sc(&s.env, &s.treasury)], i128_val(fee))]);
    assert_eq!(
        s.vault.balance(&alice) + s.vault.balance(&bob) + fee,
        s.vault.total_supply(),
        "sum of balances must equal total supply"
    );
}

// ---------------------------------------------------------------------------
// W4.2b — SEP-41 v0.5 `transfer(from, to: MuxedAddress, amount)`: exchanges
// and custodians credit customers by muxed id; a plain-Address-only transfer
// cannot receive to them.
// ---------------------------------------------------------------------------
#[test]
fn w6_transfer_accepts_a_muxed_destination_and_reports_its_id() {
    use soroban_sdk::testutils::MuxedAddress as _;
    use soroban_sdk::xdr::{ScMap, ScMapEntry, ScVal};
    let s = deploy_stack();
    let alice = Address::generate(&s.env);
    s.usdc_admin.mint(&alice, &(100 * ONE));
    s.vault.deposit(&(100 * ONE), &alice, &alice, &alice);

    // Only accounts (G...) can be muxed; generate an account-based M... address.
    let muxed = soroban_sdk::MuxedAddress::new(soroban_sdk::MuxedAddress::generate(&s.env), 42);
    let custodian = muxed.address();
    assert_eq!(muxed.id(), Some(42));
    authorize(&s, &alice, "transfer", (alice.clone(), muxed.clone(), 10 * ONE).into_val(&s.env));
    s.vault.transfer(&alice, &muxed, &(10 * ONE));
    assert_eq!(s.env.auths()[0].0, alice);

    let data = ScVal::Map(Some(ScMap(
        std::vec![
            ScMapEntry { key: sym("amount"), val: i128_val(10 * ONE) },
            ScMapEntry { key: sym("to_muxed_id"), val: ScVal::U64(42) },
        ]
        .try_into()
        .unwrap(),
    )));
    assert_eq!(
        vault_events(&s),
        std::vec![(std::vec![sym("transfer"), sc(&s.env, &alice), sc(&s.env, &custodian)], data)]
    );
    // Shares land on the underlying account; the id is routing metadata only.
    assert_eq!(s.vault.balance(&custodian), 10 * ONE);
    assert_eq!(s.vault.balance(&alice), 90 * ONE);
    assert_eq!(s.vault.total_supply(), 100 * ONE);
}

// ---------------------------------------------------------------------------
// W4.2b — SEP-56 v0.1.2 conformance: deposit/mint take (receiver, from,
// operator), withdraw/redeem take (receiver, owner, operator), operator
// authorizes; preview_* and query_asset exist. A third-party operator may only
// move `from`'s numeraire under an allowance `from` granted on the numeraire,
// and may only burn `owner`'s shares under an allowance on the vault shares.
// ---------------------------------------------------------------------------
fn authorize_tree(s: &Stack, who: &Address, fn_name: &'static str, args: Vec<soroban_sdk::Val>, subs: &[MockAuthInvoke]) {
    s.env.mock_auths(&[MockAuth {
        address: who,
        invoke: &MockAuthInvoke { contract: &s.vault_id, fn_name, args, sub_invokes: subs },
    }]);
}

fn assets_shares_map(assets: i128, shares: i128) -> soroban_sdk::xdr::ScVal {
    use soroban_sdk::xdr::{ScMap, ScMapEntry, ScVal};
    ScVal::Map(Some(ScMap(
        std::vec![
            ScMapEntry { key: sym("assets"), val: i128_val(assets) },
            ScMapEntry { key: sym("shares"), val: i128_val(shares) },
        ]
        .try_into()
        .unwrap(),
    )))
}

#[test]
fn sep56_operator_deposits_only_from_a_payer_who_approved_it() {
    let s = deploy_stack();
    assert_eq!(s.vault.query_asset(), s.usdc_addr);
    let payer = Address::generate(&s.env);
    let operator = Address::generate(&s.env);
    let receiver = Address::generate(&s.env);
    s.usdc_admin.mint(&payer, &(100 * ONE));
    let amt = 100 * ONE;
    let pull: Vec<soroban_sdk::Val> = (operator.clone(), payer.clone(), s.vault_id.clone(), amt).into_val(&s.env);
    let sub = [MockAuthInvoke { contract: &s.usdc_addr, fn_name: "transfer_from", args: pull, sub_invokes: &[] }];
    let args: Vec<soroban_sdk::Val> = (amt, receiver.clone(), payer.clone(), operator.clone()).into_val(&s.env);

    // No numeraire allowance from the payer: the operator cannot spend its funds.
    authorize_tree(&s, &operator, "deposit", args.clone(), &sub);
    assert!(s.vault.try_deposit(&amt, &receiver, &payer, &operator).is_err());
    assert_eq!(s.usdc.balance(&payer), amt);

    let until = s.env.ledger().sequence() + 1_000;
    s.env.mock_all_auths();
    s.usdc.approve(&payer, &operator, &amt, &until);

    // Only the operator signs this call.
    authorize_tree(&s, &operator, "deposit", args, &sub);
    let shares = s.vault.deposit(&amt, &receiver, &payer, &operator);
    assert!(s.env.auths().iter().all(|(a, _)| *a == operator), "only the operator may sign");
    assert_eq!(
        vault_events(&s).into_iter().filter(|(t, _)| t[0] == sym("deposit")).collect::<std::vec::Vec<_>>(),
        std::vec![(
            std::vec![sym("deposit"), sc(&s.env, &operator), sc(&s.env, &payer), sc(&s.env, &receiver)],
            assets_shares_map(amt, shares)
        )]
    );
    assert_eq!(s.vault.balance(&receiver), shares);
    assert_eq!(s.vault.balance(&operator), 0);
    assert_eq!(s.usdc.balance(&payer), 0);
    assert_eq!(s.usdc.balance(&s.vault_id), amt);
}

#[test]
fn sep56_operator_redeems_only_within_the_owners_share_allowance() {
    let s = deploy_stack();
    let owner = Address::generate(&s.env);
    let operator = Address::generate(&s.env);
    let receiver = Address::generate(&s.env);
    s.usdc_admin.mint(&owner, &(100 * ONE));
    s.vault.deposit(&(100 * ONE), &owner, &owner, &owner);

    // No share allowance: rejected with the vault's allowance error.
    let args: Vec<soroban_sdk::Val> = (20 * ONE, receiver.clone(), owner.clone(), operator.clone()).into_val(&s.env);
    authorize_tree(&s, &operator, "redeem", args.clone(), &[]);
    assert_eq!(
        s.vault.try_redeem(&(20 * ONE), &receiver, &owner, &operator).err().unwrap().unwrap(),
        contract_error(7)
    );

    let until = s.env.ledger().sequence() + 1_000;
    s.env.mock_all_auths(); // setup: the owner grants the share allowance
    s.vault.approve(&owner, &operator, &(30 * ONE), &until);
    authorize_tree(&s, &operator, "redeem", args, &[]);
    let assets = s.vault.redeem(&(20 * ONE), &receiver, &owner, &operator);
    assert_eq!(s.env.auths().len(), 1);
    assert_eq!(s.env.auths()[0].0, operator);
    assert_eq!(
        vault_events(&s).into_iter().filter(|(t, _)| t[0] == sym("withdraw")).collect::<std::vec::Vec<_>>(),
        std::vec![(
            std::vec![sym("withdraw"), sc(&s.env, &operator), sc(&s.env, &receiver), sc(&s.env, &owner)],
            assets_shares_map(assets, 20 * ONE)
        )]
    );
    assert_eq!(s.usdc.balance(&receiver), assets);
    assert_eq!(s.vault.balance(&owner), 80 * ONE);
    assert_eq!(s.vault.allowance(&owner, &operator), 10 * ONE);

    // withdraw also spends the allowance; 10 shares left cannot cover 11 assets' worth.
    let wargs: Vec<soroban_sdk::Val> = (11 * ONE, receiver.clone(), owner.clone(), operator.clone()).into_val(&s.env);
    authorize_tree(&s, &operator, "withdraw", wargs, &[]);
    assert_eq!(
        s.vault.try_withdraw(&(11 * ONE), &receiver, &owner, &operator).err().unwrap().unwrap(),
        contract_error(7)
    );
}

#[test]
fn sep56_previews_equal_execution_even_with_pending_fees() {
    // FIXTURE: 25 bps/year, the ceiling in docs/build-spec/README.md.
    let s = deploy_stack_cfg(true, 25);
    let a = Address::generate(&s.env);
    s.usdc_admin.mint(&a, &(10_000 * ONE));
    s.vault.deposit(&(5_000 * ONE), &a, &a, &a);
    let mut t = T0;
    let mut later = |s: &Stack| {
        t += 37 * 86_400;
        s.env.ledger().set_timestamp(t);
    };

    later(&s);
    let p = s.vault.preview_deposit(&(1_000 * ONE));
    assert_eq!(s.vault.deposit(&(1_000 * ONE), &a, &a, &a), p, "preview_deposit");
    later(&s);
    let p = s.vault.preview_mint(&(500 * ONE));
    assert_eq!(s.vault.mint(&(500 * ONE), &a, &a, &a), p, "preview_mint");
    later(&s);
    let p = s.vault.preview_redeem(&(700 * ONE));
    assert_eq!(s.vault.redeem(&(700 * ONE), &a, &a, &a), p, "preview_redeem");
    later(&s);
    let p = s.vault.preview_withdraw(&(300 * ONE));
    assert_eq!(s.vault.withdraw(&(300 * ONE), &a, &a, &a), p, "preview_withdraw");
    assert!(s.vault.balance(&s.treasury) > 0, "fees must actually have accrued");
}

// ---------------------------------------------------------------------------
// W1a — policy-validated rebalancing through a REAL DEX. Soroswap router,
// factory and pair WASMs were fetched read-only from Stellar mainnet on
// 15 Nov 2026 (router CAG5LRYQ..., factory CA4HEQTL..., pair CC7CDFY2...;
// pair sha256 18051456... equals Soroswap's published pair_wasm_hash).
// ---------------------------------------------------------------------------
#[allow(clippy::too_many_arguments)] // generated client for Soroswap's own interface
mod soroswap_router {
    soroban_sdk::contractimport!(file = "testdata/soroswap_router_mainnet.wasm");
}
#[allow(clippy::too_many_arguments)] // generated client for Soroswap's own interface
mod soroswap_factory {
    soroban_sdk::contractimport!(file = "testdata/soroswap_factory_mainnet.wasm");
}
const SOROSWAP_PAIR_WASM: &[u8] = include_bytes!("../testdata/soroswap_pair_mainnet.wasm");

mod w1 {
    use super::*;
    use constella_vault_strategy::{
        RebalanceProposal as VaultProposal, TradeOperation as VaultTrade, TradeRoute as VaultRoute,
    };

    const DELAY: u64 = 48 * 3600;

    pub(super) struct R {
        pub(super) s: Stack<'static>,
        pub(super) eurc: Address,
        pub(super) eurc_admin: token::StellarAssetClient<'static>,
        pub(super) router: soroswap_router::Client<'static>,
        pub(super) keeper: Address,
        pub(super) now: u64,
    }

    /// Full stack + EURC sleeve + a real Soroswap USDC/EURC pool at 1.08 with
    /// 1.08M / 1M liquidity, the router configured through the 48h delay,
    /// oracle feeds refreshed, a keeper, and 10_000 USDC deposited.
    pub(super) fn setup() -> R {
        setup_with(|s| usdc_eurc_policy(s))
    }

    /// `setup` with a caller-chosen signed policy (W1b band / floor tests).
    pub(super) fn setup_with(policy: impl FnOnce(&Stack<'static>) -> Address) -> R {
        let s = deploy_stack();
        let env = &s.env;
        let eurc = policy(&s);
        let eurc_admin = token::StellarAssetClient::new(env, &eurc);

        // Vault sleeves: USDC (cash, numéraire) + EURC.
        let mut sleeves = Vec::new(env);
        sleeves.push_back(SleeveState { asset_id: String::from_str(env, "USDC"), contract_address: s.usdc_addr.clone(),
            target_weight_bps: 5000, band_bps: 5000, current_balance: 0, sleeve_type: SleeveType::CashStable, liquidity_tier: 0 });
        sleeves.push_back(SleeveState { asset_id: String::from_str(env, "EURC"), contract_address: eurc.clone(),
            target_weight_bps: 5000, band_bps: 5000, current_balance: 0, sleeve_type: SleeveType::EurSovereign, liquidity_tier: 0 });
        s.vault.set_sleeves(&sleeves);

        // Real Soroswap: upload the pair code, deploy factory + router.
        let pair_hash = env.deployer().upload_contract_wasm(SOROSWAP_PAIR_WASM);
        let factory = soroswap_factory::Client::new(env, &env.register(soroswap_factory::WASM, ()));
        factory.initialize(&Address::generate(env), &pair_hash);
        let router = soroswap_router::Client::new(env, &env.register(soroswap_router::WASM, ()));
        router.initialize(&factory.address);
        let lp = Address::generate(env);
        s.usdc_admin.mint(&lp, &(1_080_000 * ONE));
        eurc_admin.mint(&lp, &(1_000_000 * ONE));
        router.add_liquidity(&s.usdc_addr, &eurc, &(1_080_000 * ONE), &(1_000_000 * ONE), &0, &0, &lp, &(T0 + 10_000));

        // Router through the 48h delay (a router can move vault funds).
        s.vault.propose_router(&VaultRoute::Soroswap, &router.address);
        let now = T0 + DELAY;
        env.ledger().set_timestamp(now);
        s.vault.activate_router(&VaultRoute::Soroswap);
        // Feeds refreshed after the wait (their feeders push).
        for (id, px) in [("USDC_A", ONE), ("USDC_B", ONE), ("EURC_A", ONE * 108 / 100), ("EURC_B", ONE * 108 / 100)] {
            s.oracle.update_price(&Symbol::new(env, id), &px, &200_000, &now);
        }
        let keeper = Address::generate(env);
        s.vault.set_keeper(&keeper);
        let alice = Address::generate(env);
        s.usdc_admin.mint(&alice, &(10_000 * ONE));
        s.vault.deposit(&(10_000 * ONE), &alice, &alice, &alice);
        let r = R { s, eurc, eurc_admin, router, keeper, now };
        let _ = &r.eurc_admin;
        r
    }

    pub(super) fn proposal(r: &R, amount_in: i128, min_out: i128) -> VaultProposal {
        let env = &r.s.env;
        let mut w = Vec::new(env);
        w.push_back((Symbol::new(env, "USDC"), 9000u32));
        w.push_back((Symbol::new(env, "EURC"), 1000u32));
        VaultProposal {
            vault: r.s.vault_id.clone(),
            policy_hash: r.s.policy.get_policy_hash(&r.s.vault_id),
            operations: Vec::from_array(env, [VaultTrade {
                asset_in: r.s.usdc_addr.clone(), asset_out: r.eurc.clone(), amount_in, min_amount_out: min_out, route: VaultRoute::Soroswap,
            }]),
            weights_after: w,
            deadline: r.now + 600,
            proposer: r.keeper.clone(),
        }
    }

    pub(super) fn quote(r: &R, amount_in: i128) -> i128 {
        let path = Vec::from_array(&r.s.env, [r.s.usdc_addr.clone(), r.eurc.clone()]);
        r.router.router_get_amounts_out(&amount_in, &path).get(1).unwrap()
    }

    pub(super) fn as_keeper(r: &R, p: &VaultProposal) {
        authorize(&r.s, &r.keeper, "rebalance", (p.clone(),).into_val(&r.s.env));
    }

    pub(super) fn holdings(r: &R) -> (i128, i128) {
        let sl = r.s.vault.get_sleeves();
        (sl.get(0).unwrap().current_balance, sl.get(1).unwrap().current_balance)
    }

    #[test]
    fn a_policy_validated_swap_executes_through_real_soroswap() {
        let r = setup();
        let out = quote(&r, 1_000 * ONE);
        let p = proposal(&r, 1_000 * ONE, out);
        as_keeper(&r, &p);
        // The SDK's auth-snapshot code for env.auths() (test observation, run in
        // the host's shadow budget) exhausts the default test budget on this deep
        // cross-contract tree. Lift the harness cap, then enforce the REAL limit
        // explicitly: the rebalance must fit a mainnet transaction.
        r.s.env.cost_estimate().budget().reset_unlimited();
        r.s.vault.rebalance(&p);
        let used = r.s.env.cost_estimate().resources();
        assert!(used.instructions < 100_000_000, "rebalance needs {} instructions (mainnet tx limit 100M)", used.instructions);
        assert!(used.mem_bytes < 40 * 1024 * 1024, "rebalance needs {} bytes (mainnet tx limit 40 MB)", used.mem_bytes);
        assert!(r.s.env.auths().iter().all(|(a, _)| *a == r.keeper), "only the keeper signs");
        // Booked from measured token deltas, equal to the real balances.
        assert_eq!(holdings(&r), (9_000 * ONE, out));
        assert_eq!(r.s.usdc.balance(&r.s.vault_id), 9_000 * ONE);
        assert_eq!(token::Client::new(&r.s.env, &r.eurc).balance(&r.s.vault_id), out);
        // The funded EURC sleeve is priced per asset (1.08): the vault keeps working.
        let expected_total = 9_000 * ONE + out * 108 / 100;
        assert_eq!(r.s.vault.total_assets(), expected_total);
        r.s.env.mock_all_auths();
        let bob = Address::generate(&r.s.env);
        r.s.usdc_admin.mint(&bob, &(100 * ONE));
        assert!(r.s.vault.deposit(&(100 * ONE), &bob, &bob, &bob) > 0);
    }

    #[test]
    fn a_proposal_the_policy_rejects_changes_nothing() {
        let r = setup();
        // 2% below fair value: beyond the signed 0.5% slippage cap.
        let p = proposal(&r, 1_000 * ONE, 1_000 * ONE * 100 / 108 * 98 / 100);
        as_keeper(&r, &p);
        assert!(r.s.vault.try_rebalance(&p).is_err());
        assert_eq!(holdings(&r), (10_000 * ONE, 0));
        assert_eq!(r.s.usdc.balance(&r.s.vault_id), 10_000 * ONE);
    }

    #[test]
    fn only_the_designated_keeper_may_rebalance() {
        let r = setup();
        let mut p = proposal(&r, 1_000 * ONE, quote(&r, 1_000 * ONE));
        let stranger = Address::generate(&r.s.env);
        p.proposer = stranger.clone();
        authorize(&r.s, &stranger, "rebalance", (p.clone(),).into_val(&r.s.env));
        assert!(r.s.vault.try_rebalance(&p).is_err());
        assert_eq!(holdings(&r), (10_000 * ONE, 0));
    }

    #[test]
    fn router_changes_wait_48_hours() {
        let r = setup();
        r.s.env.mock_all_auths();
        let evil = Address::generate(&r.s.env);
        r.s.vault.propose_router(&VaultRoute::Soroswap, &evil);
        assert!(r.s.vault.try_activate_router(&VaultRoute::Soroswap).is_err(), "not before 48h");
        assert_eq!(r.s.vault.get_router(&VaultRoute::Soroswap), Some(r.router.address.clone()), "old router stays");
        // A stranger cannot propose a router.
        let stranger = Address::generate(&r.s.env);
        authorize(&r.s, &stranger, "propose_router", (VaultRoute::Soroswap, evil.clone()).into_val(&r.s.env));
        assert!(r.s.vault.try_propose_router(&VaultRoute::Soroswap, &evil).is_err());
    }

    #[test]
    fn a_front_run_that_moves_the_pool_reverts_the_whole_rebalance() {
        let r = setup();
        let p = proposal(&r, 1_000 * ONE, quote(&r, 1_000 * ONE));
        // Someone else buys EURC first (sandwich / price move).
        r.s.env.mock_all_auths();
        let whale = Address::generate(&r.s.env);
        r.s.usdc_admin.mint(&whale, &(50_000 * ONE));
        let path = Vec::from_array(&r.s.env, [r.s.usdc_addr.clone(), r.eurc.clone()]);
        r.router.swap_exact_tokens_for_tokens(&(50_000 * ONE), &0, &path, &whale, &(r.now + 600));
        as_keeper(&r, &p);
        assert!(r.s.vault.try_rebalance(&p).is_err(), "min_amount_out no longer reachable");
        assert_eq!(holdings(&r), (10_000 * ONE, 0));
    }

    #[test]
    fn a_trade_larger_than_the_sleeve_is_refused() {
        let r = setup();
        let p = proposal(&r, 10_001 * ONE, quote(&r, 10_001 * ONE));
        as_keeper(&r, &p);
        assert!(r.s.vault.try_rebalance(&p).is_err());
        assert_eq!(holdings(&r), (10_000 * ONE, 0));
    }

    #[test]
    fn a_paused_vault_cannot_rebalance() {
        let r = setup();
        r.s.env.mock_all_auths();
        r.s.vault.pause();
        let p = proposal(&r, 1_000 * ONE, quote(&r, 1_000 * ONE));
        as_keeper(&r, &p);
        assert!(r.s.vault.try_rebalance(&p).is_err());
    }
}

/// Audit W1b: after the trades, the vault's ACTUAL holdings (measured
/// balances x per-asset median prices) must satisfy the signed policy's
/// bands, caps and liquidity floor - not merely the weights the keeper claims.
mod w1b {
    use super::w1::*;
    use super::*;

    const ALLOCATION_OUT_OF_POLICY: u32 = 27;
    const ORACLE_UNAVAILABLE: u32 = 10;

    fn claim(r: &R, usdc_bps: u32, eurc_bps: u32, amount_in: i128) -> constella_vault_strategy::RebalanceProposal {
        let env = &r.s.env;
        let mut p = proposal(r, amount_in, quote(r, amount_in));
        p.weights_after = Vec::from_array(env, [(Symbol::new(env, "USDC"), usdc_bps), (Symbol::new(env, "EURC"), eurc_bps)]);
        p
    }

    #[test]
    fn a_claim_inside_the_bands_cannot_hide_an_actual_breach() {
        // Signed bands: USDC 85-95 %, EURC 5-15 %.
        let r = setup_with(|s| usdc_eurc_policy_with(s, (8500, 9500), (500, 1500), "cash", 1000));
        // The keeper CLAIMS 90/10 (valid) but trades half the vault (~50/50).
        let p = claim(&r, 9000, 1000, 5_000 * ONE);
        as_keeper(&r, &p);
        r.s.env.cost_estimate().budget().reset_unlimited();
        let res = r.s.vault.try_rebalance(&p);
        assert_eq!(res.unwrap_err().unwrap(), contract_error(ALLOCATION_OUT_OF_POLICY));
        // Atomic: the swap is reverted with everything else.
        assert_eq!(holdings(&r), (10_000 * ONE, 0));
        assert_eq!(r.s.usdc.balance(&r.s.vault_id), 10_000 * ONE);
    }

    #[test]
    fn an_actual_result_inside_the_bands_is_accepted() {
        let r = setup_with(|s| usdc_eurc_policy_with(s, (8500, 9500), (500, 1500), "cash", 1000));
        let p = claim(&r, 9000, 1000, 1_000 * ONE);
        as_keeper(&r, &p);
        r.s.env.cost_estimate().budget().reset_unlimited();
        r.s.vault.rebalance(&p);
        let used = r.s.env.cost_estimate().resources();
        assert!(used.instructions < 100_000_000, "rebalance + post-trade check needs {} instructions", used.instructions);
        assert!(used.mem_bytes < 40 * 1024 * 1024, "rebalance + post-trade check needs {} bytes", used.mem_bytes);
        assert_eq!(holdings(&r).0, 9_000 * ONE);
    }

    #[test]
    fn an_actual_breach_of_a_band_maximum_is_refused_too() {
        // EURC may be at most 15 %; trading 20 % of the vault breaches it.
        let r = setup_with(|s| usdc_eurc_policy_with(s, (0, 10_000), (0, 1500), "cash", 1000));
        let p = claim(&r, 9000, 1000, 2_000 * ONE);
        as_keeper(&r, &p);
        r.s.env.cost_estimate().budget().reset_unlimited();
        assert_eq!(r.s.vault.try_rebalance(&p).unwrap_err().unwrap(), contract_error(ALLOCATION_OUT_OF_POLICY));
        assert_eq!(holdings(&r), (10_000 * ONE, 0));
    }

    #[test]
    fn the_liquidity_floor_is_checked_on_actual_holdings() {
        // EURC is not cash under this policy; cash (USDC) must stay >= 60 %.
        let r = setup_with(|s| usdc_eurc_policy_with(s, (0, 10_000), (0, 10_000), "eur_sovereign", 6000));
        // Claims 70/30 (floor met) but actually trades to ~50/50.
        let p = claim(&r, 7000, 3000, 5_000 * ONE);
        as_keeper(&r, &p);
        r.s.env.cost_estimate().budget().reset_unlimited();
        assert_eq!(r.s.vault.try_rebalance(&p).unwrap_err().unwrap(), contract_error(ALLOCATION_OUT_OF_POLICY));
        assert_eq!(holdings(&r), (10_000 * ONE, 0));
        // The same policy accepts a trade that keeps the floor (~70/30).
        let ok = claim(&r, 7000, 3000, 3_000 * ONE);
        as_keeper(&r, &ok);
        r.s.vault.rebalance(&ok);
        assert_eq!(holdings(&r).0, 7_000 * ONE);
    }

    /// Audit V4 regression on a REAL acquisition: once a sleeve holds EURC
    /// bought through Soroswap, losing its price stops every valuation path
    /// with OracleUnavailable - nothing falls back to par or to zero.
    #[test]
    fn a_funded_sleeve_without_a_valid_price_stops_valuation() {
        let r = setup();
        let p = proposal(&r, 1_000 * ONE, quote(&r, 1_000 * ONE));
        as_keeper(&r, &p);
        r.s.env.cost_estimate().budget().reset_unlimited();
        r.s.vault.rebalance(&p);
        assert!(holdings(&r).1 > 0, "EURC sleeve funded by a real swap");

        // Both EURC feeds go stale (staleness limit 1h); USDC is cash.
        r.s.env.ledger().set_timestamp(r.now + 3_601);
        let err = contract_error(ORACLE_UNAVAILABLE);
        assert_eq!(r.s.vault.try_total_assets().unwrap_err().unwrap(), err);
        assert_eq!(r.s.vault.try_get_redemption_capacity().unwrap_err().unwrap(), err);
        assert_eq!(r.s.vault.try_get_risk_sheet().unwrap_err().unwrap(), err);
        assert_eq!(r.s.vault.try_preview_deposit(&(100 * ONE)).unwrap_err().unwrap(), err);
        assert_eq!(r.s.vault.try_convert_to_assets(&ONE).unwrap_err().unwrap(), err);
        assert!(r.s.vault.get_nav_deposit().is_none() && r.s.vault.get_nav_conservative().is_none());
        r.s.env.mock_all_auths();
        let bob = Address::generate(&r.s.env);
        r.s.usdc_admin.mint(&bob, &(100 * ONE));
        assert!(r.s.vault.try_deposit(&(100 * ONE), &bob, &bob, &bob).is_err(), "no deposit at an unknown price");
    }
}
