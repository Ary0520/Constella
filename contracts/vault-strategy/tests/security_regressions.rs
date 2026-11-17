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
use constella_vault_strategy::{PolicySleeveTargetRef, SleeveState, SleeveType, StrategyVault, StrategyVaultClient, VaultConfig};
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

fn deploy_stack_cfg(vault_nav_feeds: bool, performance_fee_bps: u32) -> Stack<'static> {
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
            performance_fee_bps,
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
    usdc_eurc_policy_cfg(s, usdc_band, eurc_band, "cash", eurc_type, floor_bps, 10_000, 10_000)
}

/// Full control: USDC's policy sleeve type ("numeraire" = the vault
/// currency, decision D-A) and the single-asset / issuer caps (bps).
#[allow(clippy::too_many_arguments)]
fn usdc_eurc_policy_cfg(s: &Stack, usdc_band: (u32, u32), eurc_band: (u32, u32), usdc_type: &str, eurc_type: &str, floor_bps: u32,
    single_bps: u32, issuer_bps: u32) -> Address {
    let registry = AssetRegistryClient::new(&s.env, &s.registry_id);
    // Like Circle's real EURC issuer: revocable + clawback enabled, so balances
    // created afterwards (the vault's) can be clawed back (audit V6 test).
    let sac = s.env.register_stellar_asset_contract_v2(Address::generate(&s.env));
    sac.issuer().set_flag(soroban_sdk::testutils::IssuerFlags::RevocableFlag);
    sac.issuer().set_flag(soroban_sdk::testutils::IssuerFlags::ClawbackEnabledFlag);
    let eurc = sac.address();
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
    for (id, (lo, hi), kind) in [("USDC", usdc_band, usdc_type), ("EURC", eurc_band, eurc_type)] {
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
        &floor_bps, &single_bps, &issuer_bps, &50, &500, &targets,
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
        performance_fee_bps: 10,
        tvl_cap: 1_000_000 * ONE,
        per_user_cap: 100_000 * ONE,
    };
    let vault = env.register(StrategyVault, (legit.clone(), String::from_str(&env, "V"), String::from_str(&env, "V"), 7u32));
    let mut hostile = legit.clone();
    hostile.admin = attacker.clone();
    hostile.treasury = attacker.clone();
    hostile.performance_fee_bps = 1_000; // the maximum valid fee (D-D)
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
        performance_fee_bps: 0,
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
    // D-D: a fee only exists when the vault made a real profit - here EURC
    // bought through real Soroswap appreciates 2 % over a year.
    let r = dd::funded();
    let alice = r.depositor.clone();
    let bob = Address::generate(&r.s.env);
    assert_eq!(r.s.vault.balance(&r.s.treasury), 0);
    dd::price(&r, ONE * 11_016 / 10_000, r.s.vault.get_last_fee_accrual() + dd::YEAR);
    authorize(&r.s, &alice, "transfer", (alice.clone(), bob.clone(), ONE).into_val(&r.s.env));
    r.s.vault.transfer(&alice, &bob, &ONE);
    let events = vault_events(&r.s); // events of the transfer call only

    let fee = r.s.vault.balance(&r.s.treasury);
    assert!(fee > 0, "fee must have accrued");
    let mints: std::vec::Vec<_> = events.iter().filter(|(t, _)| t[0] == sym("mint")).cloned().collect();
    assert_eq!(mints, std::vec![(std::vec![sym("mint"), sc(&r.s.env, &r.s.treasury)], i128_val(fee))]);
    assert!(events.iter().any(|(t, _)| t[0] == sym("FEE_ACCRUAL")), "FEE_ACCRUAL announced");
    assert_eq!(
        r.s.vault.balance(&alice) + r.s.vault.balance(&bob) + fee,
        r.s.vault.total_supply(),
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
    // D-D: pending performance fees (real EURC appreciation between steps)
    // are priced into every preview exactly as execution mints them.
    let r = dd::funded();
    let a = r.depositor.clone();
    r.s.usdc_admin.mint(&a, &(10_000 * ONE));
    let mut px = ONE * 108 / 100;
    let mut later = |r: &R| {
        px = px * 10_100 / 10_000; // +1 % per step
        dd::price(r, px, r.s.env.ledger().timestamp() + 37 * 86_400);
    };
    later(&r);
    let p = r.s.vault.preview_deposit(&(1_000 * ONE));
    assert_eq!(r.s.vault.deposit(&(1_000 * ONE), &a, &a, &a), p, "preview_deposit");
    later(&r);
    let p = r.s.vault.preview_mint(&(500 * ONE));
    assert_eq!(r.s.vault.mint(&(500 * ONE), &a, &a, &a), p, "preview_mint");
    later(&r);
    let p = r.s.vault.preview_redeem(&(700 * ONE));
    assert_eq!(r.s.vault.redeem(&(700 * ONE), &a, &a, &a), p, "preview_redeem");
    later(&r);
    let p = r.s.vault.preview_withdraw(&(300 * ONE));
    assert_eq!(r.s.vault.withdraw(&(300 * ONE), &a, &a, &a), p, "preview_withdraw");
    assert!(r.s.vault.balance(&r.s.treasury) > 0, "fees must actually have accrued");
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

use w1::R;

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
        pub(super) depositor: Address,
        pub(super) now: u64,
    }

    /// Full stack + EURC sleeve + a real Soroswap USDC/EURC pool at 1.08 with
    /// 10.8M / 10M liquidity (deep enough that a 5_000 USDC trade stays inside
    /// the signed 0.5 % value slippage: 0.3 % fee + ~0.05 % impact), the router configured through the 48h delay,
    /// oracle feeds refreshed, a keeper, and 10_000 USDC deposited.
    pub(super) fn setup() -> R {
        setup_with(|s| usdc_eurc_policy(s))
    }

    /// `setup` with a caller-chosen signed policy (W1b band / floor tests).
    pub(super) fn setup_with(policy: impl FnOnce(&Stack<'static>) -> Address) -> R {
        setup_with_stack(deploy_stack(), policy)
    }

    /// `setup_with` on a caller-built stack (e.g. a vault with a fee, D-D).
    pub(super) fn setup_with_stack(s: Stack<'static>, policy: impl FnOnce(&Stack<'static>) -> Address) -> R {
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
        s.usdc_admin.mint(&lp, &(10_800_000 * ONE));
        eurc_admin.mint(&lp, &(10_000_000 * ONE));
        router.add_liquidity(&s.usdc_addr, &eurc, &(10_800_000 * ONE), &(10_000_000 * ONE), &0, &0, &lp, &(T0 + 10_000));

        // Router through the 48h delay (a router can move vault funds).
        // Fresh vault, no shares yet: the first router is immediate (D-K).
        s.vault.propose_router(&VaultRoute::Soroswap, &router.address);
        let now = T0 + DELAY;
        env.ledger().set_timestamp(now);
        // Feeds refreshed after the wait (their feeders push).
        for (id, px) in [("USDC_A", ONE), ("USDC_B", ONE), ("EURC_A", ONE * 108 / 100), ("EURC_B", ONE * 108 / 100)] {
            s.oracle.update_price(&Symbol::new(env, id), &px, &200_000, &now);
        }
        let keeper = Address::generate(env);
        s.vault.set_keeper(&keeper);
        let alice = Address::generate(env);
        s.usdc_admin.mint(&alice, &(10_000 * ONE));
        s.vault.deposit(&(10_000 * ONE), &alice, &alice, &alice);
        let r = R { s, eurc, eurc_admin, router, keeper, depositor: alice, now };
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
    const SLEEVE_TARGET_MISMATCH: u32 = 28;
    const DELAY_48H: u64 = 48 * 3600;

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

    const LIQUIDITY_FLOOR_BREACHED: u32 = 9;
    const EXCEEDS_REDEMPTION_CAPACITY: u32 = 8;

    /// Spec formula (CONTRACTS-SPEC redemption capacity): cash that can leave
    /// while cash / total stays >= floor.
    fn instant(cash: i128, total: i128, floor_bps: i128) -> i128 {
        (cash * 10_000 - total * floor_bps) / (10_000 - floor_bps)
    }

    /// Formerly src/test.rs `test_t46_liquidity_sleeve_floor_protection`
    /// (ignored since audit V2 because its holdings were fabricated). Now the
    /// 20 / 80 basket is bought through real Soroswap and every expected value
    /// is derived from the measured balances and oracle prices.
    #[test]
    fn the_liquidity_floor_limits_instant_redemptions_on_real_holdings() {
        let r = setup_with(|s| usdc_eurc_policy_with(s, (1000, 3000), (7000, 9000), "eur_sovereign", 1000));
        let p = claim(&r, 2000, 8000, 8_000 * ONE);
        as_keeper(&r, &p);
        r.s.env.cost_estimate().budget().reset_unlimited();
        r.s.vault.rebalance(&p);
        let (cash, eurc) = holdings(&r);
        assert_eq!(cash, 2_000 * ONE);
        let eurc_value = eurc * 108 / 100;
        let total = cash + eurc_value;
        assert_eq!(r.s.vault.total_assets(), total);
        let cap = r.s.vault.get_redemption_capacity();
        assert_eq!(cap.instant, instant(cash, total, 1000));
        assert_eq!(cap.in_24h, eurc_value, "EURC is a tier-0 sleeve");
        assert!(cap.instant > 1_000 * ONE && cap.instant < cash);

        r.s.env.mock_all_auths();
        let holder = holder(&r);
        // Inside instant capacity: paid.
        r.s.vault.withdraw(&(500 * ONE), &holder, &holder, &holder);
        let cash = cash - 500 * ONE;
        let total = total - 500 * ONE;
        let room = r.s.vault.get_redemption_capacity().instant;
        assert_eq!(room, instant(cash, total, 1000));
        // Beyond the floor but within cash: LiquidityFloorBreached.
        assert_eq!(
            r.s.vault.try_withdraw(&(room + ONE), &holder, &holder, &holder).unwrap_err().unwrap(),
            contract_error(LIQUIDITY_FLOOR_BREACHED)
        );
        // Beyond the cash itself: ExceedsRedemptionCapacity.
        assert_eq!(
            r.s.vault.try_withdraw(&(cash + 1), &holder, &holder, &holder).unwrap_err().unwrap(),
            contract_error(EXCEEDS_REDEMPTION_CAPACITY)
        );
        // The EURC feeds are visible through the registry lookup.
        assert!(r.s.vault.get_oracle_status().iter().any(|o| o.source_id == Symbol::new(&r.s.env, "EURC_A")));
    }

    /// Formerly `test_exact_liquidity_floor_boundary`: withdrawing exactly the
    /// instant capacity lands cash on the floor (to the stroop), after which
    /// capacity is 0 and even one more stroop is refused.
    #[test]
    fn redeeming_exactly_to_the_floor_leaves_zero_capacity() {
        let r = setup_with(|s| usdc_eurc_policy_with(s, (500, 1500), (8500, 9500), "eur_sovereign", 1000));
        let p = claim(&r, 1000, 9000, 9_000 * ONE);
        as_keeper(&r, &p);
        r.s.env.cost_estimate().budget().reset_unlimited();
        r.s.vault.rebalance(&p);
        r.s.env.mock_all_auths();
        let holder = holder(&r);
        let room = r.s.vault.get_redemption_capacity().instant;
        assert!(room > 0, "a little over 10 % cash after swap fees");
        assert_eq!(r.s.vault.max_withdraw(&holder), room);
        r.s.vault.withdraw(&room, &holder, &holder, &holder);

        let (cash, eurc) = holdings(&r);
        let total = cash + eurc * 108 / 100;
        assert!(cash * 10_000 >= total * 1000, "never below the floor");
        assert!(cash * 10_000 - total * 1000 < 9000, "on the floor to the stroop");
        assert_eq!(r.s.vault.get_redemption_capacity().instant, 0);
        assert_eq!(r.s.vault.max_withdraw(&holder), 0);
        assert_eq!(r.s.vault.max_redeem(&holder), 0);
        let floor = contract_error(LIQUIDITY_FLOOR_BREACHED);
        assert_eq!(r.s.vault.try_withdraw(&1, &holder, &holder, &holder).unwrap_err().unwrap(), floor);
        assert_eq!(r.s.vault.try_redeem(&ONE, &holder, &holder, &holder).unwrap_err().unwrap(), floor);
    }

    /// Formerly `test_dual_nav_asymmetric_max_redeem` (its 50 % feed spread is
    /// impossible since O5 caps the breaker at 5 %). After a real purchase the
    /// two EURC feeds diverge within the 2 % breaker: redemptions are priced at
    /// the conservative (minimum) NAV, so max_redeem never pays out more cash
    /// than the vault has - at the median NAV the same shares would.
    #[test]
    fn redemptions_use_the_conservative_nav_after_a_real_purchase() {
        let r = setup_with(|s| usdc_eurc_policy_with(s, (0, 10_000), (0, 10_000), "eur_sovereign", 0));
        let p = claim(&r, 5000, 5000, 5_000 * ONE);
        as_keeper(&r, &p);
        r.s.env.cost_estimate().budget().reset_unlimited();
        r.s.vault.rebalance(&p);
        r.s.env.mock_all_auths();
        // One EURC feeder now reports 1.065 (1.4 % below the other).
        r.s.oracle.update_price(&Symbol::new(&r.s.env, "EURC_B"), &(ONE * 1065 / 1000), &200_000, &r.now);
        let low = r.s.oracle.collateral_price(&r.eurc).unwrap().price;
        let med = r.s.oracle.asset_price(&r.eurc).unwrap().price;
        assert_eq!(low, ONE * 1065 / 1000);
        assert!(med > low);

        let (cash, eurc) = holdings(&r);
        assert_eq!(cash, 5_000 * ONE);
        let supply = r.s.vault.total_supply();
        let cons = (cash + eurc * low / ONE) * ONE / supply;
        let dep = (cash + eurc * med / ONE) * ONE / supply;
        assert_eq!(r.s.vault.get_nav_conservative().unwrap().price, cons);
        assert_eq!(r.s.vault.get_nav_deposit().unwrap().price, dep);

        let holder = holder(&r);
        assert_eq!(r.s.vault.get_redemption_capacity().instant, cash, "0 % floor: all cash");
        let max_red = r.s.vault.max_redeem(&holder);
        assert_eq!(max_red, cash * ONE / cons);
        assert!(max_red * dep / ONE > cash, "at the median NAV these shares would overdraw the cash");
        let out = r.s.vault.redeem(&max_red, &holder, &holder, &holder);
        assert!(out <= cash && out >= cash - 1, "paid at the conservative NAV: {out}");
    }

    /// Once a policy is bound, the vault's sleeve targets come from it: the
    /// admin can neither re-weight a sleeve nor add one the policy does not
    /// declare.
    #[test]
    fn sleeve_targets_must_match_the_signed_policy() {
        let r = setup();
        let env = &r.s.env;
        r.s.env.mock_all_auths();
        let current = r.s.vault.get_sleeves();
        // Re-weighting to 60 / 40 contradicts the signed 50 / 50.
        let mut reweighted = current.clone();
        let mut a = reweighted.get(0).unwrap();
        a.target_weight_bps = 6000;
        reweighted.set(0, a);
        let mut b = reweighted.get(1).unwrap();
        b.target_weight_bps = 4000;
        reweighted.set(1, b);
        assert_eq!(r.s.vault.try_set_sleeves(&reweighted).unwrap_err().unwrap(), contract_error(SLEEVE_TARGET_MISMATCH));
        // A registered asset the policy does not declare cannot become a sleeve.
        let other = r.s.env.register_stellar_asset_contract_v2(Address::generate(env)).address();
        AssetRegistryClient::new(env, &r.s.registry_id).add_asset(&AssetEntry {
            contract_address: other.clone(),
            asset_id: Symbol::new(env, "OTHER"),
            issuer: Symbol::new(env, "SOMEONE"),
            sleeve_type: RegSleeve::Cash,
            liquidity_tier: 0,
            permissionless: true,
            oracle_sources: Vec::from_array(env, [Symbol::new(env, "OTHER_A"), Symbol::new(env, "OTHER_B")]),
            eligible_jurisdictions: Vec::new(env),
            blocked_jurisdictions: Vec::new(env),
            investor_classes_required: Vec::new(env),
            is_active: true,
            added_at: T0,
        });
        let mut extra = current.clone();
        extra.push_back(SleeveState { asset_id: String::from_str(env, "OTHER"), contract_address: other, target_weight_bps: 0,
            band_bps: 0, current_balance: 0, sleeve_type: SleeveType::EurSovereign, liquidity_tier: 0 });
        assert_eq!(r.s.vault.try_set_sleeves(&extra).unwrap_err().unwrap(), contract_error(SLEEVE_TARGET_MISMATCH));
        // The layout that matches the signed targets is accepted.
        r.s.vault.set_sleeves(&current);
        assert_eq!(r.s.vault.get_sleeves(), current);
    }

    /// A policy amendment (owner-signed, after the 48 h notice) changes the
    /// targets the vault uses at once - nobody has to re-run set_sleeves.
    /// The empty-vault risk sheet is target-weighted, so it must follow the
    /// signed targets, and get_sleeves must report them.
    #[test]
    fn an_amended_policy_drives_the_vault_targets_without_set_sleeves() {
        let r = setup();
        let env = &r.s.env;
        r.s.env.mock_all_auths();
        // Empty the vault so the risk sheet weights by targets.
        let holder = holder(&r);
        r.s.vault.redeem(&r.s.vault.balance(&holder), &holder, &holder, &holder);
        assert_eq!(r.s.vault.total_assets(), 0);
        // Signed 50/50: USDC cash haircut 10 %, EURC (EurSovereign) 15 %.
        assert_eq!(r.s.vault.get_risk_sheet().recommended_haircut_bps, 1250);

        // The owner amends to 20/80; it takes effect after the notice period.
        let mut targets = Vec::new(env);
        for (id, w) in [("USDC", 2000u32), ("EURC", 8000u32)] {
            targets.push_back(SleeveTarget { asset_id: Symbol::new(env, id), target_weight_bps: w, min_band_bps: 0,
                max_band_bps: 10_000, sleeve_type: Symbol::new(env, "cash") });
        }
        r.s.policy.update_policy(&r.s.vault_id, &soroban_sdk::BytesN::from_array(env, &[0x78; 32]),
            &1000, &10_000, &10_000, &50, &500, &targets);
        env.ledger().set_timestamp(r.now + DELAY_48H + 1);
        r.s.policy.activate_policy_update(&r.s.vault_id);

        // 0.2 * 10 % + 0.8 * 15 % = 14 %.
        assert_eq!(r.s.vault.get_risk_sheet().recommended_haircut_bps, 1400);
        let shown = r.s.vault.get_sleeves();
        assert_eq!((shown.get(0).unwrap().target_weight_bps, shown.get(1).unwrap().target_weight_bps), (2000, 8000));
        // What get_sleeves shows is a layout set_sleeves accepts.
        r.s.vault.set_sleeves(&shown);
    }

    /// A sleeve the amended policy no longer declares has a target of 0.
    #[test]
    fn a_sleeve_dropped_by_an_amendment_has_a_zero_target() {
        let r = setup();
        let env = &r.s.env;
        r.s.env.mock_all_auths();
        let holder = holder(&r);
        r.s.vault.redeem(&r.s.vault.balance(&holder), &holder, &holder, &holder);
        let targets = Vec::from_array(env, [SleeveTarget { asset_id: Symbol::new(env, "USDC"), target_weight_bps: 10_000,
            min_band_bps: 0, max_band_bps: 10_000, sleeve_type: Symbol::new(env, "cash") }]);
        r.s.policy.update_policy(&r.s.vault_id, &soroban_sdk::BytesN::from_array(env, &[0x79; 32]),
            &1000, &10_000, &10_000, &50, &500, &targets);
        env.ledger().set_timestamp(r.now + DELAY_48H + 1);
        r.s.policy.activate_policy_update(&r.s.vault_id);
        let shown = r.s.vault.get_sleeves();
        assert_eq!((shown.get(0).unwrap().target_weight_bps, shown.get(1).unwrap().target_weight_bps), (10_000, 0));
        assert_eq!(r.s.vault.get_risk_sheet().recommended_haircut_bps, 1000, "all cash");
    }

    /// The setup's only depositor (holds every share).
    fn holder(r: &R) -> Address {
        let h = r.depositor.clone();
        assert_eq!(r.s.vault.balance(&h), r.s.vault.total_supply());
        h
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

/// Decision D-H / Blend adapter: the vault supplies USDC into a REAL Blend v2
/// pool (mainnet WASMs via constella-testkit), holds the position as bTokens,
/// values it at the oracle's live-b_rate price and withdraws it again - every
/// leg policy-validated, measured on the vault's own balances, and re-checked
/// against the signed policy afterwards.
mod blend_sleeve {
    use super::*;
    use constella_testkit::blend::{self, pool::Request, BORROW, RATE_SCALE, SUPPLY_COLLATERAL};
    use constella_vault_strategy::{RebalanceProposal as VP, TradeOperation as VT, TradeRoute as VR};

    const ROUTE_UNAVAILABLE: u32 = 24;
    const SLEEVE_HAS_HOLDINGS: u32 = 16;
    const NOW: u64 = 1_731_600_000;

    struct X {
        s: Stack<'static>,
        b: blend::BlendEnv<'static>,
        pool: Address,
        keeper: Address,
        alice: Address,
        owner: Address,
        eurc: Address,
    }

    fn setup() -> X {
        setup_with(false)
    }

    /// `with_eurc`: also an EURC sleeve (priced by two feeds, declared in the
    /// policy) - a token that is NOT the Blend position's underlying.
    fn setup_with(with_eurc: bool) -> X {
        let s = deploy_stack();
        let env = &s.env;
        env.cost_estimate().budget().reset_unlimited();
        env.ledger().set_timestamp(NOW); // Blend's backstop epoch is 2015
        let b = blend::deploy(env, &s.usdc_admin, &s.oracle.address);
        let pool = b.pool.address.clone();
        s.oracle.set_blend_factory(&b.factory.address);
        s.oracle.register_blend_position(&pool, &s.usdc_addr);
        AssetRegistryClient::new(env, &s.registry_id).add_asset(&AssetEntry {
            contract_address: pool.clone(),
            asset_id: Symbol::new(env, "BLEND_USDC_SUPPLY"),
            issuer: Symbol::new(env, "BLEND"),
            sleeve_type: RegSleeve::Lending,
            liquidity_tier: 1,
            permissionless: true,
            oracle_sources: Vec::from_array(env, [Symbol::new(env, "BLEND_B_RATE")]),
            eligible_jurisdictions: Vec::new(env),
            blocked_jurisdictions: Vec::new(env),
            investor_classes_required: Vec::new(env),
            is_active: true,
            added_at: NOW,
        });
        let eurc = s.env.register_stellar_asset_contract_v2(Address::generate(env)).address();
        let mut layout: std::vec::Vec<(&str, &str, u32)> = std::vec![("USDC", "cash", 5000), ("BLEND_USDC_SUPPLY", "lending", 5000)];
        if with_eurc {
            AssetRegistryClient::new(env, &s.registry_id).add_asset(&AssetEntry {
                contract_address: eurc.clone(),
                asset_id: Symbol::new(env, "EURC"),
                issuer: Symbol::new(env, "CIRCLE"),
                sleeve_type: RegSleeve::Cash,
                liquidity_tier: 0,
                permissionless: true,
                oracle_sources: Vec::from_array(env, [Symbol::new(env, "EURC_A"), Symbol::new(env, "EURC_B")]),
                eligible_jurisdictions: Vec::new(env),
                blocked_jurisdictions: Vec::new(env),
                investor_classes_required: Vec::new(env),
                is_active: true,
                added_at: NOW,
            });
            let key = Symbol::new(env, "EURC");
            s.oracle.configure_sleeve(&key, &200, &2);
            for id in ["EURC_A", "EURC_B"] {
                let mut src = oracle_source(env, id, &eurc, ONE * 108 / 100);
                src.sleeve = key.clone();
                src.timestamp = env.ledger().timestamp();
                s.oracle.register_source(&src);
            }
            layout = std::vec![("USDC", "cash", 4000), ("BLEND_USDC_SUPPLY", "lending", 3000), ("EURC", "eur_sovereign", 3000)];
        }
        let mut targets = Vec::new(env);
        for &(id, kind, w) in layout.iter() {
            targets.push_back(SleeveTarget { asset_id: Symbol::new(env, id), target_weight_bps: w, min_band_bps: 0,
                max_band_bps: 10_000, sleeve_type: Symbol::new(env, kind) });
        }
        let owner = Address::generate(env);
        s.policy.register_policy(&s.vault_id, &owner, &soroban_sdk::BytesN::from_array(env, &[0xb1; 32]), &s.registry_id,
            &1000, &10_000, &10_000, &50, &500, &targets);
        let w = |i: usize| layout[i].2;
        let mut sleeves = Vec::new(env);
        sleeves.push_back(SleeveState { asset_id: String::from_str(env, "USDC"), contract_address: s.usdc_addr.clone(),
            target_weight_bps: w(0), band_bps: 5000, current_balance: 0, sleeve_type: SleeveType::CashStable, liquidity_tier: 0 });
        sleeves.push_back(SleeveState { asset_id: String::from_str(env, "BLEND_USDC_SUPPLY"), contract_address: pool.clone(),
            target_weight_bps: w(1), band_bps: 5000, current_balance: 0, sleeve_type: SleeveType::Lending, liquidity_tier: 1 });
        if with_eurc {
            sleeves.push_back(SleeveState { asset_id: String::from_str(env, "EURC"), contract_address: eurc.clone(),
                target_weight_bps: w(2), band_bps: 5000, current_balance: 0, sleeve_type: SleeveType::EurSovereign, liquidity_tier: 0 });
        }
        s.vault.set_sleeves(&sleeves);
        let keeper = Address::generate(env);
        s.vault.set_keeper(&keeper);
        let alice = Address::generate(env);
        s.usdc_admin.mint(&alice, &(10_000 * ONE));
        s.vault.deposit(&(10_000 * ONE), &alice, &alice, &alice);
        X { s, b, pool, keeper, alice, owner, eurc }
    }

    fn leg(x: &X, asset_in: &Address, asset_out: &Address, amount_in: i128, min_out: i128, route: VR) -> VP {
        let env = &x.s.env;
        VP {
            vault: x.s.vault_id.clone(),
            policy_hash: x.s.policy.get_policy_hash(&x.s.vault_id),
            operations: Vec::from_array(env, [VT { asset_in: asset_in.clone(), asset_out: asset_out.clone(), amount_in,
                min_amount_out: min_out, route }]),
            weights_after: Vec::from_array(env, [(Symbol::new(env, "USDC"), 7000u32), (Symbol::new(env, "BLEND_USDC_SUPPLY"), 3000u32)]),
            deadline: env.ledger().timestamp() + 600,
            proposer: x.keeper.clone(),
        }
    }

    fn run(x: &X, p: &VP) -> Result<(), soroban_sdk::Error> {
        authorize(&x.s, &x.keeper, "rebalance", (p.clone(),).into_val(&x.s.env));
        match x.s.vault.try_rebalance(p) {
            Ok(_) => Ok(()),
            Err(e) => Err(e.unwrap()),
        }
    }

    fn b_tokens(x: &X) -> i128 {
        x.b.pool.get_positions(&x.s.vault_id).supply.get(x.b.usdc_index).unwrap_or(0)
    }

    fn booked(x: &X) -> (i128, i128) {
        let sl = x.s.vault.get_sleeves();
        (sl.get(0).unwrap().current_balance, sl.get(1).unwrap().current_balance)
    }

    fn supply_3000(x: &X) {
        let p = leg(x, &x.s.usdc_addr, &x.pool, 3_000 * ONE, 3_000 * ONE * 999 / 1000, VR::Blend);
        run(x, &p).unwrap();
    }

    /// A borrower creates real utilization (USDC collateral, USDC debt).
    fn borrow(x: &X, collateral: i128, debt: i128) {
        let env = &x.s.env;
        env.mock_all_auths();
        let who = Address::generate(env);
        x.s.usdc_admin.mint(&who, &collateral);
        x.b.pool.submit(&who, &who, &who, &Vec::from_array(env, [
            Request { request_type: SUPPLY_COLLATERAL, address: x.s.usdc_addr.clone(), amount: collateral },
            Request { request_type: BORROW, address: x.s.usdc_addr.clone(), amount: debt },
        ]));
    }

    #[test]
    fn a_supply_into_real_blend_is_measured_and_booked_in_btokens() {
        let x = setup();
        let p = leg(&x, &x.s.usdc_addr, &x.pool, 3_000 * ONE, 3_000 * ONE * 999 / 1000, VR::Blend);
        authorize(&x.s, &x.keeper, "rebalance", (p.clone(),).into_val(&x.s.env));
        x.s.vault.rebalance(&p);
        let used = x.s.env.cost_estimate().resources();
        assert!(used.instructions < 100_000_000, "{} instructions (mainnet tx limit 100M)", used.instructions);
        assert!(used.mem_bytes < 40 * 1024 * 1024, "{} bytes (mainnet tx limit 40 MB)", used.mem_bytes);
        assert!(x.s.env.auths().iter().all(|(a, _)| *a == x.keeper), "only the keeper signs");
        // Fresh reserve: 1 bToken = 1 USDC. Booked = the pool's own record.
        assert_eq!(b_tokens(&x), 3_000 * ONE);
        assert_eq!(booked(&x), (7_000 * ONE, 3_000 * ONE));
        assert_eq!(x.s.usdc.balance(&x.s.vault_id), 7_000 * ONE);
        assert!(x.b.pool.get_positions(&x.s.vault_id).collateral.is_empty(), "plain supply: never collateral");
        assert_eq!(x.s.vault.total_assets(), 10_000 * ONE);
    }

    /// Audit A3: Blend v2's supply rate, computed from the pool's live data
    /// with Blend's own interest model (pool/src/pool/interest.rs: three-part
    /// curve, kink at 95 %, ir_mod; supply = borrow x util x (1 - bstop_rate)),
    /// must match what the REAL pool then accrues. ceil/floor as in Blend.
    fn mul_ceil(a: i128, b: i128, s: i128) -> i128 { (a * b + s - 1) / s }
    fn div_ceil(a: i128, b: i128, s: i128) -> i128 { (a * s + b - 1) / b }
    const S7: i128 = 10_000_000;

    /// (borrow APR, supply APR), 7 decimals; the same function as
    /// services/ai-engine/src/yields.py (cross-checked by test vectors).
    fn blend_rates(c: &blend::pool::ReserveConfig, d: &blend::pool::ReserveData, bstop_rate: u32) -> (i128, i128) {
        let liabilities = mul_ceil(d.d_supply, d.d_rate, RATE_SCALE);
        let supply = d.b_supply * d.b_rate / RATE_SCALE;
        let util = if liabilities == 0 { 0 } else if liabilities >= supply { S7 } else { div_ceil(liabilities, supply, S7) };
        let target = c.util as i128;
        let (r0, r1, r2, r3) = (c.r_base as i128, c.r_one as i128, c.r_two as i128, c.r_three as i128);
        let borrow = if util <= target {
            mul_ceil(mul_ceil(div_ceil(util, target, S7), r1, S7) + r0, d.ir_mod, S7)
        } else if util <= 9_500_000 {
            mul_ceil(mul_ceil(div_ceil(util - target, 9_500_000 - target, S7), r2, S7) + r1 + r0, d.ir_mod, S7)
        } else {
            mul_ceil(div_ceil(util - 9_500_000, 500_000, S7), r3, S7) + mul_ceil(d.ir_mod, r2 + r1 + r0, S7)
        };
        let supply_apr = borrow * util / S7 * (S7 - bstop_rate as i128) / S7;
        (borrow, supply_apr)
    }

    #[test]
    fn the_blend_supply_rate_formula_matches_what_the_real_pool_accrues() {
        let x = setup();
        supply_3000(&x);
        borrow(&x, 20_000 * ONE, 10_000 * ONE);
        let env = &x.s.env;
        let r0 = x.b.pool.get_reserve(&x.s.usdc_addr);
        let bstop = x.b.pool.get_config().bstop_rate;
        let (borrow_apr, supply_apr) = blend_rates(&r0.config, &r0.data, bstop);
        assert!(supply_apr > 0 && supply_apr < borrow_apr);
        std::println!("VECTOR util_target={} r=({},{},{},{}) ir_mod={} b_rate={} d_rate={} b_supply={} d_supply={} bstop={} -> borrow={} supply={}",
            r0.config.util, r0.config.r_base, r0.config.r_one, r0.config.r_two, r0.config.r_three, r0.data.ir_mod, r0.data.b_rate,
            r0.data.d_rate, r0.data.b_supply, r0.data.d_supply, bstop, borrow_apr, supply_apr);
        let day = 86_400u64;
        env.ledger().set_timestamp(env.ledger().timestamp() + day);
        let r1 = x.b.pool.get_reserve(&x.s.usdc_addr);
        // Measured: the b_rate growth over one day, annualised (Blend accrues
        // simple interest per load, over a 365-day year).
        let measured = (r1.data.b_rate - r0.data.b_rate) * S7 * 31_536_000 / (r0.data.b_rate * day as i128);
        let err = (measured - supply_apr).abs() * 10_000 / supply_apr; // in bps of the rate
        assert!(err <= 1, "formula {supply_apr} vs real pool {measured} ({err} bps off)");
    }

    #[test]
    fn interest_earned_in_blend_raises_the_share_price() {
        let x = setup();
        supply_3000(&x);
        borrow(&x, 20_000 * ONE, 10_000 * ONE); // 15k power >= 13.3k
        x.s.env.ledger().set_timestamp(x.s.env.ledger().timestamp() + 30 * 24 * 3600);
        let rate = x.b.pool.get_reserve(&x.s.usdc_addr).data.b_rate;
        assert!(rate > RATE_SCALE);
        let price = ONE * rate / RATE_SCALE; // the oracle's price per bToken
        let expected = 7_000 * ONE + b_tokens(&x) * price / ONE;
        assert!(expected > 10_000 * ONE);
        assert_eq!(x.s.vault.total_assets(), expected);
        assert!(x.s.vault.convert_to_assets(&ONE) > ONE, "every share is worth more");
        // A further supply now buys FEWER bTokens than USDC; the books follow
        // the pool's own record exactly.
        let before = b_tokens(&x);
        let p = leg(&x, &x.s.usdc_addr, &x.pool, 1_000 * ONE, 1_000 * ONE * RATE_SCALE / rate * 999 / 1000, VR::Blend);
        run(&x, &p).unwrap();
        let bought = b_tokens(&x) - before;
        assert!(bought < 1_000 * ONE);
        assert_eq!(booked(&x), (6_000 * ONE, b_tokens(&x)));
    }

    #[test]
    fn a_withdraw_returns_the_position_with_its_interest_as_cash() {
        let x = setup();
        supply_3000(&x);
        borrow(&x, 20_000 * ONE, 10_000 * ONE);
        x.s.env.ledger().set_timestamp(x.s.env.ledger().timestamp() + 30 * 24 * 3600);
        let held = b_tokens(&x);
        let rate = x.b.pool.get_reserve(&x.s.usdc_addr).data.b_rate;
        let worth = held * rate / RATE_SCALE;
        let p = leg(&x, &x.pool, &x.s.usdc_addr, held, worth * 999 / 1000, VR::Blend);
        authorize(&x.s, &x.keeper, "rebalance", (p.clone(),).into_val(&x.s.env));
        x.s.vault.rebalance(&p);
        assert_eq!(b_tokens(&x), 0);
        let (cash, blend_booked) = booked(&x);
        assert_eq!(blend_booked, 0);
        let received = cash - 7_000 * ONE;
        assert!(received > 3_000 * ONE && received <= worth && received >= worth - 1, "{received} vs {worth}");
        assert_eq!(x.s.usdc.balance(&x.s.vault_id), cash);
    }

    /// Blend's liquidity risk is real: a withdraw that would push utilization
    /// to 100 % reverts inside Blend, and the whole rebalance with it.
    #[test]
    fn a_withdraw_the_pool_cannot_pay_reverts_atomically() {
        let x = setup();
        let env = &x.s.env;
        env.mock_all_auths();
        // EURC collateral priced by Constella's oracle (two feeds).
        let eurc = env.register_stellar_asset_contract_v2(Address::generate(env)).address();
        blend::add_reserve(env, &x.b, &eurc, 7);
        let sleeve = Symbol::new(env, "EURC");
        x.s.oracle.configure_sleeve(&sleeve, &200, &2);
        for (id, px) in [("EURC_A", 10_800_000i128), ("EURC_B", 10_700_000i128)] {
            let mut src = oracle_source(env, id, &eurc, px);
            src.sleeve = sleeve.clone();
            src.timestamp = env.ledger().timestamp();
            x.s.oracle.register_source(&src);
        }
        supply_3000(&x);
        // Borrow 95 % of the vault's 3,000 (Blend's max_util) against EURC.
        env.mock_all_auths();
        let who = Address::generate(env);
        token::StellarAssetClient::new(env, &eurc).mint(&who, &(10_000 * ONE));
        x.b.pool.submit(&who, &who, &who, &Vec::from_array(env, [
            Request { request_type: SUPPLY_COLLATERAL, address: eurc.clone(), amount: 10_000 * ONE },
            Request { request_type: BORROW, address: x.s.usdc_addr.clone(), amount: 2_850 * ONE },
        ]));
        let held = b_tokens(&x);
        let rate = x.b.pool.get_reserve(&x.s.usdc_addr).data.b_rate;
        let worth = |b: i128| b * rate / RATE_SCALE;
        // Fair value minus 0.1 % (inside the signed 0.5 %): the POLICY accepts,
        // and Blend itself refuses - utilization would reach 100 %.
        let all = leg(&x, &x.pool, &x.s.usdc_addr, held, worth(held) * 999 / 1000, VR::Blend);
        assert_eq!(run(&x, &all), Err(contract_error(1207)), "Blend InvalidUtilRate, not a policy rejection");
        assert_eq!(b_tokens(&x), held);
        assert_eq!(booked(&x), (7_000 * ONE, held));
        assert_eq!(x.s.usdc.balance(&x.s.vault_id), 7_000 * ONE);
        // What the pool can pay, it pays.
        let some = leg(&x, &x.pool, &x.s.usdc_addr, 100 * ONE, worth(100 * ONE) * 999 / 1000, VR::Blend);
        run(&x, &some).unwrap();
        assert!(booked(&x).0 >= 7_099 * ONE);
        assert_eq!(booked(&x).1, b_tokens(&x));
    }

    #[test]
    fn a_blend_position_is_never_routed_through_an_amm() {
        let x = setup();
        // A real Soroswap router IS configured (through the 48 h delay), so the
        // refusal comes from the Blend guard, not from a missing router.
        let _router = real_soroswap(&x, None);
        let p = leg(&x, &x.s.usdc_addr, &x.pool, 1_000 * ONE, 999 * ONE, VR::Soroswap);
        assert_eq!(run(&x, &p), Err(contract_error(ROUTE_UNAVAILABLE)));
        assert_eq!(booked(&x), (10_000 * ONE, 0));
    }

    /// A Blend leg must pair the position with ITS underlying: supplying "from
    /// EURC" would otherwise move the vault's USDC while booking the spend
    /// against the EURC sleeve.
    #[test]
    fn a_blend_leg_must_use_the_positions_own_underlying() {
        let x = setup_with(true);
        let env = &x.s.env;
        env.mock_all_auths();
        // The vault first buys real EURC through a real Soroswap pool.
        let router = real_soroswap(&x, Some(&x.eurc));
        for id in ["EURC_A", "EURC_B"] {
            x.s.oracle.update_price(&Symbol::new(env, id), &(ONE * 108 / 100), &200_000, &env.ledger().timestamp());
        }
        let path = Vec::from_array(env, [x.s.usdc_addr.clone(), x.eurc.clone()]);
        let out = router.router_get_amounts_out(&(1_000 * ONE), &path).get(1).unwrap();
        let mut buy = leg(&x, &x.s.usdc_addr, &x.eurc, 1_000 * ONE, out, VR::Soroswap);
        buy.weights_after = Vec::from_array(env, [(Symbol::new(env, "USDC"), 9000u32), (Symbol::new(env, "BLEND_USDC_SUPPLY"), 0u32),
            (Symbol::new(env, "EURC"), 1000u32)]);
        run(&x, &buy).unwrap();
        assert_eq!(x.s.vault.get_sleeves().get(2).unwrap().current_balance, out);

        // Now a policy-valid "supply from EURC" into the USDC position.
        let mut p = leg(&x, &x.eurc, &x.pool, 500 * ONE, 500 * ONE * 108 / 100 * 999 / 1000, VR::Blend);
        p.weights_after = buy.weights_after.clone();
        let usdc_before = x.s.usdc.balance(&x.s.vault_id);
        assert_eq!(run(&x, &p), Err(contract_error(ROUTE_UNAVAILABLE)));
        assert_eq!(x.s.usdc.balance(&x.s.vault_id), usdc_before, "no USDC moved");
        assert_eq!(b_tokens(&x), 0);
        assert_eq!(x.s.vault.get_sleeves().get(2).unwrap().current_balance, out, "EURC books untouched");

        // And a withdraw must pay out the position's underlying, not EURC.
        let mut fund = leg(&x, &x.s.usdc_addr, &x.pool, 1_000 * ONE, 999 * ONE, VR::Blend);
        fund.weights_after = buy.weights_after.clone();
        run(&x, &fund).unwrap();
        let held = b_tokens(&x);
        let mut out_wrong = leg(&x, &x.pool, &x.eurc, 500 * ONE, 500 * ONE * 100 / 108 * 999 / 1000, VR::Blend);
        out_wrong.weights_after = buy.weights_after.clone();
        let usdc_before = x.s.usdc.balance(&x.s.vault_id);
        assert_eq!(run(&x, &out_wrong), Err(contract_error(ROUTE_UNAVAILABLE)));
        assert_eq!(b_tokens(&x), held);
        assert_eq!(x.s.usdc.balance(&x.s.vault_id), usdc_before);
        assert_eq!(x.s.vault.get_sleeves().get(2).unwrap().current_balance, out, "EURC books untouched");
    }

    /// A real Soroswap factory + router, configured on the vault through the
    /// 48 h delay; with `pair`, a deep USDC/`pair` pool at 1.08.
    fn real_soroswap<'a>(x: &X, pair: Option<&Address>) -> soroswap_router::Client<'a> {
        let env = &x.s.env;
        env.mock_all_auths();
        let pair_hash = env.deployer().upload_contract_wasm(SOROSWAP_PAIR_WASM);
        let factory = soroswap_factory::Client::new(env, &env.register(soroswap_factory::WASM, ()));
        factory.initialize(&Address::generate(env), &pair_hash);
        let router = soroswap_router::Client::new(env, &env.register(soroswap_router::WASM, ()));
        router.initialize(&factory.address);
        if let Some(other) = pair {
            let lp = Address::generate(env);
            x.s.usdc_admin.mint(&lp, &(10_800_000 * ONE));
            token::StellarAssetClient::new(env, other).mint(&lp, &(10_000_000 * ONE));
            router.add_liquidity(&x.s.usdc_addr, other, &(10_800_000 * ONE), &(10_000_000 * ONE), &0, &0, &lp,
                &(env.ledger().timestamp() + 10_000));
        }
        x.s.vault.propose_router(&VR::Soroswap, &router.address);
        env.ledger().set_timestamp(env.ledger().timestamp() + 48 * 3600);
        x.s.vault.activate_router(&VR::Soroswap);
        router
    }

    /// Audit V6 with three sleeves (USDC cash, Blend position, EURC): when EURC
    /// is frozen, the Blend position is still priced, so a frozen redemption
    /// pays its share of cash + Blend in cash - limited by the cash actually
    /// held and by the liquidity floor over the priced part.
    #[test]
    fn v6_cash_and_floor_limits_apply_to_the_priced_part() {
        let x = setup_with(true);
        let env = &x.s.env;
        env.mock_all_auths();
        let router = real_soroswap(&x, Some(&x.eurc));
        for id in ["EURC_A", "EURC_B"] {
            x.s.oracle.update_price(&Symbol::new(env, id), &(ONE * 108 / 100), &200_000, &env.ledger().timestamp());
        }
        let w = |u: u32, b: u32, e: u32| Vec::from_array(env, [(Symbol::new(env, "USDC"), u), (Symbol::new(env, "BLEND_USDC_SUPPLY"), b),
            (Symbol::new(env, "EURC"), e)]);
        let mut fund = leg(&x, &x.s.usdc_addr, &x.pool, 3_000 * ONE, 2_997 * ONE, VR::Blend);
        fund.weights_after = w(5000, 3000, 2000);
        run(&x, &fund).unwrap();
        let path = Vec::from_array(env, [x.s.usdc_addr.clone(), x.eurc.clone()]);
        let out = router.router_get_amounts_out(&(2_000 * ONE), &path).get(1).unwrap();
        let mut buy = leg(&x, &x.s.usdc_addr, &x.eurc, 2_000 * ONE, out, VR::Soroswap);
        buy.weights_after = w(5000, 3000, 2000);
        run(&x, &buy).unwrap();
        // EURC feeds go stale; the Blend position (USDC underlying) stays priced.
        env.ledger().set_timestamp(env.ledger().timestamp() + 3_601);
        env.mock_all_auths();
        let alice = x.alice.clone();
        // 90 %: 0.9 x (5,000 cash + 3,000 Blend) = 7,200 > 5,000 cash held.
        assert_eq!(x.s.vault.try_redeem_frozen(&(9_000 * ONE), &alice, &alice, &alice).unwrap_err().unwrap(), contract_error(8));
        // 60 %: 4,800 cash leaves 200 cash of 3,200 priced = 6.25 % < 10 % floor.
        assert_eq!(x.s.vault.try_redeem_frozen(&(6_000 * ONE), &alice, &alice, &alice).unwrap_err().unwrap(), contract_error(9));
        // 20 %: 1,600 cash; Blend units are NOT claimed (still priced), EURC is.
        let got = x.s.vault.redeem_frozen(&(2_000 * ONE), &alice, &alice, &alice);
        assert_eq!(got.cash, 1_600 * ONE);
        assert_eq!(got.claims, Vec::from_array(env, [(x.eurc.clone(), out * 2_000 / 10_000)]));
        assert_eq!(b_tokens(&x), 3_000 * ONE, "Blend position untouched");
    }

    /// A funded Blend sleeve cannot be dropped from the layout, even after the
    /// policy stops declaring it: its bTokens are read from the pool itself.
    #[test]
    fn a_funded_blend_sleeve_cannot_be_removed() {
        let x = setup();
        supply_3000(&x);
        let env = &x.s.env;
        env.mock_all_auths();
        let usdc_only = Vec::from_array(env, [SleeveTarget { asset_id: Symbol::new(env, "USDC"), target_weight_bps: 10_000,
            min_band_bps: 0, max_band_bps: 10_000, sleeve_type: Symbol::new(env, "cash") }]);
        x.s.policy.update_policy(&x.s.vault_id, &soroban_sdk::BytesN::from_array(env, &[0xb2; 32]),
            &1000, &10_000, &10_000, &50, &500, &usdc_only);
        env.ledger().set_timestamp(env.ledger().timestamp() + 48 * 3600 + 1);
        x.s.policy.activate_policy_update(&x.s.vault_id);
        let _ = &x.owner;
        let mut cash_only = Vec::new(env);
        let mut usdc = x.s.vault.get_sleeves().get(0).unwrap();
        usdc.target_weight_bps = 10_000;
        cash_only.push_back(usdc);
        assert_eq!(x.s.vault.try_set_sleeves(&cash_only).unwrap_err().unwrap(), contract_error(SLEEVE_HAS_HOLDINGS));
        let _ = &x.alice;
    }
}

/// Audit V6: a funded sleeve without a valid price no longer traps every
/// holder. `redeem` stays halted (no fair share price); `redeem_frozen` pays
/// the priced part now and records the frozen units as a claim, settled in
/// cash at the conservative price once the asset is priced again. Real
/// holdings throughout (EURC bought through real Soroswap).
mod v6 {
    use super::w1::*;
    use super::*;

    const ORACLE_UNAVAILABLE: u32 = 10;
    const NO_FROZEN_SLEEVE: u32 = 29;
    const NO_CLAIM: u32 = 30;
    const EXCEEDS_REDEMPTION_CAPACITY: u32 = 8;

    /// W1 setup, buy `usdc_in` of EURC through real Soroswap, then let both
    /// EURC feeds go stale (limit 1 h). Returns (r, EURC held).
    fn frozen(usdc_in: i128) -> (R, i128) {
        let r = setup();
        let mut p = proposal(&r, usdc_in, quote(&r, usdc_in));
        let env = &r.s.env;
        let cash_bps = ((10_000 * ONE - usdc_in) * 10_000 / (10_000 * ONE)) as u32;
        p.weights_after = Vec::from_array(env, [(Symbol::new(env, "USDC"), cash_bps), (Symbol::new(env, "EURC"), 10_000 - cash_bps)]);
        as_keeper(&r, &p);
        env.cost_estimate().budget().reset_unlimited();
        r.s.vault.rebalance(&p);
        let held = holdings(&r).1;
        env.ledger().set_timestamp(r.now + 3_601);
        env.mock_all_auths();
        (r, held)
    }

    /// A1 (storage rent): a claim is money owed - its entries are renewed for
    /// 30 days whenever they are written or settled-against.
    #[test]
    fn a1_claim_entries_are_renewed_when_written() {
        use soroban_sdk::testutils::storage::Persistent as _;
        let (r, _) = frozen(3_000 * ONE);
        let alice = r.depositor.clone();
        r.s.vault.redeem_frozen(&(1_000 * ONE), &alice, &alice, &alice);
        let env = &r.s.env;
        env.as_contract(&r.s.vault.address, || {
            let claim = (Symbol::new(env, "Claim"), alice.clone(), r.eurc.clone());
            assert!(env.storage().persistent().get_ttl(&claim) >= 29 * 17_280);
            // Claim totals are instance data, renewed on every vault call.
        });
    }

    fn resume(r: &R, a: i128, b: i128) {
        let env = &r.s.env;
        let t = env.ledger().timestamp();
        r.s.oracle.update_price(&Symbol::new(env, "EURC_A"), &a, &200_000, &t);
        r.s.oracle.update_price(&Symbol::new(env, "EURC_B"), &b, &200_000, &t);
    }

    #[test]
    fn redeem_stays_halted_but_redeem_frozen_pays_the_priced_part_and_records_a_claim() {
        let (r, held) = frozen(3_000 * ONE);
        let alice = r.depositor.clone();
        assert_eq!(r.s.vault.try_redeem(&(1_000 * ONE), &alice, &alice, &alice).unwrap_err().unwrap(), contract_error(ORACLE_UNAVAILABLE));
        assert!(r.s.vault.try_deposit(&ONE, &alice, &alice, &alice).is_err(), "deposits stay halted: no fair price");

        let units = held * 1_000 / 10_000;
        let preview = r.s.vault.preview_redeem_frozen(&(1_000 * ONE));
        assert_eq!(preview.cash, 700 * ONE, "10 % of the priced part: 7,000 USDC");
        assert_eq!(preview.claims, Vec::from_array(&r.s.env, [(r.eurc.clone(), units)]));
        let got = r.s.vault.redeem_frozen(&(1_000 * ONE), &alice, &alice, &alice);
        assert_eq!(got, preview);

        assert_eq!(r.s.usdc.balance(&alice), 700 * ONE);
        assert_eq!(r.s.vault.balance(&alice), 9_000 * ONE);
        assert_eq!(r.s.vault.get_claim(&alice, &r.eurc), units);
        assert_eq!(r.s.vault.get_claim_total(&r.eurc), units);
        assert_eq!(holdings(&r), (6_300 * ONE, held - units), "claimed units left the sleeve's books");
        assert_eq!(token::Client::new(&r.s.env, &r.eurc).balance(&r.s.vault_id), held, "but are still held for the claim");
    }

    #[test]
    fn remaining_holders_keep_their_per_share_value() {
        let (r, held) = frozen(3_000 * ONE);
        let per_share = |cash: i128, eurc: i128, supply: i128| (cash + eurc * 108 / 100) * ONE / supply;
        let before = per_share(7_000 * ONE, held, 10_000 * ONE);
        let alice = r.depositor.clone();
        r.s.vault.redeem_frozen(&(2_500 * ONE), &alice, &alice, &alice);
        resume(&r, ONE * 108 / 100, ONE * 108 / 100);
        let after = r.s.vault.total_assets() * ONE / r.s.vault.total_supply();
        assert_eq!(after, per_share(holdings(&r).0, holdings(&r).1, r.s.vault.total_supply()));
        assert!(after >= before && after - before <= 1, "before {before}, after {after}");
    }

    #[test]
    fn a_claim_settles_in_cash_at_the_conservative_price_once_priced_again() {
        let (r, held) = frozen(3_000 * ONE);
        let alice = r.depositor.clone();
        r.s.vault.redeem_frozen(&(1_000 * ONE), &alice, &alice, &alice);
        let units = held * 1_000 / 10_000;
        assert_eq!(r.s.vault.try_settle_claim(&alice, &r.eurc).unwrap_err().unwrap(), contract_error(ORACLE_UNAVAILABLE));
        let bob = Address::generate(&r.s.env);
        assert_eq!(r.s.vault.try_settle_claim(&bob, &r.eurc).unwrap_err().unwrap(), contract_error(NO_CLAIM));

        resume(&r, ONE * 108 / 100, ONE * 107 / 100);
        // Permissionless: nobody signs; the vault pays only the holder.
        r.s.env.set_auths(&[]);
        let paid = r.s.vault.settle_claim(&alice, &r.eurc);
        assert_eq!(paid, units * (ONE * 107 / 100) / ONE, "conservative (minimum) price");
        assert_eq!(r.s.usdc.balance(&alice), 700 * ONE + paid);
        assert_eq!(r.s.vault.get_claim(&alice, &r.eurc), 0);
        assert_eq!(r.s.vault.get_claim_total(&r.eurc), 0);
        assert_eq!(holdings(&r), (6_300 * ONE - paid, held), "units back on the books, cash out");
        assert_eq!(r.s.vault.try_settle_claim(&alice, &r.eurc).unwrap_err().unwrap(), contract_error(NO_CLAIM), "once");
    }

    #[test]
    fn a_paused_vault_neither_redeems_frozen_nor_settles() {
        let (r, _) = frozen(3_000 * ONE);
        let alice = r.depositor.clone();
        r.s.vault.redeem_frozen(&(1_000 * ONE), &alice, &alice, &alice);
        resume(&r, ONE * 108 / 100, ONE * 108 / 100);
        r.s.vault.pause();
        assert_eq!(r.s.vault.try_redeem_frozen(&ONE, &alice, &alice, &alice).unwrap_err().unwrap(), contract_error(4));
        assert_eq!(r.s.vault.try_settle_claim(&alice, &r.eurc).unwrap_err().unwrap(), contract_error(4));
    }

    #[test]
    fn redeem_frozen_is_only_for_a_vault_with_a_frozen_sleeve() {
        let r = setup();
        r.s.env.mock_all_auths();
        let alice = r.depositor.clone();
        assert_eq!(r.s.vault.try_redeem_frozen(&ONE, &alice, &alice, &alice).unwrap_err().unwrap(), contract_error(NO_FROZEN_SLEEVE));
    }

    #[test]
    fn only_the_owner_or_an_approved_operator_can_redeem_frozen() {
        let (r, _) = frozen(3_000 * ONE);
        let alice = r.depositor.clone();
        let mallory = Address::generate(&r.s.env);
        // Mallory signs for herself, but has no allowance on Alice's shares.
        authorize(&r.s, &mallory, "redeem_frozen", (ONE, mallory.clone(), alice.clone(), mallory.clone()).into_val(&r.s.env));
        assert_eq!(r.s.vault.try_redeem_frozen(&ONE, &mallory, &alice, &mallory).unwrap_err().unwrap(), contract_error(7),
            "InsufficientAllowance");
        // Nor can she act as Alice without Alice's signature.
        authorize(&r.s, &mallory, "redeem_frozen", (ONE, mallory.clone(), alice.clone(), alice.clone()).into_val(&r.s.env));
        assert!(r.s.vault.try_redeem_frozen(&ONE, &mallory, &alice, &alice).is_err());
        assert_eq!(r.s.vault.balance(&alice), 10_000 * ONE);
        assert_eq!(r.s.vault.get_claim(&mallory, &r.eurc), 0);
    }

    #[test]
    fn a_settlement_the_cash_cannot_cover_waits_with_the_claim_intact() {
        let (r, held) = frozen(9_000 * ONE);
        let alice = r.depositor.clone();
        let got = r.s.vault.redeem_frozen(&(9_000 * ONE), &alice, &alice, &alice);
        assert_eq!(got.cash, 900 * ONE);
        resume(&r, ONE * 108 / 100, ONE * 107 / 100);
        let units = held * 9_000 / 10_000;
        assert_eq!(r.s.vault.try_settle_claim(&alice, &r.eurc).unwrap_err().unwrap(), contract_error(EXCEEDS_REDEMPTION_CAPACITY));
        assert_eq!(r.s.vault.get_claim(&alice, &r.eurc), units);
        assert_eq!(holdings(&r), (100 * ONE, held - units));
    }

    /// If the issuer claws tokens back while claims are outstanding, the
    /// settlement is capped pro rata to the reserved units still held - the
    /// remaining holders never pay cash for units that are gone.
    #[test]
    fn a_clawback_meanwhile_caps_the_settlement_pro_rata() {
        let (r, held) = frozen(3_000 * ONE);
        let alice = r.depositor.clone();
        r.s.vault.redeem_frozen(&(5_000 * ONE), &alice, &alice, &alice);
        let units = held * 5_000 / 10_000;
        let booked = holdings(&r).1;
        r.eurc_admin.clawback(&r.s.vault_id, &(units / 2));
        resume(&r, ONE * 108 / 100, ONE * 107 / 100);
        assert_eq!(holdings(&r).1, booked, "the remaining holders' units are untouched");
        let present = held - units / 2 - booked;
        let paid_units = units * present / units;
        let paid = r.s.vault.settle_claim(&alice, &r.eurc);
        assert_eq!(paid, paid_units * (ONE * 107 / 100) / ONE);
        assert!(paid_units < units);
        let real = token::Client::new(&r.s.env, &r.eurc).balance(&r.s.vault_id);
        assert_eq!(holdings(&r).1, real, "books equal the real balance after settlement");
        assert_eq!(r.s.vault.get_claim_total(&r.eurc), 0);
    }
}

/// Decision D-D: holders pay 10 % of PROFIT above the per-share high-water
/// mark (conservative NAV), never more than 0.20 %/yr of assets, at most once
/// per day, and nothing when the vault earns nothing. Real profit: EURC bought
/// through a real Soroswap pool, then its two price feeds move.
mod dd {
    use super::w1::*;
    use super::*;

    pub(super) const FEE_BPS: u32 = 1_000;
    pub(super) const YEAR: u64 = 31_536_000;
    const DAY: u64 = 86_400;

    /// W1 stack with a 10 % performance fee; 5,000 USDC of EURC bought.
    pub(super) fn funded() -> R {
        let r = setup_with_stack(deploy_stack_cfg(true, FEE_BPS), usdc_eurc_policy);
        let env = &r.s.env;
        let mut p = proposal(&r, 5_000 * ONE, quote(&r, 5_000 * ONE));
        p.weights_after = Vec::from_array(env, [(Symbol::new(env, "USDC"), 5000u32), (Symbol::new(env, "EURC"), 5000u32)]);
        as_keeper(&r, &p);
        env.cost_estimate().budget().reset_unlimited();
        r.s.vault.rebalance(&p);
        env.mock_all_auths();
        r
    }

    /// Both EURC feeds report `px` at time `at` (ledger moved there).
    pub(super) fn price(r: &R, px: i128, at: u64) {
        let env = &r.s.env;
        env.ledger().set_timestamp(at);
        env.mock_all_auths();
        for id in ["EURC_A", "EURC_B"] {
            r.s.oracle.update_price(&Symbol::new(env, id), &px, &200_000, &at);
        }
    }

    /// Independent model of the D-D formula from on-chain state:
    /// (fee shares, high-water mark afterwards, fee by profit, cap).
    fn model(r: &R, px: i128, elapsed: u64) -> (i128, i128, i128, i128) {
        let (cash, eurc) = holdings(r);
        let total = cash + eurc * px / ONE;
        let supply = r.s.vault.total_supply();
        let hwm = r.s.vault.get_high_water_mark();
        let nav = total * ONE / supply;
        if nav <= hwm {
            return (0, hwm, 0, 0);
        }
        let by_profit = (nav - hwm) * supply / ONE * FEE_BPS as i128 / 10_000;
        let cap = total * 20 * elapsed as i128 / (10_000 * YEAR as i128);
        let fee = by_profit.min(cap);
        let shares = fee * supply / (total - fee);
        (shares, total * ONE / (supply + shares), by_profit, cap)
    }

    #[test]
    fn profit_pays_ten_percent_when_the_cap_does_not_bind() {
        let r = funded();
        let last = r.s.vault.get_last_fee_accrual();
        let px = ONE * 11_016 / 10_000; // EURC +2 % (the minimum)
        price(&r, px, last + YEAR);
        // The other feed reports higher: profit is measured on the MINIMUM,
        // so a single high (or manipulated) source cannot create a fee.
        r.s.oracle.update_price(&Symbol::new(&r.s.env, "EURC_B"), &(ONE * 11_070 / 10_000), &200_000, &(last + YEAR));
        let (shares, hwm, by_profit, cap) = model(&r, px, YEAR);
        assert!(by_profit > 0 && by_profit < cap, "profit-bound case: {by_profit} < {cap}");
        assert_eq!(r.s.vault.accrue_fees(), shares);
        assert_eq!(r.s.vault.balance(&r.s.treasury), shares);
        assert_eq!(r.s.vault.get_high_water_mark(), hwm);
        // The treasury's shares are worth 10 % of the profit: never more, and
        // less only by rounding in the holders' favour (NAV per share floors to
        // 7 decimals, times ~shares/ONE units, plus the share floor).
        let worth = r.s.vault.convert_to_assets(&shares);
        assert!(worth <= by_profit && by_profit - worth <= shares / ONE + 2, "worth {worth} vs fee {by_profit}");
    }

    #[test]
    fn the_annual_cap_binds_on_fast_profit_and_the_rest_is_forgiven() {
        let r = funded();
        let last = r.s.vault.get_last_fee_accrual();
        let px = ONE * 11_016 / 10_000;
        price(&r, px, last + 2 * DAY);
        let (shares, hwm, by_profit, cap) = model(&r, px, 2 * DAY);
        assert!(cap < by_profit, "cap-bound case: {cap} < {by_profit}");
        assert_eq!(r.s.vault.accrue_fees(), shares);
        assert_eq!(r.s.vault.get_high_water_mark(), hwm);
        // Same price a day later: nothing more is charged for that profit.
        price(&r, px, last + 3 * DAY);
        assert_eq!(r.s.vault.accrue_fees(), 0);
    }

    #[test]
    fn no_fee_below_the_high_water_mark_and_only_on_new_highs() {
        let r = funded();
        let last = r.s.vault.get_last_fee_accrual();
        let up = ONE * 11_016 / 10_000;
        price(&r, up, last + YEAR);
        assert!(r.s.vault.accrue_fees() > 0);
        let treasury = r.s.vault.balance(&r.s.treasury);
        // Down 2 %, then back to the old peak: no fee either time.
        price(&r, ONE * 108 / 100, last + YEAR + DAY);
        assert_eq!(r.s.vault.accrue_fees(), 0);
        price(&r, up, last + YEAR + 2 * DAY);
        assert_eq!(r.s.vault.accrue_fees(), 0, "recovering a loss is not profit");
        // Above the peak: a fee on the excess only.
        let higher = ONE * 11_124 / 10_000;
        price(&r, higher, last + 2 * YEAR);
        let (shares, _, _, _) = model(&r, higher, YEAR - 2 * DAY);
        assert!(shares > 0);
        assert_eq!(r.s.vault.accrue_fees(), shares);
        assert_eq!(r.s.vault.balance(&r.s.treasury), treasury + shares);
    }

    #[test]
    fn fees_crystallize_at_most_once_a_day() {
        let r = funded();
        let last = r.s.vault.get_last_fee_accrual();
        price(&r, ONE * 11_016 / 10_000, last + YEAR);
        assert!(r.s.vault.accrue_fees() > 0);
        let at = r.s.vault.get_last_fee_accrual();
        price(&r, ONE * 11_124 / 10_000, at + 3_600);
        assert_eq!(r.s.vault.accrue_fees(), 0, "no second fee within the day");
        assert_eq!(r.s.vault.get_last_fee_accrual(), at);
        price(&r, ONE * 11_124 / 10_000, at + DAY);
        assert!(r.s.vault.accrue_fees() > 0);
    }

    #[test]
    fn no_fee_while_a_funded_sleeve_is_unpriced_and_that_time_is_forfeited() {
        let r = funded();
        let last = r.s.vault.get_last_fee_accrual();
        // A year with stale EURC feeds: no fee, no panic; the clock moves on.
        r.s.env.ledger().set_timestamp(last + YEAR);
        assert_eq!(r.s.vault.accrue_fees(), 0);
        assert_eq!(r.s.vault.get_last_fee_accrual(), last + YEAR);
        // Priced again a day later: the cap covers only that day.
        let px = ONE * 11_016 / 10_000;
        price(&r, px, last + YEAR + DAY);
        let (shares, _, by_profit, cap) = model(&r, px, DAY);
        assert!(cap < by_profit);
        assert_eq!(r.s.vault.accrue_fees(), shares);
    }

    #[test]
    fn a_vault_that_earns_nothing_pays_nothing() {
        let s = deploy_stack_cfg(true, FEE_BPS);
        let alice = Address::generate(&s.env);
        s.usdc_admin.mint(&alice, &(1_000 * ONE));
        s.vault.deposit(&(1_000 * ONE), &alice, &alice, &alice);
        s.env.ledger().set_timestamp(T0 + YEAR);
        assert_eq!(s.vault.accrue_fees(), 0);
        assert_eq!(s.vault.balance(&s.treasury), 0);
        assert_eq!(s.vault.total_supply(), 1_000 * ONE);
    }

    #[test]
    fn the_fee_rate_is_capped_at_ten_percent_in_the_constructor() {
        let ok = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| deploy_stack_cfg(true, FEE_BPS)));
        assert!(ok.is_ok());
        let too_high = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| deploy_stack_cfg(true, FEE_BPS + 1)));
        assert!(too_high.is_err(), "a fee above 10 % of profit is refused");
    }
}

/// Decision D-A / audit A1b: the vault currency, declared in the signed
/// policy as the "numeraire" sleeve, is exempt from the concentration caps on
/// chain - so the AI engine's Cash Reserve allocation (>= 85 % USDC) passes
/// both policy checks, while EURC still counts toward Circle.
mod a1b {
    use super::w1::*;
    use super::*;

    const ALLOCATION_OUT_OF_POLICY: u32 = 27;
    const REBALANCE_REJECTED: u32 = 23;
    const INVALID_SLEEVE_CONFIG: u32 = 17;

    /// Real caps: 25 % single asset, 30 % issuer, 10 % floor.
    fn cash_reserve(usdc_type: &'static str) -> R {
        setup_with(move |s| usdc_eurc_policy_cfg(s, (0, 10_000), (0, 10_000), usdc_type, "cash", 1000, 2500, 3000))
    }

    fn buy_eurc(r: &R, usdc_in: i128) -> Result<(), soroban_sdk::Error> {
        let env = &r.s.env;
        let mut p = proposal(r, usdc_in, quote(r, usdc_in));
        let cash = ((10_000 * ONE - usdc_in) * 10_000 / (10_000 * ONE)) as u32;
        p.weights_after = Vec::from_array(env, [(Symbol::new(env, "USDC"), cash), (Symbol::new(env, "EURC"), 10_000 - cash)]);
        as_keeper(r, &p);
        env.cost_estimate().budget().reset_unlimited();
        match r.s.vault.try_rebalance(&p) {
            Ok(_) => Ok(()),
            Err(e) => Err(e.unwrap()),
        }
    }

    #[test]
    fn a_cash_reserve_rebalance_passes_on_chain_with_the_currency_declared() {
        let r = cash_reserve("numeraire");
        buy_eurc(&r, 1_500 * ONE).unwrap(); // 85 % USDC / ~15 % EURC
        assert_eq!(holdings(&r).0, 8_500 * ONE);
    }

    #[test]
    fn without_the_declaration_the_same_allocation_is_refused() {
        let r = cash_reserve("cash");
        assert_eq!(buy_eurc(&r, 1_500 * ONE), Err(contract_error(REBALANCE_REJECTED)), "85 % USDC breaches the 25 % cap");
        assert_eq!(holdings(&r), (10_000 * ONE, 0));
    }

    #[test]
    fn eurc_still_counts_toward_circle_on_chain() {
        // 35 % EURC (> 30 % Circle cap; the single-asset cap is lifted so the
        // issuer cap alone decides) - claim says 70/30, actual ~65/35.
        let r = setup_with(|s| usdc_eurc_policy_cfg(s, (0, 10_000), (0, 10_000), "numeraire", "cash", 1000, 10_000, 3000));
        let env = &r.s.env;
        let mut p = proposal(&r, 3_500 * ONE, quote(&r, 3_500 * ONE));
        p.weights_after = Vec::from_array(env, [(Symbol::new(env, "USDC"), 7000u32), (Symbol::new(env, "EURC"), 3000u32)]);
        as_keeper(&r, &p);
        env.cost_estimate().budget().reset_unlimited();
        assert_eq!(r.s.vault.try_rebalance(&p).unwrap_err().unwrap(), contract_error(ALLOCATION_OUT_OF_POLICY));
        assert_eq!(holdings(&r), (10_000 * ONE, 0));
    }

    #[test]
    fn the_vault_binds_only_its_own_currency_as_the_numeraire() {
        let s = deploy_stack();
        let env = &s.env;
        let eurc = env.register_stellar_asset_contract_v2(Address::generate(env)).address();
        AssetRegistryClient::new(env, &s.registry_id).add_asset(&AssetEntry {
            contract_address: eurc, asset_id: Symbol::new(env, "EURC"), issuer: Symbol::new(env, "CIRCLE"),
            sleeve_type: RegSleeve::Cash, liquidity_tier: 0, permissionless: true,
            oracle_sources: Vec::from_array(env, [Symbol::new(env, "A"), Symbol::new(env, "B")]),
            eligible_jurisdictions: Vec::new(env), blocked_jurisdictions: Vec::new(env),
            investor_classes_required: Vec::new(env), is_active: true, added_at: T0,
        });
        let sleeves = |cur: &str, kind: &str| Vec::from_array(env, [
            PolicySleeveTargetRef { asset_id: Symbol::new(env, cur), target_weight_bps: 10_000, min_band_bps: 0, max_band_bps: 10_000,
                sleeve_type: Symbol::new(env, kind) }]);
        env.mock_all_auths_allowing_non_root_auth();
        let owner = Address::generate(env);
        let h = soroban_sdk::BytesN::from_array(env, &[0xd1; 32]);
        // EURC is not this USDC vault's currency.
        assert_eq!(s.vault.try_bind_policy(&owner, &h, &1000, &2500, &3000, &50, &500, &sleeves("EURC", "numeraire")).unwrap_err().unwrap(),
            contract_error(INVALID_SLEEVE_CONFIG));
        s.vault.bind_policy(&owner, &h, &1000, &2500, &3000, &50, &500, &sleeves("USDC", "numeraire"));
        assert!(s.vault.get_policy_hash().is_some());
    }
}

/// Decision D-K: a vault's FIRST router takes effect immediately while no
/// shares exist (nobody to protect yet, e.g. a fresh testnet or factory
/// vault); once anyone holds shares - or once a router was ever set - every
/// change waits 48 h.
mod dk {
    use super::*;
    use constella_vault_strategy::TradeRoute as VaultRoute;

    #[test]
    fn the_first_router_is_immediate_while_the_vault_has_no_shares() {
        let s = deploy_stack();
        s.env.mock_all_auths();
        let router = Address::generate(&s.env);
        s.vault.propose_router(&VaultRoute::Soroswap, &router);
        // Set in the same call: no pending window for a depositor to fall into.
        assert_eq!(s.vault.get_router(&VaultRoute::Soroswap), Some(router.clone()));
        assert!(s.vault.try_activate_router(&VaultRoute::Soroswap).is_err(), "nothing pending");
        // A second change (still no shares) waits: a router was already set.
        let other = Address::generate(&s.env);
        s.vault.propose_router(&VaultRoute::Soroswap, &other);
        assert!(s.vault.try_activate_router(&VaultRoute::Soroswap).is_err());
        assert_eq!(s.vault.get_router(&VaultRoute::Soroswap), Some(router));
    }

    #[test]
    fn once_anyone_holds_shares_even_the_first_router_waits_48_hours() {
        let s = deploy_stack();
        s.env.mock_all_auths();
        let user = Address::generate(&s.env);
        s.usdc_admin.mint(&user, &(100 * ONE));
        s.vault.deposit(&(100 * ONE), &user, &user, &user);
        let router = Address::generate(&s.env);
        s.vault.propose_router(&VaultRoute::Soroswap, &router);
        assert!(s.vault.try_activate_router(&VaultRoute::Soroswap).is_err());
        s.env.ledger().set_timestamp(s.env.ledger().timestamp() + 48 * 3600);
        s.vault.activate_router(&VaultRoute::Soroswap);
        assert_eq!(s.vault.get_router(&VaultRoute::Soroswap), Some(router));
    }
}

/// Audit A3 (real yield numbers): the vault records its own conservative NAV
/// per share at most once a day (bounded history), so realised yield comes
/// from the chain itself - never an invented number.
mod a3 {
    use super::w1::*;
    use super::*;

    fn with_eurc() -> R {
        let r = setup();
        let env = &r.s.env;
        let usdc_in = 3_000 * ONE;
        let mut p = proposal(&r, usdc_in, quote(&r, usdc_in));
        p.weights_after = Vec::from_array(env, [(Symbol::new(env, "USDC"), 7000u32), (Symbol::new(env, "EURC"), 3000u32)]);
        as_keeper(&r, &p);
        env.cost_estimate().budget().reset_unlimited();
        r.s.vault.rebalance(&p);
        env.mock_all_auths();
        r
    }

    fn next_day(r: &R, eurc_px: i128) {
        let env = &r.s.env;
        env.ledger().with_mut(|l| { l.timestamp += 86_400; l.sequence_number += 17_280; });
        let t = env.ledger().timestamp();
        // The two EURC feeds differ by 0.5 % (inside the deviation limit), so
        // the conservative NAV (minimum) differs from the deposit NAV (median).
        for (id, px) in [("USDC_A", ONE), ("USDC_B", ONE), ("EURC_A", eurc_px), ("EURC_B", eurc_px + eurc_px / 200)] {
            r.s.oracle.update_price(&Symbol::new(env, id), &px, &200_000, &t);
        }
    }

    #[test]
    fn one_conservative_nav_point_per_day_from_the_chain() {
        let r = with_eurc();
        assert!(r.s.vault.get_nav_history().is_empty());
        let mut px = ONE * 108 / 100;
        for _ in 0..3 {
            next_day(&r, px);
            r.s.vault.accrue_fees();
            r.s.vault.accrue_fees(); // same day: no second point
            px += ONE / 100;
        }
        let h = r.s.vault.get_nav_history();
        assert_eq!(h.len(), 3);
        let last = h.last().unwrap();
        assert_eq!(last.timestamp, r.s.env.ledger().timestamp());
        assert_eq!(last.nav, r.s.vault.get_nav_conservative().unwrap().price, "the vault's own conservative NAV");
        assert!(h.get(0).unwrap().nav < h.get(1).unwrap().nav && h.get(1).unwrap().nav < h.get(2).unwrap().nav, "EURC rose");
    }

    #[test]
    fn unpriced_days_record_nothing_and_history_is_bounded() {
        let r = with_eurc();
        let env = &r.s.env;
        // A day with stale feeds (no push): no point.
        env.ledger().with_mut(|l| { l.timestamp += 86_400; l.sequence_number += 17_280; });
        r.s.vault.accrue_fees();
        assert!(r.s.vault.get_nav_history().is_empty());
        for _ in 0..(constella_vault_strategy::NAV_HISTORY_MAX + 5) {
            next_day(&r, ONE * 108 / 100);
            r.s.vault.accrue_fees();
        }
        let h = r.s.vault.get_nav_history();
        assert_eq!(h.len(), constella_vault_strategy::NAV_HISTORY_MAX);
        assert_eq!(h.last().unwrap().timestamp, env.ledger().timestamp(), "oldest dropped, newest kept");
    }
}
