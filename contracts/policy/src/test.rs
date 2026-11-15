#![cfg(test)]
#![allow(clippy::bool_assert_comparison)]

extern crate std;

use super::*;
use constella_nav_oracle::{NavOracle, NavOracleClient, OracleSource};
use constella_registry::{AssetEntry, AssetRegistry, AssetRegistryClient, SleeveType};
use soroban_sdk::{
    testutils::{Address as _, Events, Ledger},
    vec, Address, BytesN, Env, Symbol, Vec,
};

#[allow(dead_code)]
struct TestContext {
    env: Env,
    admin: Address,
    owner: Address,
    vault: Address,
    proposer: Address,
    registry_address: Address,
    policy_address: Address,
    usdc_addr: Address,
    eurc_addr: Address,
    benji_addr: Address,
    eustbl_addr: Address,
    blend_addr: Address,
    usdf_addr: Address,
    deactivated_addr: Address,
    unregistered_addr: Address,
    oracle_address: Address,
}

fn register_registry_asset(
    env: &Env,
    registry_client: &AssetRegistryClient,
    contract_addr: &Address,
    asset_id: &str,
    issuer: &str,
    sleeve: SleeveType,
) {
    let entry = AssetEntry {
        contract_address: contract_addr.clone(),
        asset_id: Symbol::new(env, asset_id),
        issuer: Symbol::new(env, issuer),
        sleeve_type: sleeve,
        liquidity_tier: 0,
        permissionless: true,
        oracle_sources: vec![env, Symbol::new(env, "PYTH_ORACLE")],
        eligible_jurisdictions: vec![env],
        blocked_jurisdictions: vec![env],
        investor_classes_required: vec![env],
        is_active: true,
        added_at: env.ledger().timestamp(),
    };
    registry_client.add_asset(&entry);
}

fn setup_test_context() -> TestContext {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1731600000);

    let admin = Address::generate(&env);
    let owner = Address::generate(&env);
    let vault = Address::generate(&env);
    let proposer = Address::generate(&env);

    // Register closed universe asset registry
    let reg_id = env.register(AssetRegistry, (admin.clone(),));
    let reg_client = AssetRegistryClient::new(&env, &reg_id);

    // Real token contracts (SACs, 7 decimals): slippage valuation reads decimals.
    let token = |env: &Env| env.register_stellar_asset_contract_v2(Address::generate(env)).address();
    let usdc_addr = token(&env);
    let eurc_addr = token(&env);
    let benji_addr = token(&env);
    let eustbl_addr = token(&env);
    let blend_addr = token(&env);
    let usdf_addr = token(&env);
    let deactivated_addr = token(&env);
    let unregistered_addr = token(&env);

    register_registry_asset(&env, &reg_client, &usdc_addr, "CIRCLE_USDC", "CIRCLE", SleeveType::Cash);
    register_registry_asset(&env, &reg_client, &eurc_addr, "CIRCLE_EURC", "CIRCLE", SleeveType::Cash);
    register_registry_asset(&env, &reg_client, &benji_addr, "FRANKLIN_BENJI", "FRANKLIN_TEMPLETON", SleeveType::UsdGovFund);
    register_registry_asset(&env, &reg_client, &eustbl_addr, "SPIKO_EUSTBL", "SPIKO", SleeveType::EurSovereign);
    register_registry_asset(&env, &reg_client, &blend_addr, "BLEND_BLND", "BLEND", SleeveType::UsdGovFund);
    register_registry_asset(&env, &reg_client, &usdf_addr, "FRANKLIN_USDF", "FRANKLIN_TEMPLETON", SleeveType::UsdGovFund);
    register_registry_asset(&env, &reg_client, &deactivated_addr, "RISKY_TOKEN", "RISKY_ISSUER", SleeveType::YieldUsd);

    // Deactivate the deactivated asset
    reg_client.deactivate_asset(&Symbol::new(&env, "RISKY_TOKEN"));

    // NAV oracle with USDC as numéraire. FIXTURE: every other asset is priced
    // at exactly 1.0 by two independent feeds (each under its own key), so the
    // legacy tests keep their meaning. P1 tests use a realistic EUR/USD rate.
    let oracle_id = env.register(NavOracle, ());
    let oracle = NavOracleClient::new(&env, &oracle_id);
    oracle.initialize(&admin, &usdc_addr, &7, &300);
    for (addr, key) in [
        (&eurc_addr, "P_EURC"),
        (&benji_addr, "P_BENJI"),
        (&eustbl_addr, "P_EUSTBL"),
        (&blend_addr, "P_BLEND"),
        (&usdf_addr, "P_USDF"),
    ] {
        price_asset(&env, &oracle, addr, key, 10_000_000);
    }

    // Register policy contract
    let policy_id = env.register(PolicyContract, ());
    let policy_client = PolicyContractClient::new(&env, &policy_id);
    policy_client.initialize(&admin, &oracle_id);

    TestContext {
        env,
        admin,
        owner,
        vault,
        proposer,
        registry_address: reg_id,
        policy_address: policy_id,
        usdc_addr,
        eurc_addr,
        benji_addr,
        eustbl_addr,
        blend_addr,
        usdf_addr,
        deactivated_addr,
        unregistered_addr,
        oracle_address: oracle_id,
    }
}

/// Two independent, deep, fresh feeds for `asset` at `price` (7 decimals).
fn price_asset(env: &Env, oracle: &NavOracleClient, asset: &Address, key: &str, price: i128) {
    let sleeve = Symbol::new(env, key);
    oracle.configure_sleeve(&sleeve, &200, &2);
    for suffix in ["_A", "_B"] {
        let mut id = std::string::String::from(key);
        id.push_str(suffix);
        oracle.register_source(&OracleSource {
            source_id: Symbol::new(env, &id),
            asset: asset.clone(),
            sleeve: sleeve.clone(),
            price,
            timestamp: env.ledger().timestamp(),
            staleness_limit_seconds: 14_400,
            min_liquidity_depth_usd: 100_000,
            current_liquidity_depth_usd: 200_000,
            is_active: true,
        });
    }
}

/// The vault's currently registered policy hash - what an honest keeper builds
/// its proposal against - or all zeros if no policy is registered.
fn current_policy_hash(env: &Env, policy: &Address, vault: &Address) -> BytesN<32> {
    PolicyContractClient::new(env, policy)
        .get_policy_config(vault)
        .map(|c| c.policy_hash)
        .unwrap_or(BytesN::from_array(env, &[0u8; 32]))
}

fn sample_sleeves(env: &Env) -> Vec<SleeveTarget> {
    vec![
        env,
        SleeveTarget {
            asset_id: Symbol::new(env, "CIRCLE_USDC"),
            target_weight_bps: 2000,
            min_band_bps: 1000,
            max_band_bps: 2500,
            sleeve_type: Symbol::new(env, "cash"),
        },
        SleeveTarget {
            asset_id: Symbol::new(env, "CIRCLE_EURC"),
            target_weight_bps: 1000,
            min_band_bps: 500,
            max_band_bps: 1500,
            sleeve_type: Symbol::new(env, "cash"),
        },
        SleeveTarget {
            asset_id: Symbol::new(env, "FRANKLIN_BENJI"),
            target_weight_bps: 2500,
            min_band_bps: 1500,
            max_band_bps: 2500,
            sleeve_type: Symbol::new(env, "usd_gov_fund"),
        },
        SleeveTarget {
            asset_id: Symbol::new(env, "SPIKO_EUSTBL"),
            target_weight_bps: 2500,
            min_band_bps: 1500,
            max_band_bps: 2500,
            sleeve_type: Symbol::new(env, "eur_sovereign"),
        },
        SleeveTarget {
            asset_id: Symbol::new(env, "BLEND_BLND"),
            target_weight_bps: 2000,
            min_band_bps: 500,
            max_band_bps: 2500,
            sleeve_type: Symbol::new(env, "usd_gov_fund"),
        },
    ]
}

fn sample_hash(env: &Env, val: u8) -> BytesN<32> {
    BytesN::from_array(env, &[val; 32])
}

#[test]
fn test_t26_policy_registration_and_hash_storage() {
    let ctx = setup_test_context();
    let policy = PolicyContractClient::new(&ctx.env, &ctx.policy_address);

    // 1. Verify admin
    assert_eq!(policy.get_admin(), ctx.admin);

    // 3. Register policy
    let p_hash = sample_hash(&ctx.env, 0xAA);
    let sleeves = sample_sleeves(&ctx.env);

    policy.register_policy(
        &ctx.vault,
        &ctx.owner,
        &p_hash,
        &ctx.registry_address,
        &1000, // min_liquidity_sleeve_bps
        &2500, // max_single_asset_bps
        &3000, // max_issuer_bps
        &50,   // max_slippage_bps
        &500,  // rebalance_band_bps
        &sleeves,
    );

    // Verify registration event IMMEDIATELY after register_policy
    use soroban_sdk::TryFromVal;
    let events_after_reg = ctx.env.events().all();
    let reg_event_found = events_after_reg.iter().any(|e| {
        e.1.len() >= 2
            && Symbol::try_from_val(&ctx.env, &e.1.get(0).unwrap())
                == Ok(Symbol::new(&ctx.env, "POLICY_REGISTERED"))
    });
    assert!(reg_event_found, "POLICY_REGISTERED event should be emitted");

    // 4. Retrieve hash and config
    let stored_hash = policy.get_policy_hash(&ctx.vault);
    assert_eq!(stored_hash, p_hash);

    let config = policy.get_policy_config(&ctx.vault).expect("Config missing");
    assert_eq!(config.vault, ctx.vault);
    assert_eq!(config.policy_hash, p_hash);
    assert_eq!(config.version, 1);
    assert_eq!(config.registry_address, ctx.registry_address);
    assert_eq!(config.min_liquidity_sleeve_bps, 1000);
    assert_eq!(config.max_single_asset_bps, 2500);
    assert_eq!(config.max_issuer_bps, 3000);
    assert_eq!(config.max_slippage_bps, 50);
    assert_eq!(config.rebalance_band_bps, 500);
    assert_eq!(config.is_active, true);
    assert_eq!(config.owner, ctx.owner);
    assert_eq!(
        config.terms_hash,
        compute_terms_hash(&ctx.env, &ctx.registry_address, 1000, 2500, 3000, 50, 500, &sleeves)
    );

    let stored_sleeves = policy.get_sleeve_targets(&ctx.vault);
    assert_eq!(stored_sleeves.len(), 5);

    // 5. Update policy
    let new_hash = sample_hash(&ctx.env, 0xBB);
    policy.update_policy(&ctx.vault, &new_hash, &1000, &2500, &3000, &50, &500, &sleeves);
    // Audit P4: amendments apply only after the notice period.
    ctx.env.ledger().set_timestamp(ctx.env.ledger().timestamp() + PROTOCOL_MIN_NOTICE_SECONDS);
    PolicyContractClient::new(&ctx.env, &ctx.policy_address).activate_policy_update(&ctx.vault);

    // Verify POLICY_UPDATED event IMMEDIATELY after activation
    let events_after_upd = ctx.env.events().all();
    let upd_event_found = events_after_upd.iter().any(|e| {
        e.1.len() >= 2
            && Symbol::try_from_val(&ctx.env, &e.1.get(0).unwrap())
                == Ok(Symbol::new(&ctx.env, "POLICY_UPDATED"))
    });
    assert!(upd_event_found, "POLICY_UPDATED event should be emitted");

    let updated_hash = policy.get_policy_hash(&ctx.vault);
    assert_eq!(updated_hash, new_hash);

    let updated_config = policy.get_policy_config(&ctx.vault).unwrap();
    assert_eq!(updated_config.version, 2);
    assert_eq!(updated_config.policy_hash, new_hash);

    // 6. Querying unregistered vault fails with PolicyNotFound
    let unregistered_vault = Address::generate(&ctx.env);
    let missing_hash = policy.try_get_policy_hash(&unregistered_vault);
    assert_eq!(
        missing_hash,
        Err(Ok(soroban_sdk::Error::from_contract_error(
            PolicyError::PolicyNotFound as u32
        )))
    );
}

#[test]
fn test_t26_uninitialized_and_unauthorized_checks() {
    let env = Env::default();
    let policy_id = env.register(PolicyContract, ());
    let policy = PolicyContractClient::new(&env, &policy_id);

    // Calling get_admin on uninitialized contract fails
    let admin_res = policy.try_get_admin();
    assert_eq!(
        admin_res,
        Err(Ok(soroban_sdk::Error::from_contract_error(
            PolicyError::NotInitialized as u32
        )))
    );

    // Calling register_policy on uninitialized contract fails
    let vault = Address::generate(&env);
    let owner = Address::generate(&env);
    let reg_addr = Address::generate(&env);
    let p_hash = sample_hash(&env, 0x01);
    let sleeves = sample_sleeves(&env);

    let reg_res = policy.try_register_policy(
        &vault,
        &owner,
        &p_hash,
        &reg_addr,
        &1000,
        &2500,
        &3000,
        &50,
        &500,
        &sleeves,
    );
    assert_eq!(
        reg_res,
        Err(Ok(soroban_sdk::Error::from_contract_error(
            PolicyError::NotInitialized as u32
        )))
    );

    // Initialize with admin
    let admin = Address::generate(&env);
    let oracle_addr = Address::generate(&env); // never called in this test
    policy.initialize(&admin, &oracle_addr);

    // Double initialization must fail
    let double_init = policy.try_initialize(&admin, &oracle_addr);
    assert_eq!(
        double_init,
        Err(Ok(soroban_sdk::Error::from_contract_error(
            PolicyError::AlreadyInitialized as u32
        )))
    );

    // Registration without owner + vault authorization fails
    env.mock_auths(&[]);
    let unauth_reg = policy.try_register_policy(
        &vault,
        &owner,
        &p_hash,
        &reg_addr,
        &1000,
        &2500,
        &3000,
        &50,
        &500,
        &sleeves,
    );
    assert!(unauth_reg.is_err(), "Unauthorized registration should fail");

    let new_hash = sample_hash(&env, 0x02);
    let unauth_upd = policy.try_update_policy(&vault, &new_hash, &1000, &2500, &3000, &50, &500, &sleeves);
    assert!(unauth_upd.is_err(), "Unauthorized update should fail");
}

#[test]
fn test_t27_compliant_rebalance_acceptance() {
    let ctx = setup_test_context();
    let policy = PolicyContractClient::new(&ctx.env, &ctx.policy_address);

    let p_hash = sample_hash(&ctx.env, 0x11);
    let sleeves = sample_sleeves(&ctx.env);

    policy.register_policy(
        &ctx.vault,
        &ctx.owner,
        &p_hash,
        &ctx.registry_address,
        &1000,
        &2500,
        &3000,
        &50,
        &500,
        &sleeves,
    );

    // Construct compliant proposal:
    // CIRCLE_USDC: 2000
    // CIRCLE_EURC: 1000
    // FRANKLIN_BENJI: 2500
    // SPIKO_EUSTBL: 2500
    // BLEND_BLND: 2000
    // Sum = 10,000 bps
    // Cash = 3000 bps >= 1000 bps
    // Single asset max = 2500 bps <= 2500 bps
    // Issuer max = 3000 bps (CIRCLE=3000, FRANKLIN=2500, SPIKO=2500, BLEND=2000)
    let weights_after: Vec<(Symbol, u32)> = vec![
        &ctx.env,
        (Symbol::new(&ctx.env, "CIRCLE_USDC"), 2000),
        (Symbol::new(&ctx.env, "CIRCLE_EURC"), 1000),
        (Symbol::new(&ctx.env, "FRANKLIN_BENJI"), 2500),
        (Symbol::new(&ctx.env, "SPIKO_EUSTBL"), 2500),
        (Symbol::new(&ctx.env, "BLEND_BLND"), 2000),
    ];

    // Swap 10,000 USDC -> 9,970 EURC (30 bps slippage <= 50 bps cap)
    let ops = vec![
        &ctx.env,
        TradeOperation {
            asset_in: ctx.usdc_addr.clone(),
            asset_out: ctx.eurc_addr.clone(),
            amount_in: 10_000,
            min_amount_out: 9_970,
            route: TradeRoute::Soroswap,
        },
    ];

    let proposal = RebalanceProposal {
        vault: ctx.vault.clone(),
        policy_hash: current_policy_hash(&ctx.env, &ctx.policy_address, &ctx.vault),
        operations: ops,
        weights_after,
        deadline: ctx.env.ledger().timestamp() + 3600,
        proposer: ctx.proposer.clone(),
    };

    let result = policy.validate_rebalance(&ctx.vault, &proposal);
    assert_eq!(result.is_valid, true);
    assert_eq!(result.code, 0);
    assert_eq!(result.message, Symbol::new(&ctx.env, "VALID"));

    // Verify REBALANCE_VALIDATED event emitted
    use soroban_sdk::TryFromVal;
    let events = ctx.env.events().all();
    let val_event_found = events.iter().any(|e| {
        e.1.len() >= 2
            && Symbol::try_from_val(&ctx.env, &e.1.get(0).unwrap())
                == Ok(Symbol::new(&ctx.env, "REBALANCE_VALIDATED"))
    });
    assert!(val_event_found, "REBALANCE_VALIDATED event must be published");
}

#[test]
fn test_t28_weight_band_breach_rejected() {
    let ctx = setup_test_context();
    let policy = PolicyContractClient::new(&ctx.env, &ctx.policy_address);

    let p_hash = sample_hash(&ctx.env, 0x28);
    let sleeves = sample_sleeves(&ctx.env);

    policy.register_policy(
        &ctx.vault,
        &ctx.owner,
        &p_hash,
        &ctx.registry_address,
        &1000,
        &2500,
        &3000,
        &50,
        &500,
        &sleeves,
    );

    // Case 1: Weights do not sum to 10000 bps (sums to 9500)
    let weights_sum_fail: Vec<(Symbol, u32)> = vec![
        &ctx.env,
        (Symbol::new(&ctx.env, "CIRCLE_USDC"), 1500),
        (Symbol::new(&ctx.env, "CIRCLE_EURC"), 1000),
        (Symbol::new(&ctx.env, "FRANKLIN_BENJI"), 2500),
        (Symbol::new(&ctx.env, "SPIKO_EUSTBL"), 2500),
        (Symbol::new(&ctx.env, "BLEND_BLND"), 2000),
    ];
    let proposal1 = RebalanceProposal {
        vault: ctx.vault.clone(),
        policy_hash: current_policy_hash(&ctx.env, &ctx.policy_address, &ctx.vault),
        operations: vec![&ctx.env],
        weights_after: weights_sum_fail,
        deadline: ctx.env.ledger().timestamp() + 3600,
        proposer: ctx.proposer.clone(),
    };
    let res1 = policy.validate_rebalance(&ctx.vault, &proposal1);
    assert_eq!(res1.is_valid, false);
    assert_eq!(res1.code, PolicyError::WeightsDoNotSumTo10000 as u32);

    // Case 2: Weight exceeds declared sleeve max band
    // CIRCLE_EURC band is [500, 1500]. Propose 1600.
    let weights_band_max_fail: Vec<(Symbol, u32)> = vec![
        &ctx.env,
        (Symbol::new(&ctx.env, "CIRCLE_USDC"), 1400),
        (Symbol::new(&ctx.env, "CIRCLE_EURC"), 1600), // Exceeds 1500 max band
        (Symbol::new(&ctx.env, "FRANKLIN_BENJI"), 2500),
        (Symbol::new(&ctx.env, "SPIKO_EUSTBL"), 2500),
        (Symbol::new(&ctx.env, "BLEND_BLND"), 2000),
    ];
    let proposal2 = RebalanceProposal {
        vault: ctx.vault.clone(),
        policy_hash: current_policy_hash(&ctx.env, &ctx.policy_address, &ctx.vault),
        operations: vec![&ctx.env],
        weights_after: weights_band_max_fail,
        deadline: ctx.env.ledger().timestamp() + 3600,
        proposer: ctx.proposer.clone(),
    };
    let res2 = policy.validate_rebalance(&ctx.vault, &proposal2);
    assert_eq!(res2.is_valid, false);
    assert_eq!(res2.code, PolicyError::WeightBandExceeded as u32);

    // Case 3: Weight below declared sleeve min band
    // FRANKLIN_BENJI band is [1500, 2500]. Propose 1400.
    let weights_band_min_fail: Vec<(Symbol, u32)> = vec![
        &ctx.env,
        (Symbol::new(&ctx.env, "CIRCLE_USDC"), 2500),
        (Symbol::new(&ctx.env, "CIRCLE_EURC"), 1000),
        (Symbol::new(&ctx.env, "FRANKLIN_BENJI"), 1400), // Below 1500 min band
        (Symbol::new(&ctx.env, "SPIKO_EUSTBL"), 2500),
        (Symbol::new(&ctx.env, "BLEND_BLND"), 2600),
    ];
    let proposal3 = RebalanceProposal {
        vault: ctx.vault.clone(),
        policy_hash: current_policy_hash(&ctx.env, &ctx.policy_address, &ctx.vault),
        operations: vec![&ctx.env],
        weights_after: weights_band_min_fail,
        deadline: ctx.env.ledger().timestamp() + 3600,
        proposer: ctx.proposer.clone(),
    };
    let res3 = policy.validate_rebalance(&ctx.vault, &proposal3);
    assert_eq!(res3.is_valid, false);
    assert_eq!(res3.code, PolicyError::WeightBandExceeded as u32);

    // Case 4: Undeclared asset in weights_after
    let weights_undeclared: Vec<(Symbol, u32)> = vec![
        &ctx.env,
        (Symbol::new(&ctx.env, "CIRCLE_USDC"), 2000),
        (Symbol::new(&ctx.env, "UNKNOWN_ASSET"), 1000),
        (Symbol::new(&ctx.env, "FRANKLIN_BENJI"), 2500),
        (Symbol::new(&ctx.env, "SPIKO_EUSTBL"), 2500),
        (Symbol::new(&ctx.env, "BLEND_BLND"), 2000),
    ];
    let proposal4 = RebalanceProposal {
        vault: ctx.vault.clone(),
        policy_hash: current_policy_hash(&ctx.env, &ctx.policy_address, &ctx.vault),
        operations: vec![&ctx.env],
        weights_after: weights_undeclared,
        deadline: ctx.env.ledger().timestamp() + 3600,
        proposer: ctx.proposer.clone(),
    };
    let res4 = policy.validate_rebalance(&ctx.vault, &proposal4);
    assert_eq!(res4.is_valid, false);
    assert_eq!(res4.code, PolicyError::WeightBandExceeded as u32);
}

#[test]
fn test_t29_liquidity_floor_breach_rejected() {
    let ctx = setup_test_context();
    let policy = PolicyContractClient::new(&ctx.env, &ctx.policy_address);

    let p_hash = sample_hash(&ctx.env, 0x29);

    // Configure sleeves where cash min bands allow low values, but policy enforces min_liquidity_sleeve_bps = 1000
    let sleeves = vec![
        &ctx.env,
        SleeveTarget {
            asset_id: Symbol::new(&ctx.env, "CIRCLE_USDC"),
            target_weight_bps: 1000,
            min_band_bps: 400,
            max_band_bps: 2000,
            sleeve_type: Symbol::new(&ctx.env, "cash"),
        },
        SleeveTarget {
            asset_id: Symbol::new(&ctx.env, "CIRCLE_EURC"),
            target_weight_bps: 500,
            min_band_bps: 200,
            max_band_bps: 1000,
            sleeve_type: Symbol::new(&ctx.env, "cash"),
        },
        SleeveTarget {
            asset_id: Symbol::new(&ctx.env, "FRANKLIN_BENJI"),
            target_weight_bps: 2500,
            min_band_bps: 1000,
            max_band_bps: 3000,
            sleeve_type: Symbol::new(&ctx.env, "usd_gov_fund"),
        },
        SleeveTarget {
            asset_id: Symbol::new(&ctx.env, "SPIKO_EUSTBL"),
            target_weight_bps: 3000,
            min_band_bps: 1000,
            max_band_bps: 3500,
            sleeve_type: Symbol::new(&ctx.env, "eur_sovereign"),
        },
        SleeveTarget {
            asset_id: Symbol::new(&ctx.env, "BLEND_BLND"),
            target_weight_bps: 3000,
            min_band_bps: 1000,
            max_band_bps: 3500,
            sleeve_type: Symbol::new(&ctx.env, "usd_gov_fund"),
        },
    ];

    policy.register_policy(
        &ctx.vault,
        &ctx.owner,
        &p_hash,
        &ctx.registry_address,
        &1000, // min cash liquidity sleeve = 1000 bps
        &3500, // max_single_asset_bps
        &4000, // max_issuer_bps
        &50,
        &500,
        &sleeves,
    );

    // Proposal sets cash sleeves: CIRCLE_USDC (500) + CIRCLE_EURC (300) = 800 bps (< 1000 bps floor)
    let weights_after: Vec<(Symbol, u32)> = vec![
        &ctx.env,
        (Symbol::new(&ctx.env, "CIRCLE_USDC"), 500),
        (Symbol::new(&ctx.env, "CIRCLE_EURC"), 300),
        (Symbol::new(&ctx.env, "FRANKLIN_BENJI"), 3000),
        (Symbol::new(&ctx.env, "SPIKO_EUSTBL"), 3100),
        (Symbol::new(&ctx.env, "BLEND_BLND"), 3100),
    ];

    let proposal = RebalanceProposal {
        vault: ctx.vault.clone(),
        policy_hash: current_policy_hash(&ctx.env, &ctx.policy_address, &ctx.vault),
        operations: vec![&ctx.env],
        weights_after,
        deadline: ctx.env.ledger().timestamp() + 3600,
        proposer: ctx.proposer.clone(),
    };

    let result = policy.validate_rebalance(&ctx.vault, &proposal);
    assert_eq!(result.is_valid, false);
    assert_eq!(result.code, PolicyError::LiquidityFloorBreached as u32);
    assert_eq!(result.message, Symbol::new(&ctx.env, "LIQUIDITY_FLOOR_BREACHED"));
}

#[test]
fn test_t30_single_asset_cap_and_issuer_cap_rejected() {
    let ctx = setup_test_context();
    let policy = PolicyContractClient::new(&ctx.env, &ctx.policy_address);

    let p_hash = sample_hash(&ctx.env, 0x30);

    // Declare sleeves with 2 assets under same issuer FRANKLIN: FRANKLIN_BENJI & FRANKLIN_USDF
    let sleeves = vec![
        &ctx.env,
        SleeveTarget {
            asset_id: Symbol::new(&ctx.env, "CIRCLE_USDC"),
            target_weight_bps: 1000,
            min_band_bps: 500,
            max_band_bps: 2000,
            sleeve_type: Symbol::new(&ctx.env, "cash"),
        },
        SleeveTarget {
            asset_id: Symbol::new(&ctx.env, "CIRCLE_EURC"),
            target_weight_bps: 1000,
            min_band_bps: 500,
            max_band_bps: 2000,
            sleeve_type: Symbol::new(&ctx.env, "cash"),
        },
        SleeveTarget {
            asset_id: Symbol::new(&ctx.env, "FRANKLIN_BENJI"),
            target_weight_bps: 2500,
            min_band_bps: 500,
            max_band_bps: 3000,
            sleeve_type: Symbol::new(&ctx.env, "usd_gov_fund"),
        },
        SleeveTarget {
            asset_id: Symbol::new(&ctx.env, "FRANKLIN_USDF"),
            target_weight_bps: 2500,
            min_band_bps: 500,
            max_band_bps: 3000,
            sleeve_type: Symbol::new(&ctx.env, "usd_gov_fund"),
        },
        SleeveTarget {
            asset_id: Symbol::new(&ctx.env, "SPIKO_EUSTBL"),
            target_weight_bps: 3000,
            min_band_bps: 1000,
            max_band_bps: 4000,
            sleeve_type: Symbol::new(&ctx.env, "eur_sovereign"),
        },
    ];

    policy.register_policy(
        &ctx.vault,
        &ctx.owner,
        &p_hash,
        &ctx.registry_address,
        &1000, // min_liquidity
        &2500, // max_single_asset_bps = 2500
        &3000, // max_issuer_bps = 3000
        &50,
        &500,
        &sleeves,
    );

    // Case 1: Single asset cap exceeded (> 2500 bps)
    // FRANKLIN_BENJI = 2600 bps (> 2500 single asset cap)
    let weights_single_asset_breach: Vec<(Symbol, u32)> = vec![
        &ctx.env,
        (Symbol::new(&ctx.env, "CIRCLE_USDC"), 1000),
        (Symbol::new(&ctx.env, "CIRCLE_EURC"), 1000),
        (Symbol::new(&ctx.env, "FRANKLIN_BENJI"), 2600), // Exceeds max_single_asset_bps 2500
        (Symbol::new(&ctx.env, "FRANKLIN_USDF"), 2400),
        (Symbol::new(&ctx.env, "SPIKO_EUSTBL"), 3000),
    ];
    let proposal1 = RebalanceProposal {
        vault: ctx.vault.clone(),
        policy_hash: current_policy_hash(&ctx.env, &ctx.policy_address, &ctx.vault),
        operations: vec![&ctx.env],
        weights_after: weights_single_asset_breach,
        deadline: ctx.env.ledger().timestamp() + 3600,
        proposer: ctx.proposer.clone(),
    };
    let res1 = policy.validate_rebalance(&ctx.vault, &proposal1);
    assert_eq!(res1.is_valid, false);
    assert_eq!(res1.code, PolicyError::SingleAssetCapExceeded as u32);

    // Case 2: Issuer cap exceeded with 2 assets from same issuer
    // FRANKLIN_BENJI = 2500 (<= 2500 single asset cap)
    // FRANKLIN_USDF = 2500 (<= 2500 single asset cap)
    // Total FRANKLIN issuer = 2500 + 2500 = 5000 bps (> 3000 max_issuer_bps!)
    let weights_franklin_issuer_breach: Vec<(Symbol, u32)> = vec![
        &ctx.env,
        (Symbol::new(&ctx.env, "CIRCLE_USDC"), 1500), // CIRCLE: 1500
        (Symbol::new(&ctx.env, "CIRCLE_EURC"), 1000), // CIRCLE: 1000 -> Total CIRCLE = 2500 <= 3000
        (Symbol::new(&ctx.env, "FRANKLIN_BENJI"), 2500), // single asset cap <= 2500
        (Symbol::new(&ctx.env, "FRANKLIN_USDF"), 2500),  // single asset cap <= 2500 -> Total FRANKLIN = 5000 > 3000!
        (Symbol::new(&ctx.env, "SPIKO_EUSTBL"), 2500),   // SPIKO: 2500 <= 3000
    ];
    // Sum = 1500 + 1000 + 2500 + 2500 + 2500 = 10000.
    // Single asset cap: all are <= 2500. Passes!
    // Cash: 1500 + 1000 = 2500 >= 1000. Passes!
    // Issuer FRANKLIN: 2500 + 2500 = 5000 > 3000. Fails with IssuerCapExceeded!
    let proposal2 = RebalanceProposal {
        vault: ctx.vault.clone(),
        policy_hash: current_policy_hash(&ctx.env, &ctx.policy_address, &ctx.vault),
        operations: vec![&ctx.env],
        weights_after: weights_franklin_issuer_breach,
        deadline: ctx.env.ledger().timestamp() + 3600,
        proposer: ctx.proposer.clone(),
    };
    let res2 = policy.validate_rebalance(&ctx.vault, &proposal2);
    assert_eq!(res2.is_valid, false);
    assert_eq!(res2.code, PolicyError::IssuerCapExceeded as u32);
    assert_eq!(res2.message, Symbol::new(&ctx.env, "ISSUER_CAP_EXCEEDED"));
}

#[test]
fn test_t31_slippage_and_deadline_rejections() {
    let ctx = setup_test_context();
    let policy = PolicyContractClient::new(&ctx.env, &ctx.policy_address);

    let p_hash = sample_hash(&ctx.env, 0x31);
    let sleeves = sample_sleeves(&ctx.env);

    policy.register_policy(
        &ctx.vault,
        &ctx.owner,
        &p_hash,
        &ctx.registry_address,
        &1000,
        &2500,
        &3000,
        &50, // max slippage = 50 bps
        &500,
        &sleeves,
    );

    let valid_weights: Vec<(Symbol, u32)> = vec![
        &ctx.env,
        (Symbol::new(&ctx.env, "CIRCLE_USDC"), 2000),
        (Symbol::new(&ctx.env, "CIRCLE_EURC"), 1000),
        (Symbol::new(&ctx.env, "FRANKLIN_BENJI"), 2500),
        (Symbol::new(&ctx.env, "SPIKO_EUSTBL"), 2500),
        (Symbol::new(&ctx.env, "BLEND_BLND"), 2000),
    ];

    // Case 1: Deadline expired (timestamp in past)
    let past_deadline = ctx.env.ledger().timestamp() - 1;
    let proposal1 = RebalanceProposal {
        vault: ctx.vault.clone(),
        policy_hash: current_policy_hash(&ctx.env, &ctx.policy_address, &ctx.vault),
        operations: vec![&ctx.env],
        weights_after: valid_weights.clone(),
        deadline: past_deadline,
        proposer: ctx.proposer.clone(),
    };
    let res1 = policy.validate_rebalance(&ctx.vault, &proposal1);
    assert_eq!(res1.is_valid, false);
    assert_eq!(res1.code, PolicyError::DeadlineExceeded as u32);
    assert_eq!(res1.message, Symbol::new(&ctx.env, "DEADLINE_EXCEEDED"));

    // Case 2: Excessive slippage
    // amount_in = 10,000, max_slippage_bps = 50. Minimum allowed out is 9,950.
    // Proposed min_amount_out = 9,900 (100 bps slippage)
    let ops_excessive_slippage = vec![
        &ctx.env,
        TradeOperation {
            asset_in: ctx.usdc_addr.clone(),
            asset_out: ctx.eurc_addr.clone(),
            amount_in: 10_000,
            min_amount_out: 9_900,
            route: TradeRoute::Aquarius,
        },
    ];
    let proposal2 = RebalanceProposal {
        vault: ctx.vault.clone(),
        policy_hash: current_policy_hash(&ctx.env, &ctx.policy_address, &ctx.vault),
        operations: ops_excessive_slippage,
        weights_after: valid_weights.clone(),
        deadline: ctx.env.ledger().timestamp() + 3600,
        proposer: ctx.proposer.clone(),
    };
    let res2 = policy.validate_rebalance(&ctx.vault, &proposal2);
    assert_eq!(res2.is_valid, false);
    assert_eq!(res2.code, PolicyError::SlippageLimitExceeded as u32);
    assert_eq!(res2.message, Symbol::new(&ctx.env, "SLIPPAGE_EXCEEDED"));

    // Case 3: Invalid trade amount_in <= 0
    let ops_invalid_amount = vec![
        &ctx.env,
        TradeOperation {
            asset_in: ctx.usdc_addr.clone(),
            asset_out: ctx.eurc_addr.clone(),
            amount_in: 0,
            min_amount_out: 0,
            route: TradeRoute::Sdex,
        },
    ];
    let proposal3 = RebalanceProposal {
        vault: ctx.vault.clone(),
        policy_hash: current_policy_hash(&ctx.env, &ctx.policy_address, &ctx.vault),
        operations: ops_invalid_amount,
        weights_after: valid_weights.clone(),
        deadline: ctx.env.ledger().timestamp() + 3600,
        proposer: ctx.proposer.clone(),
    };
    let res3 = policy.validate_rebalance(&ctx.vault, &proposal3);
    assert_eq!(res3.is_valid, false);
    assert_eq!(res3.code, PolicyError::SlippageLimitExceeded as u32);
}

#[test]
fn test_t32_closed_universe_registry_cross_check() {
    let ctx = setup_test_context();
    let policy = PolicyContractClient::new(&ctx.env, &ctx.policy_address);

    let p_hash = sample_hash(&ctx.env, 0x32);
    let sleeves = sample_sleeves(&ctx.env);

    policy.register_policy(
        &ctx.vault,
        &ctx.owner,
        &p_hash,
        &ctx.registry_address,
        &1000,
        &2500,
        &3000,
        &50,
        &500,
        &sleeves,
    );

    let valid_weights: Vec<(Symbol, u32)> = vec![
        &ctx.env,
        (Symbol::new(&ctx.env, "CIRCLE_USDC"), 2000),
        (Symbol::new(&ctx.env, "CIRCLE_EURC"), 1000),
        (Symbol::new(&ctx.env, "FRANKLIN_BENJI"), 2500),
        (Symbol::new(&ctx.env, "SPIKO_EUSTBL"), 2500),
        (Symbol::new(&ctx.env, "BLEND_BLND"), 2000),
    ];

    // Case 1: Unregistered asset in trade operation (asset_in)
    let ops_unregistered_in = vec![
        &ctx.env,
        TradeOperation {
            asset_in: ctx.unregistered_addr.clone(),
            asset_out: ctx.usdc_addr.clone(),
            amount_in: 10_000,
            min_amount_out: 9_960,
            route: TradeRoute::Soroswap,
        },
    ];
    let proposal1 = RebalanceProposal {
        vault: ctx.vault.clone(),
        policy_hash: current_policy_hash(&ctx.env, &ctx.policy_address, &ctx.vault),
        operations: ops_unregistered_in,
        weights_after: valid_weights.clone(),
        deadline: ctx.env.ledger().timestamp() + 3600,
        proposer: ctx.proposer.clone(),
    };
    let res1 = policy.validate_rebalance(&ctx.vault, &proposal1);
    assert_eq!(res1.is_valid, false);
    assert_eq!(res1.code, PolicyError::AssetNotRegistered as u32);
    assert_eq!(res1.message, Symbol::new(&ctx.env, "ASSET_NOT_REGISTERED"));

    // Case 2: Unregistered asset in trade operation (asset_out)
    let ops_unregistered_out = vec![
        &ctx.env,
        TradeOperation {
            asset_in: ctx.usdc_addr.clone(),
            asset_out: ctx.unregistered_addr.clone(),
            amount_in: 10_000,
            min_amount_out: 9_960,
            route: TradeRoute::Soroswap,
        },
    ];
    let proposal2 = RebalanceProposal {
        vault: ctx.vault.clone(),
        policy_hash: current_policy_hash(&ctx.env, &ctx.policy_address, &ctx.vault),
        operations: ops_unregistered_out,
        weights_after: valid_weights.clone(),
        deadline: ctx.env.ledger().timestamp() + 3600,
        proposer: ctx.proposer.clone(),
    };
    let res2 = policy.validate_rebalance(&ctx.vault, &proposal2);
    assert_eq!(res2.is_valid, false);
    assert_eq!(res2.code, PolicyError::AssetNotRegistered as u32);

    // Case 3: Deactivated asset in trade operation
    let ops_deactivated = vec![
        &ctx.env,
        TradeOperation {
            asset_in: ctx.deactivated_addr.clone(),
            asset_out: ctx.usdc_addr.clone(),
            amount_in: 10_000,
            min_amount_out: 9_960,
            route: TradeRoute::Issuer,
        },
    ];
    let proposal3 = RebalanceProposal {
        vault: ctx.vault.clone(),
        policy_hash: current_policy_hash(&ctx.env, &ctx.policy_address, &ctx.vault),
        operations: ops_deactivated,
        weights_after: valid_weights.clone(),
        deadline: ctx.env.ledger().timestamp() + 3600,
        proposer: ctx.proposer.clone(),
    };
    let res3 = policy.validate_rebalance(&ctx.vault, &proposal3);
    assert_eq!(res3.is_valid, false);
    assert_eq!(res3.code, PolicyError::AssetNotRegistered as u32);
}

#[test]
fn test_exact_boundaries_acceptance() {
    let ctx = setup_test_context();
    let policy = PolicyContractClient::new(&ctx.env, &ctx.policy_address);

    let p_hash = sample_hash(&ctx.env, 0x40);
    let sleeves = sample_sleeves(&ctx.env);

    policy.register_policy(
        &ctx.vault,
        &ctx.owner,
        &p_hash,
        &ctx.registry_address,
        &1000, // min liquidity = 1000 bps
        &2500, // max single asset = 2500 bps
        &3000, // max issuer = 3000 bps
        &50,   // max slippage = 50 bps
        &500,
        &sleeves,
    );

    // Exact boundary proposal:
    // Cash is exactly 1000 bps (CIRCLE_USDC = 1000, CIRCLE_EURC = 0 -> wait, EURC min band is 500.
    // Let's set CIRCLE_USDC = 1000, min band is 1000; CIRCLE_EURC = 500, min band is 500 -> total cash = 1500 > 1000.
    // What if sleeves are configured so cash can be exactly 1000?
    // Let's test standard compliant weights at exact boundaries:
    // FRANKLIN_BENJI = 2500 (exactly equal to max_single_asset_bps!)
    // CIRCLE issuer total = 2000 + 1000 = 3000 (exactly equal to max_issuer_bps!)
    let weights: Vec<(Symbol, u32)> = vec![
        &ctx.env,
        (Symbol::new(&ctx.env, "CIRCLE_USDC"), 2000),
        (Symbol::new(&ctx.env, "CIRCLE_EURC"), 1000),
        (Symbol::new(&ctx.env, "FRANKLIN_BENJI"), 2500),
        (Symbol::new(&ctx.env, "SPIKO_EUSTBL"), 2500),
        (Symbol::new(&ctx.env, "BLEND_BLND"), 2000),
    ];

    // Exact boundary slippage:
    // amount_in = 10_000, max_slippage_bps = 50.
    // min_amount_out = 9,950 (exact 50 bps slippage: 10000 * 9950 / 10000 = 9950)
    let ops = vec![
        &ctx.env,
        TradeOperation {
            asset_in: ctx.usdc_addr.clone(),
            asset_out: ctx.eurc_addr.clone(),
            amount_in: 10_000,
            min_amount_out: 9_950,
            route: TradeRoute::Soroswap,
        },
    ];

    // Exact boundary deadline: proposal.deadline == current ledger timestamp
    let proposal = RebalanceProposal {
        vault: ctx.vault.clone(),
        policy_hash: current_policy_hash(&ctx.env, &ctx.policy_address, &ctx.vault),
        operations: ops,
        weights_after: weights,
        deadline: ctx.env.ledger().timestamp(), // exactly current timestamp!
        proposer: ctx.proposer.clone(),
    };

    let res = policy.validate_rebalance(&ctx.vault, &proposal);
    assert_eq!(res.is_valid, true);
    assert_eq!(res.code, 0);
    assert_eq!(res.message, Symbol::new(&ctx.env, "VALID"));
}

#[test]
fn test_rebalance_without_operations_passes() {
    let ctx = setup_test_context();
    let policy = PolicyContractClient::new(&ctx.env, &ctx.policy_address);

    let p_hash = sample_hash(&ctx.env, 0x50);
    let sleeves = sample_sleeves(&ctx.env);

    policy.register_policy(
        &ctx.vault,
        &ctx.owner,
        &p_hash,
        &ctx.registry_address,
        &1000,
        &2500,
        &3000,
        &50,
        &500,
        &sleeves,
    );

    let weights: Vec<(Symbol, u32)> = vec![
        &ctx.env,
        (Symbol::new(&ctx.env, "CIRCLE_USDC"), 2000),
        (Symbol::new(&ctx.env, "CIRCLE_EURC"), 1000),
        (Symbol::new(&ctx.env, "FRANKLIN_BENJI"), 2500),
        (Symbol::new(&ctx.env, "SPIKO_EUSTBL"), 2500),
        (Symbol::new(&ctx.env, "BLEND_BLND"), 2000),
    ];

    // Proposal with empty operations
    let proposal = RebalanceProposal {
        vault: ctx.vault.clone(),
        policy_hash: current_policy_hash(&ctx.env, &ctx.policy_address, &ctx.vault),
        operations: vec![&ctx.env],
        weights_after: weights,
        deadline: ctx.env.ledger().timestamp() + 3600,
        proposer: ctx.proposer.clone(),
    };

    let res = policy.validate_rebalance(&ctx.vault, &proposal);
    assert_eq!(res.is_valid, true);
    assert_eq!(res.code, 0);
}

#[test]
fn test_unregistered_vault_rebalance_rejected() {
    let ctx = setup_test_context();
    let policy = PolicyContractClient::new(&ctx.env, &ctx.policy_address);

    let random_vault = Address::generate(&ctx.env);
    let proposal = RebalanceProposal {
        vault: random_vault.clone(),
        policy_hash: current_policy_hash(&ctx.env, &ctx.policy_address, &ctx.vault),
        operations: vec![&ctx.env],
        weights_after: vec![&ctx.env],
        deadline: ctx.env.ledger().timestamp() + 3600,
        proposer: ctx.proposer.clone(),
    };

    let res = policy.validate_rebalance(&random_vault, &proposal);
    assert_eq!(res.is_valid, false);
    assert_eq!(res.code, PolicyError::PolicyNotFound as u32);
    assert_eq!(res.message, Symbol::new(&ctx.env, "POLICY_NOT_FOUND"));
}

#[test]
fn test_duplicate_asset_in_weights_after_rejected() {
    let ctx = setup_test_context();
    let policy = PolicyContractClient::new(&ctx.env, &ctx.policy_address);

    let p_hash = sample_hash(&ctx.env, 0x60);
    let sleeves = sample_sleeves(&ctx.env);

    policy.register_policy(
        &ctx.vault,
        &ctx.owner,
        &p_hash,
        &ctx.registry_address,
        &1000,
        &2500, // max_single_asset_bps = 2500
        &5000, // max_issuer_bps = 5000
        &50,
        &500,
        &sleeves,
    );

    // Exploit attempt: Split CIRCLE_USDC across 2 entries (2000 + 2000 = 4000)
    // to bypass single asset cap (2500) and sleeve max band (2500).
    let duplicate_weights: Vec<(Symbol, u32)> = vec![
        &ctx.env,
        (Symbol::new(&ctx.env, "CIRCLE_USDC"), 2000),
        (Symbol::new(&ctx.env, "CIRCLE_USDC"), 2000),
        (Symbol::new(&ctx.env, "FRANKLIN_BENJI"), 2000),
        (Symbol::new(&ctx.env, "SPIKO_EUSTBL"), 2000),
        (Symbol::new(&ctx.env, "BLEND_BLND"), 2000),
    ];

    let proposal = RebalanceProposal {
        vault: ctx.vault.clone(),
        policy_hash: current_policy_hash(&ctx.env, &ctx.policy_address, &ctx.vault),
        operations: vec![&ctx.env],
        weights_after: duplicate_weights,
        deadline: ctx.env.ledger().timestamp() + 3600,
        proposer: ctx.proposer.clone(),
    };

    let res = policy.validate_rebalance(&ctx.vault, &proposal);
    assert_eq!(res.is_valid, false);
    assert_eq!(res.code, PolicyError::WeightBandExceeded as u32);
    assert_eq!(res.message, Symbol::new(&ctx.env, "DUPLICATE_ASSET"));
}

#[test]
fn test_slippage_overflow_rejected() {
    let ctx = setup_test_context();
    let policy = PolicyContractClient::new(&ctx.env, &ctx.policy_address);

    let p_hash = sample_hash(&ctx.env, 0x61);
    let sleeves = sample_sleeves(&ctx.env);

    policy.register_policy(
        &ctx.vault,
        &ctx.owner,
        &p_hash,
        &ctx.registry_address,
        &1000,
        &2500,
        &3000,
        &50,
        &500,
        &sleeves,
    );

    let valid_weights: Vec<(Symbol, u32)> = vec![
        &ctx.env,
        (Symbol::new(&ctx.env, "CIRCLE_USDC"), 2000),
        (Symbol::new(&ctx.env, "CIRCLE_EURC"), 1000),
        (Symbol::new(&ctx.env, "FRANKLIN_BENJI"), 2500),
        (Symbol::new(&ctx.env, "SPIKO_EUSTBL"), 2500),
        (Symbol::new(&ctx.env, "BLEND_BLND"), 2000),
    ];

    // Exploit attempt: amount_in = i128::MAX with min_amount_out = 1
    // Previously caused integer overflow in amount_in.checked_mul(9950) -> unwrap_or(0) -> min_required_out = 0 -> passed!
    let ops_overflow = vec![
        &ctx.env,
        TradeOperation {
            asset_in: ctx.usdc_addr.clone(),
            asset_out: ctx.eurc_addr.clone(),
            amount_in: i128::MAX,
            min_amount_out: 1,
            route: TradeRoute::Soroswap,
        },
    ];

    let proposal = RebalanceProposal {
        vault: ctx.vault.clone(),
        policy_hash: current_policy_hash(&ctx.env, &ctx.policy_address, &ctx.vault),
        operations: ops_overflow,
        weights_after: valid_weights,
        deadline: ctx.env.ledger().timestamp() + 3600,
        proposer: ctx.proposer.clone(),
    };

    let res = policy.validate_rebalance(&ctx.vault, &proposal);
    assert_eq!(res.is_valid, false);
    assert_eq!(res.code, PolicyError::SlippageLimitExceeded as u32);
    assert_eq!(res.message, Symbol::new(&ctx.env, "SLIPPAGE_OVERFLOW"));
}

#[test]
fn test_vault_mismatch_rejected() {
    let ctx = setup_test_context();
    let policy = PolicyContractClient::new(&ctx.env, &ctx.policy_address);

    let p_hash = sample_hash(&ctx.env, 0x62);
    let sleeves = sample_sleeves(&ctx.env);

    policy.register_policy(
        &ctx.vault,
        &ctx.owner,
        &p_hash,
        &ctx.registry_address,
        &1000,
        &2500,
        &3000,
        &50,
        &500,
        &sleeves,
    );

    let valid_weights: Vec<(Symbol, u32)> = vec![
        &ctx.env,
        (Symbol::new(&ctx.env, "CIRCLE_USDC"), 2000),
        (Symbol::new(&ctx.env, "CIRCLE_EURC"), 1000),
        (Symbol::new(&ctx.env, "FRANKLIN_BENJI"), 2500),
        (Symbol::new(&ctx.env, "SPIKO_EUSTBL"), 2500),
        (Symbol::new(&ctx.env, "BLEND_BLND"), 2000),
    ];

    let other_vault = Address::generate(&ctx.env);
    let proposal = RebalanceProposal {
        vault: other_vault, // Vault mismatch!
        policy_hash: current_policy_hash(&ctx.env, &ctx.policy_address, &ctx.vault),
        operations: vec![&ctx.env],
        weights_after: valid_weights,
        deadline: ctx.env.ledger().timestamp() + 3600,
        proposer: ctx.proposer.clone(),
    };

    let res = policy.validate_rebalance(&ctx.vault, &proposal);
    assert_eq!(res.is_valid, false);
    assert_eq!(res.code, PolicyError::Unauthorized as u32);
    assert_eq!(res.message, Symbol::new(&ctx.env, "VAULT_MISMATCH"));
}

#[test]
fn test_weights_after_with_deactivated_asset_rejected() {
    let ctx = setup_test_context();
    let policy = PolicyContractClient::new(&ctx.env, &ctx.policy_address);

    let p_hash = sample_hash(&ctx.env, 0x63);

    // Sleeves include RISKY_TOKEN (which was deactivated in setup_test_context)
    let sleeves = vec![
        &ctx.env,
        SleeveTarget {
            asset_id: Symbol::new(&ctx.env, "CIRCLE_USDC"),
            target_weight_bps: 2000,
            min_band_bps: 1000,
            max_band_bps: 2500,
            sleeve_type: Symbol::new(&ctx.env, "cash"),
        },
        SleeveTarget {
            asset_id: Symbol::new(&ctx.env, "RISKY_TOKEN"),
            target_weight_bps: 1000,
            min_band_bps: 500,
            max_band_bps: 1500,
            sleeve_type: Symbol::new(&ctx.env, "usd_gov_fund"),
        },
        SleeveTarget {
            asset_id: Symbol::new(&ctx.env, "FRANKLIN_BENJI"),
            target_weight_bps: 2500,
            min_band_bps: 1500,
            max_band_bps: 2500,
            sleeve_type: Symbol::new(&ctx.env, "usd_gov_fund"),
        },
        SleeveTarget {
            asset_id: Symbol::new(&ctx.env, "SPIKO_EUSTBL"),
            target_weight_bps: 2500,
            min_band_bps: 1500,
            max_band_bps: 2500,
            sleeve_type: Symbol::new(&ctx.env, "eur_sovereign"),
        },
        SleeveTarget {
            asset_id: Symbol::new(&ctx.env, "BLEND_BLND"),
            target_weight_bps: 2000,
            min_band_bps: 500,
            max_band_bps: 2500,
            sleeve_type: Symbol::new(&ctx.env, "usd_gov_fund"),
        },
    ];

    policy.register_policy(
        &ctx.vault,
        &ctx.owner,
        &p_hash,
        &ctx.registry_address,
        &1000,
        &2500,
        &3000,
        &50,
        &500,
        &sleeves,
    );

    // Proposal allocates 1000 bps to RISKY_TOKEN with NO operations
    // Previously passed because check 8 only inspected operations!
    let proposal = RebalanceProposal {
        vault: ctx.vault.clone(),
        policy_hash: current_policy_hash(&ctx.env, &ctx.policy_address, &ctx.vault),
        operations: vec![&ctx.env],
        weights_after: vec![
            &ctx.env,
            (Symbol::new(&ctx.env, "CIRCLE_USDC"), 2000),
            (Symbol::new(&ctx.env, "RISKY_TOKEN"), 1000),
            (Symbol::new(&ctx.env, "FRANKLIN_BENJI"), 2500),
            (Symbol::new(&ctx.env, "SPIKO_EUSTBL"), 2500),
            (Symbol::new(&ctx.env, "BLEND_BLND"), 2000),
        ],
        deadline: ctx.env.ledger().timestamp() + 3600,
        proposer: ctx.proposer.clone(),
    };

    let res = policy.validate_rebalance(&ctx.vault, &proposal);
    assert_eq!(res.is_valid, false);
    assert_eq!(res.code, PolicyError::AssetNotRegistered as u32);
    assert_eq!(res.message, Symbol::new(&ctx.env, "ASSET_NOT_REGISTERED"));
}

#[test]
fn test_identical_trade_assets_rejected() {
    let ctx = setup_test_context();
    let policy = PolicyContractClient::new(&ctx.env, &ctx.policy_address);

    let p_hash = sample_hash(&ctx.env, 0x64);
    let sleeves = sample_sleeves(&ctx.env);

    policy.register_policy(
        &ctx.vault,
        &ctx.owner,
        &p_hash,
        &ctx.registry_address,
        &1000,
        &2500,
        &3000,
        &50,
        &500,
        &sleeves,
    );

    let valid_weights: Vec<(Symbol, u32)> = vec![
        &ctx.env,
        (Symbol::new(&ctx.env, "CIRCLE_USDC"), 2000),
        (Symbol::new(&ctx.env, "CIRCLE_EURC"), 1000),
        (Symbol::new(&ctx.env, "FRANKLIN_BENJI"), 2500),
        (Symbol::new(&ctx.env, "SPIKO_EUSTBL"), 2500),
        (Symbol::new(&ctx.env, "BLEND_BLND"), 2000),
    ];

    let ops_identical = vec![
        &ctx.env,
        TradeOperation {
            asset_in: ctx.usdc_addr.clone(),
            asset_out: ctx.usdc_addr.clone(), // Identical asset!
            amount_in: 10_000,
            min_amount_out: 9_960,
            route: TradeRoute::Soroswap,
        },
    ];

    let proposal = RebalanceProposal {
        vault: ctx.vault.clone(),
        policy_hash: current_policy_hash(&ctx.env, &ctx.policy_address, &ctx.vault),
        operations: ops_identical,
        weights_after: valid_weights,
        deadline: ctx.env.ledger().timestamp() + 3600,
        proposer: ctx.proposer.clone(),
    };

    let res = policy.validate_rebalance(&ctx.vault, &proposal);
    assert_eq!(res.is_valid, false);
    assert_eq!(res.code, PolicyError::SlippageLimitExceeded as u32);
    assert_eq!(res.message, Symbol::new(&ctx.env, "IDENTICAL_ASSETS"));
}

#[test]
fn test_negative_or_zero_min_amount_out_rejected() {
    let ctx = setup_test_context();
    let policy = PolicyContractClient::new(&ctx.env, &ctx.policy_address);

    let p_hash = sample_hash(&ctx.env, 0x65);
    let sleeves = sample_sleeves(&ctx.env);

    policy.register_policy(
        &ctx.vault,
        &ctx.owner,
        &p_hash,
        &ctx.registry_address,
        &1000,
        &2500,
        &3000,
        &50,
        &500,
        &sleeves,
    );

    let valid_weights: Vec<(Symbol, u32)> = vec![
        &ctx.env,
        (Symbol::new(&ctx.env, "CIRCLE_USDC"), 2000),
        (Symbol::new(&ctx.env, "CIRCLE_EURC"), 1000),
        (Symbol::new(&ctx.env, "FRANKLIN_BENJI"), 2500),
        (Symbol::new(&ctx.env, "SPIKO_EUSTBL"), 2500),
        (Symbol::new(&ctx.env, "BLEND_BLND"), 2000),
    ];

    let ops_zero_out = vec![
        &ctx.env,
        TradeOperation {
            asset_in: ctx.usdc_addr.clone(),
            asset_out: ctx.eurc_addr.clone(),
            amount_in: 10_000,
            min_amount_out: 0, // min_amount_out == 0
            route: TradeRoute::Soroswap,
        },
    ];

    let proposal = RebalanceProposal {
        vault: ctx.vault.clone(),
        policy_hash: current_policy_hash(&ctx.env, &ctx.policy_address, &ctx.vault),
        operations: ops_zero_out,
        weights_after: valid_weights,
        deadline: ctx.env.ledger().timestamp() + 3600,
        proposer: ctx.proposer.clone(),
    };

    let res = policy.validate_rebalance(&ctx.vault, &proposal);
    assert_eq!(res.is_valid, false);
    assert_eq!(res.code, PolicyError::SlippageLimitExceeded as u32);
}

#[test]
fn test_policy_registration_invalid_parameters_rejected() {
    let ctx = setup_test_context();
    let policy = PolicyContractClient::new(&ctx.env, &ctx.policy_address);
    let p_hash = sample_hash(&ctx.env, 0x66);

    // 1. max_slippage_bps > 10000
    let res_slippage = policy.try_register_policy(
        &ctx.vault,
        &ctx.owner,
        &p_hash,
        &ctx.registry_address,
        &1000,
        &2500,
        &3000,
        &10001, // > 10000
        &500,
        &sample_sleeves(&ctx.env),
    );
    assert_eq!(
        res_slippage,
        Err(Ok(soroban_sdk::Error::from_contract_error(
            PolicyError::SlippageLimitExceeded as u32
        )))
    );

    // 2. min_liquidity_sleeve_bps > 10000
    let res_liq = policy.try_register_policy(
        &ctx.vault,
        &ctx.owner,
        &p_hash,
        &ctx.registry_address,
        &10001, // > 10000
        &2500,
        &3000,
        &50,
        &500,
        &sample_sleeves(&ctx.env),
    );
    assert_eq!(
        res_liq,
        Err(Ok(soroban_sdk::Error::from_contract_error(
            PolicyError::LiquidityFloorBreached as u32
        )))
    );

    // 3. Inverted sleeve bands: min_band_bps > max_band_bps
    let inverted_sleeves = vec![
        &ctx.env,
        SleeveTarget {
            asset_id: Symbol::new(&ctx.env, "CIRCLE_USDC"),
            target_weight_bps: 10000,
            min_band_bps: 3000,
            max_band_bps: 2000, // min > max!
            sleeve_type: Symbol::new(&ctx.env, "cash"),
        },
    ];
    let res_band = policy.try_register_policy(
        &ctx.vault,
        &ctx.owner,
        &p_hash,
        &ctx.registry_address,
        &1000,
        &2500,
        &3000,
        &50,
        &500,
        &inverted_sleeves,
    );
    assert_eq!(
        res_band,
        Err(Ok(soroban_sdk::Error::from_contract_error(
            PolicyError::WeightBandExceeded as u32
        )))
    );

    // 4. Duplicate sleeves in registration
    let duplicate_sleeves = vec![
        &ctx.env,
        SleeveTarget {
            asset_id: Symbol::new(&ctx.env, "CIRCLE_USDC"),
            target_weight_bps: 5000,
            min_band_bps: 1000,
            max_band_bps: 6000,
            sleeve_type: Symbol::new(&ctx.env, "cash"),
        },
        SleeveTarget {
            asset_id: Symbol::new(&ctx.env, "CIRCLE_USDC"), // Duplicate asset_id!
            target_weight_bps: 5000,
            min_band_bps: 1000,
            max_band_bps: 6000,
            sleeve_type: Symbol::new(&ctx.env, "cash"),
        },
    ];
    let res_dup = policy.try_register_policy(
        &ctx.vault,
        &ctx.owner,
        &p_hash,
        &ctx.registry_address,
        &1000,
        &2500,
        &3000,
        &50,
        &500,
        &duplicate_sleeves,
    );
    assert_eq!(
        res_dup,
        Err(Ok(soroban_sdk::Error::from_contract_error(
            PolicyError::WeightBandExceeded as u32
        )))
    );
}

// -------------------------------------------------------------------------
// Audit P3 (2026-11-15): only the policy owner (with the vault's consent)
// may register; only the owner may amend; the admin can do neither.
// Every call under test uses an explicit auth tree (AGENT-RULES Rule 13).
// -------------------------------------------------------------------------
mod p3 {
    use super::*;
    use soroban_sdk::testutils::{MockAuth, MockAuthInvoke};
    use soroban_sdk::{IntoVal, Val};

    fn reg_args(ctx: &TestContext, hash: &BytesN<32>) -> Vec<Val> {
        (
            ctx.vault.clone(),
            ctx.owner.clone(),
            hash.clone(),
            ctx.registry_address.clone(),
            1000u32,
            2500u32,
            3000u32,
            50u32,
            500u32,
            sample_sleeves(&ctx.env),
        )
            .into_val(&ctx.env)
    }

    fn try_register(ctx: &TestContext, signers: &[&Address], hash: &BytesN<32>) -> bool {
        let policy = PolicyContractClient::new(&ctx.env, &ctx.policy_address);
        let args = reg_args(ctx, hash);
        let invoke = MockAuthInvoke {
            contract: &ctx.policy_address,
            fn_name: "register_policy",
            args,
            sub_invokes: &[],
        };
        let auths: std::vec::Vec<MockAuth> =
            signers.iter().map(|a| MockAuth { address: a, invoke: &invoke }).collect();
        ctx.env.mock_auths(&auths);
        policy
            .try_register_policy(
                &ctx.vault, &ctx.owner, hash, &ctx.registry_address,
                &1000, &2500, &3000, &50, &500, &sample_sleeves(&ctx.env),
            )
            .is_ok()
    }

    fn try_amend(ctx: &TestContext, signer: &Address, hash: &BytesN<32>, floor: u32) -> bool {
        let policy = PolicyContractClient::new(&ctx.env, &ctx.policy_address);
        let sleeves = sample_sleeves(&ctx.env);
        ctx.env.mock_auths(&[MockAuth {
            address: signer,
            invoke: &MockAuthInvoke {
                contract: &ctx.policy_address,
                fn_name: "update_policy",
                args: (ctx.vault.clone(), hash.clone(), floor, 2500u32, 3000u32, 50u32, 500u32, sleeves.clone())
                    .into_val(&ctx.env),
                sub_invokes: &[],
            },
        }]);
        policy
            .try_update_policy(&ctx.vault, hash, &floor, &2500, &3000, &50, &500, &sleeves)
            .is_ok()
    }

    #[test]
    fn register_requires_owner_and_vault() {
        let ctx = setup_test_context();
        let h = sample_hash(&ctx.env, 0x11);
        assert!(!try_register(&ctx, &[&ctx.admin], &h), "admin alone");
        assert!(!try_register(&ctx, &[&ctx.owner], &h), "owner without vault consent");
        assert!(!try_register(&ctx, &[&ctx.vault], &h), "vault without owner signature");
        assert!(!try_register(&ctx, &[&ctx.admin, &ctx.vault], &h), "admin + vault, no owner");

        assert!(try_register(&ctx, &[&ctx.owner, &ctx.vault], &h));
        let signers: std::vec::Vec<Address> = ctx.env.auths().iter().map(|a| a.0.clone()).collect();
        assert!(signers.contains(&ctx.owner) && signers.contains(&ctx.vault));
        assert!(!signers.contains(&ctx.admin));

        let policy = PolicyContractClient::new(&ctx.env, &ctx.policy_address);
        assert_eq!(policy.get_policy_config(&ctx.vault).unwrap().owner, ctx.owner);
    }

    #[test]
    fn policy_cannot_be_registered_twice() {
        let ctx = setup_test_context();
        assert!(try_register(&ctx, &[&ctx.owner, &ctx.vault], &sample_hash(&ctx.env, 0x11)));
        assert!(!try_register(&ctx, &[&ctx.owner, &ctx.vault], &sample_hash(&ctx.env, 0x22)));
        let policy = PolicyContractClient::new(&ctx.env, &ctx.policy_address);
        assert_eq!(policy.get_policy_hash(&ctx.vault), sample_hash(&ctx.env, 0x11));
        assert_eq!(policy.get_policy_config(&ctx.vault).unwrap().version, 1);
    }

    #[test]
    fn only_owner_can_amend() {
        let ctx = setup_test_context();
        let policy = PolicyContractClient::new(&ctx.env, &ctx.policy_address);
        assert!(try_register(&ctx, &[&ctx.owner, &ctx.vault], &sample_hash(&ctx.env, 0x11)));

        let intruder = Address::generate(&ctx.env);
        for who in [&ctx.admin, &ctx.vault, &ctx.proposer, &intruder] {
            assert!(!try_amend(&ctx, who, &sample_hash(&ctx.env, 0xEE), 0));
        }
        assert_eq!(policy.get_policy_config(&ctx.vault).unwrap().min_liquidity_sleeve_bps, 1000);

        assert!(try_amend(&ctx, &ctx.owner, &sample_hash(&ctx.env, 0x33), 1500));
        assert_eq!(ctx.env.auths()[0].0, ctx.owner);
        // Audit P4: amendments apply only after the notice period.
        ctx.env.ledger().set_timestamp(ctx.env.ledger().timestamp() + PROTOCOL_MIN_NOTICE_SECONDS);
        PolicyContractClient::new(&ctx.env, &ctx.policy_address).activate_policy_update(&ctx.vault);
        let cfg = policy.get_policy_config(&ctx.vault).unwrap();
        assert_eq!((cfg.version, cfg.min_liquidity_sleeve_bps), (2, 1500));
        assert_eq!(cfg.policy_hash, sample_hash(&ctx.env, 0x33));
    }

    #[test]
    fn terms_hash_tracks_exactly_the_stored_terms() {
        let ctx = setup_test_context();
        let policy = PolicyContractClient::new(&ctx.env, &ctx.policy_address);
        assert!(try_register(&ctx, &[&ctx.owner, &ctx.vault], &sample_hash(&ctx.env, 0x11)));
        let sleeves = sample_sleeves(&ctx.env);
        let expected = compute_terms_hash(&ctx.env, &ctx.registry_address, 1000, 2500, 3000, 50, 500, &sleeves);
        let before = policy.get_policy_config(&ctx.vault).unwrap().terms_hash;
        assert_eq!(before, expected);

        assert!(try_amend(&ctx, &ctx.owner, &sample_hash(&ctx.env, 0x33), 1500));
        // Audit P4: amendments apply only after the notice period.
        ctx.env.ledger().set_timestamp(ctx.env.ledger().timestamp() + PROTOCOL_MIN_NOTICE_SECONDS);
        PolicyContractClient::new(&ctx.env, &ctx.policy_address).activate_policy_update(&ctx.vault);
        let after = policy.get_policy_config(&ctx.vault).unwrap().terms_hash;
        assert_ne!(after, before);
        assert_eq!(after, compute_terms_hash(&ctx.env, &ctx.registry_address, 1500, 2500, 3000, 50, 500, &sleeves));
    }

    #[test]
    fn amending_unknown_vault_fails() {
        let ctx = setup_test_context();
        assert!(!try_amend(&ctx, &ctx.owner, &sample_hash(&ctx.env, 0x33), 1000));
    }
}

// -------------------------------------------------------------------------
// Audit P1 (2026-11-15): slippage is measured in value using oracle prices,
// not raw token units. EUR/USD = 1.08, cap = 50 bps.
// -------------------------------------------------------------------------
mod p1 {
    use super::*;

    const ONE: i128 = 10_000_000;

    /// Registers FX_EURC (priced 1.08 by two feeds) and a USDC/FX_EURC policy.
    fn fx_setup(ctx: &TestContext) -> Address {
        let eur = ctx.env.register_stellar_asset_contract_v2(Address::generate(&ctx.env)).address();
        register_registry_asset(&ctx.env, &AssetRegistryClient::new(&ctx.env, &ctx.registry_address), &eur, "FX_EURC", "CIRCLE", SleeveType::Cash);
        price_asset(&ctx.env, &NavOracleClient::new(&ctx.env, &ctx.oracle_address), &eur, "P_FX", ONE * 108 / 100);
        let sleeves = vec![
            &ctx.env,
            SleeveTarget { asset_id: Symbol::new(&ctx.env, "CIRCLE_USDC"), target_weight_bps: 5000, min_band_bps: 0, max_band_bps: 10000, sleeve_type: Symbol::new(&ctx.env, "cash") },
            SleeveTarget { asset_id: Symbol::new(&ctx.env, "FX_EURC"), target_weight_bps: 5000, min_band_bps: 0, max_band_bps: 10000, sleeve_type: Symbol::new(&ctx.env, "cash") },
        ];
        PolicyContractClient::new(&ctx.env, &ctx.policy_address).register_policy(
            &ctx.vault, &ctx.owner, &sample_hash(&ctx.env, 0x01), &ctx.registry_address,
            &1000, &10000, &10000, &50, &500, &sleeves,
        );
        eur
    }

    fn check(ctx: &TestContext, a_in: &Address, a_out: &Address, amount_in: i128, min_out: i128) -> ValidationResult {
        let proposal = RebalanceProposal {
            vault: ctx.vault.clone(),
            policy_hash: current_policy_hash(&ctx.env, &ctx.policy_address, &ctx.vault),
            operations: vec![&ctx.env, TradeOperation { asset_in: a_in.clone(), asset_out: a_out.clone(), amount_in, min_amount_out: min_out, route: TradeRoute::Soroswap }],
            weights_after: vec![&ctx.env, (Symbol::new(&ctx.env, "CIRCLE_USDC"), 5000u32), (Symbol::new(&ctx.env, "FX_EURC"), 5000u32)],
            deadline: ctx.env.ledger().timestamp() + 600,
            proposer: ctx.proposer.clone(),
        };
        PolicyContractClient::new(&ctx.env, &ctx.policy_address).validate_rebalance(&ctx.vault, &proposal)
    }

    #[test]
    fn fair_trades_accepted_in_both_directions() {
        let ctx = setup_test_context();
        let eur = fx_setup(&ctx);
        // 1000 USDC = 925.93 EUR; 925 EUR is ~0.1% slippage.
        assert!(check(&ctx, &ctx.usdc_addr, &eur, 1000 * ONE, 925 * ONE).is_valid);
        // 1000 EUR = 1080 USDC; 1076 USDC is ~0.37% slippage.
        assert!(check(&ctx, &eur, &ctx.usdc_addr, 1000 * ONE, 1076 * ONE).is_valid);
    }

    #[test]
    fn losses_beyond_cap_rejected_in_both_directions() {
        let ctx = setup_test_context();
        let eur = fx_setup(&ctx);
        let r = check(&ctx, &eur, &ctx.usdc_addr, 1000 * ONE, 1070 * ONE); // 0.93% loss
        assert_eq!((r.is_valid, r.code), (false, PolicyError::SlippageLimitExceeded as u32));
        assert_eq!(r.message, Symbol::new(&ctx.env, "SLIPPAGE_EXCEEDED"));
        let r = check(&ctx, &ctx.usdc_addr, &eur, 1000 * ONE, 915 * ONE); // 1.2% loss
        assert!(!r.is_valid);
        // The old raw-unit check accepted this 7.4% loss:
        assert!(!check(&ctx, &eur, &ctx.usdc_addr, 1000 * ONE, 1000 * ONE).is_valid);
    }

    #[test]
    fn exact_cap_boundary() {
        let ctx = setup_test_context();
        let eur = fx_setup(&ctx);
        // value_in = 1000 USDC; at 50 bps the minimum value out is 995 USDC,
        // i.e. 995 / 1.08 = 921.2962962... EUR -> 9_212_962_963 units.
        assert!(check(&ctx, &ctx.usdc_addr, &eur, 1000 * ONE, 9_212_962_963).is_valid);
        assert!(!check(&ctx, &ctx.usdc_addr, &eur, 1000 * ONE, 9_212_962_962).is_valid);
    }

    #[test]
    fn unpriced_asset_makes_proposal_invalid() {
        let ctx = setup_test_context();
        let eur = fx_setup(&ctx);
        let oracle = NavOracleClient::new(&ctx.env, &ctx.oracle_address);
        oracle.trigger_circuit_breaker(&ctx.admin, &Symbol::new(&ctx.env, "P_FX"), &Symbol::new(&ctx.env, "MANUAL"));
        let r = check(&ctx, &ctx.usdc_addr, &eur, 1000 * ONE, 925 * ONE);
        assert_eq!((r.is_valid, r.code), (false, PolicyError::PriceUnavailable as u32));
        assert_eq!(r.message, Symbol::new(&ctx.env, "PRICE_UNAVAILABLE"));
    }

    #[test]
    fn oracle_is_fixed_at_initialization() {
        let ctx = setup_test_context();
        let policy = PolicyContractClient::new(&ctx.env, &ctx.policy_address);
        assert_eq!(policy.get_oracle(), ctx.oracle_address);
        // Re-initializing (the only way to set it) is refused.
        assert!(policy.try_initialize(&ctx.admin, &Address::generate(&ctx.env)).is_err());
        assert_eq!(policy.get_oracle(), ctx.oracle_address);
    }
}

// -------------------------------------------------------------------------
// Audit P2 (2026-11-15): issuer caps use the registry's issuer field.
// -------------------------------------------------------------------------
mod p2 {
    use super::*;

    fn policy_for(ctx: &TestContext, ids: &[(&str, &str)], max_issuer: u32) {
        let mut sleeves = Vec::new(&ctx.env);
        for (id, st) in ids {
            sleeves.push_back(SleeveTarget {
                asset_id: Symbol::new(&ctx.env, id),
                target_weight_bps: 0,
                min_band_bps: 0,
                max_band_bps: 10000,
                sleeve_type: Symbol::new(&ctx.env, st),
            });
        }
        PolicyContractClient::new(&ctx.env, &ctx.policy_address).register_policy(
            &ctx.vault, &ctx.owner, &sample_hash(&ctx.env, 0x02), &ctx.registry_address,
            &1000, &10000, &max_issuer, &50, &500, &sleeves,
        );
    }

    fn validate(ctx: &TestContext, weights: &[(&str, u32)]) -> ValidationResult {
        let mut w = Vec::new(&ctx.env);
        for (id, bps) in weights {
            w.push_back((Symbol::new(&ctx.env, id), *bps));
        }
        PolicyContractClient::new(&ctx.env, &ctx.policy_address).validate_rebalance(
            &ctx.vault,
            &RebalanceProposal {
                vault: ctx.vault.clone(),
                policy_hash: current_policy_hash(&ctx.env, &ctx.policy_address, &ctx.vault),
                operations: Vec::new(&ctx.env),
                weights_after: w,
                deadline: ctx.env.ledger().timestamp() + 600,
                proposer: ctx.proposer.clone(),
            },
        )
    }

    #[test]
    fn assets_of_one_issuer_share_its_cap() {
        let ctx = setup_test_context();
        policy_for(&ctx, &[("CIRCLE_USDC", "cash"), ("CIRCLE_EURC", "cash"), ("FRANKLIN_BENJI", "usd_gov_fund"), ("SPIKO_EUSTBL", "eur_sovereign")], 4000);
        // CIRCLE_USDC and CIRCLE_EURC are both issuer CIRCLE in the registry.
        let r = validate(&ctx, &[("CIRCLE_USDC", 3000), ("CIRCLE_EURC", 3000), ("FRANKLIN_BENJI", 4000)]);
        assert_eq!((r.is_valid, r.code), (false, PolicyError::IssuerCapExceeded as u32));
        assert!(validate(&ctx, &[("CIRCLE_USDC", 2000), ("CIRCLE_EURC", 2000), ("FRANKLIN_BENJI", 6000)]).code
            == PolicyError::IssuerCapExceeded as u32, "Franklin at 60% also breaches");
        let ok = validate(&ctx, &[("CIRCLE_USDC", 2000), ("CIRCLE_EURC", 2000), ("FRANKLIN_BENJI", 4000), ("SPIKO_EUSTBL", 2000)]);
        assert!(ok.is_valid, "40/40/20 respects a 40% cap (code {})", ok.code);
    }

    #[test]
    fn shared_prefix_with_different_issuers_is_not_merged() {
        let ctx = setup_test_context();
        let reg = AssetRegistryClient::new(&ctx.env, &ctx.registry_address);
        let one = ctx.env.register_stellar_asset_contract_v2(Address::generate(&ctx.env)).address();
        let two = ctx.env.register_stellar_asset_contract_v2(Address::generate(&ctx.env)).address();
        register_registry_asset(&ctx.env, &reg, &one, "ACME_ONE", "ACME_INC", SleeveType::UsdGovFund);
        register_registry_asset(&ctx.env, &reg, &two, "ACME_TWO", "OTHER_CO", SleeveType::UsdGovFund);
        policy_for(&ctx, &[("CIRCLE_USDC", "cash"), ("ACME_ONE", "usd_gov_fund"), ("ACME_TWO", "usd_gov_fund")], 4000);
        // A prefix guess would merge ACME_ONE + ACME_TWO into 70% and reject.
        let r = validate(&ctx, &[("CIRCLE_USDC", 3000), ("ACME_ONE", 3500), ("ACME_TWO", 3500)]);
        assert!(r.is_valid, "different registered issuers must not be merged (code {})", r.code);
    }
}

// -------------------------------------------------------------------------
// Audit M1 (2026-11-15): proposals must target the CURRENT policy hash, and
// may only BUY assets declared (with a non-zero band) in the signed policy.
// -------------------------------------------------------------------------
mod m1 {
    use super::*;

    const ONE: i128 = 10_000_000;

    fn signed_policy(ctx: &TestContext, hash: u8) {
        let sleeves = vec![
            &ctx.env,
            SleeveTarget { asset_id: Symbol::new(&ctx.env, "CIRCLE_USDC"), target_weight_bps: 5000, min_band_bps: 0, max_band_bps: 10000, sleeve_type: Symbol::new(&ctx.env, "cash") },
            // Declared but capped at zero: holding/buying it is not allowed.
            SleeveTarget { asset_id: Symbol::new(&ctx.env, "CIRCLE_EURC"), target_weight_bps: 0, min_band_bps: 0, max_band_bps: 0, sleeve_type: Symbol::new(&ctx.env, "cash") },
            SleeveTarget { asset_id: Symbol::new(&ctx.env, "FRANKLIN_BENJI"), target_weight_bps: 5000, min_band_bps: 0, max_band_bps: 10000, sleeve_type: Symbol::new(&ctx.env, "usd_gov_fund") },
        ];
        PolicyContractClient::new(&ctx.env, &ctx.policy_address).register_policy(
            &ctx.vault, &ctx.owner, &sample_hash(&ctx.env, hash), &ctx.registry_address,
            &1000, &10000, &10000, &50, &500, &sleeves,
        );
    }

    fn proposal(ctx: &TestContext, hash: BytesN<32>, ops: Vec<TradeOperation>) -> RebalanceProposal {
        RebalanceProposal {
            vault: ctx.vault.clone(),
            policy_hash: hash,
            operations: ops,
            weights_after: vec![
                &ctx.env,
                (Symbol::new(&ctx.env, "CIRCLE_USDC"), 5000u32),
                (Symbol::new(&ctx.env, "CIRCLE_EURC"), 0u32),
                (Symbol::new(&ctx.env, "FRANKLIN_BENJI"), 5000u32),
            ],
            deadline: ctx.env.ledger().timestamp() + 600,
            proposer: ctx.proposer.clone(),
        }
    }

    fn buy(ctx: &TestContext, a_in: &Address, a_out: &Address) -> ValidationResult {
        let ops = vec![&ctx.env, TradeOperation { asset_in: a_in.clone(), asset_out: a_out.clone(), amount_in: 1000 * ONE, min_amount_out: 999 * ONE, route: TradeRoute::Issuer }];
        let p = proposal(ctx, current_policy_hash(&ctx.env, &ctx.policy_address, &ctx.vault), ops);
        PolicyContractClient::new(&ctx.env, &ctx.policy_address).validate_rebalance(&ctx.vault, &p)
    }

    #[test]
    fn proposal_for_an_amended_policy_is_void() {
        let ctx = setup_test_context();
        let policy = PolicyContractClient::new(&ctx.env, &ctx.policy_address);
        signed_policy(&ctx, 0x01);
        let old = sample_hash(&ctx.env, 0x01);
        assert!(policy.validate_rebalance(&ctx.vault, &proposal(&ctx, old.clone(), vec![&ctx.env])).is_valid);

        // Owner amends (re-signs) the policy.
        let new = sample_hash(&ctx.env, 0x02);
        let cfg = policy.get_policy_config(&ctx.vault).unwrap();
        policy.update_policy(&ctx.vault, &new, &cfg.min_liquidity_sleeve_bps, &cfg.max_single_asset_bps,
            &cfg.max_issuer_bps, &cfg.max_slippage_bps, &cfg.rebalance_band_bps, &policy.get_sleeve_targets(&ctx.vault));
        // Audit P4: amendments apply only after the notice period.
        ctx.env.ledger().set_timestamp(ctx.env.ledger().timestamp() + PROTOCOL_MIN_NOTICE_SECONDS);
        PolicyContractClient::new(&ctx.env, &ctx.policy_address).activate_policy_update(&ctx.vault);

        let stale = policy.validate_rebalance(&ctx.vault, &proposal(&ctx, old, vec![&ctx.env]));
        assert_eq!((stale.is_valid, stale.code), (false, PolicyError::PolicyHashMismatch as u32));
        assert_eq!(stale.message, Symbol::new(&ctx.env, "POLICY_HASH_MISMATCH"));
        assert!(policy.validate_rebalance(&ctx.vault, &proposal(&ctx, new, vec![&ctx.env])).is_valid);
    }

    #[test]
    fn may_buy_only_declared_assets_with_nonzero_band() {
        let ctx = setup_test_context();
        signed_policy(&ctx, 0x01);
        assert!(buy(&ctx, &ctx.usdc_addr, &ctx.benji_addr).is_valid, "declared asset");

        let zero_band = buy(&ctx, &ctx.usdc_addr, &ctx.eurc_addr);
        assert_eq!((zero_band.is_valid, zero_band.code), (false, PolicyError::AssetNotInPolicy as u32));

        let undeclared = buy(&ctx, &ctx.usdc_addr, &ctx.eustbl_addr); // registered, not signed
        assert_eq!((undeclared.is_valid, undeclared.code), (false, PolicyError::AssetNotInPolicy as u32));
        assert_eq!(undeclared.message, Symbol::new(&ctx.env, "ASSET_NOT_IN_POLICY"));
    }

    #[test]
    fn selling_an_undeclared_asset_is_allowed() {
        let ctx = setup_test_context();
        signed_policy(&ctx, 0x01);
        // e.g. tokens sent to the vault unsolicited can be sold into the policy.
        let r = buy(&ctx, &ctx.eustbl_addr, &ctx.benji_addr);
        assert!(r.is_valid, "selling into a declared asset must be allowed (code {})", r.code);
    }
}

// -------------------------------------------------------------------------
// Audit P4 (2026-11-15, owner decision D-E): amendments wait a notice period
// (protocol minimum 48h, owner may lengthen, never shorten); anyone may
// activate after it elapses; the owner may cancel; old terms rule meanwhile.
// -------------------------------------------------------------------------
mod p4 {
    use super::*;
    use soroban_sdk::testutils::{MockAuth, MockAuthInvoke};
    use soroban_sdk::IntoVal;

    const H: u64 = 3600;

    fn registered(ctx: &TestContext) -> PolicyContractClient<'_> {
        let policy = PolicyContractClient::new(&ctx.env, &ctx.policy_address);
        policy.register_policy(
            &ctx.vault, &ctx.owner, &sample_hash(&ctx.env, 0x01), &ctx.registry_address,
            &1000, &2500, &3000, &50, &500, &sample_sleeves(&ctx.env),
        );
        policy
    }

    fn propose(ctx: &TestContext, policy: &PolicyContractClient, hash: u8, floor: u32) {
        policy.update_policy(&ctx.vault, &sample_hash(&ctx.env, hash), &floor, &2500, &3000, &50, &500, &sample_sleeves(&ctx.env));
    }

    fn warp(ctx: &TestContext, secs: u64) {
        ctx.env.ledger().set_timestamp(ctx.env.ledger().timestamp() + secs);
    }

    fn code(e: PolicyError) -> Result<(), Result<soroban_sdk::Error, soroban_sdk::InvokeError>> {
        Err(Ok(soroban_sdk::Error::from_contract_error(e as u32)))
    }

    #[test]
    fn amendment_waits_full_notice_then_anyone_activates() {
        let ctx = setup_test_context();
        let policy = registered(&ctx);
        assert_eq!(policy.get_notice_period(&ctx.vault), PROTOCOL_MIN_NOTICE_SECONDS);
        assert_eq!(PROTOCOL_MIN_NOTICE_SECONDS, 48 * H);

        propose(&ctx, &policy, 0x02, 0);
        let pending = policy.get_pending_policy_update(&ctx.vault).unwrap();
        assert_eq!(pending.effective_at, pending.proposed_at + 48 * H);
        assert_eq!(policy.get_policy_config(&ctx.vault).unwrap().min_liquidity_sleeve_bps, 1000);

        warp(&ctx, 48 * H - 1);
        assert_eq!(policy.try_activate_policy_update(&ctx.vault).map(|_| ()), code(PolicyError::NoticeNotElapsed));

        warp(&ctx, 1);
        ctx.env.set_auths(&[]); // permissionless
        policy.activate_policy_update(&ctx.vault);
        let cfg = policy.get_policy_config(&ctx.vault).unwrap();
        assert_eq!((cfg.policy_hash, cfg.version, cfg.min_liquidity_sleeve_bps), (sample_hash(&ctx.env, 0x02), 2, 0));
        assert!(policy.get_pending_policy_update(&ctx.vault).is_none());
        assert_eq!(policy.try_activate_policy_update(&ctx.vault).map(|_| ()), code(PolicyError::NoPendingUpdate));
    }

    #[test]
    fn old_terms_govern_rebalances_during_notice() {
        let ctx = setup_test_context();
        let policy = registered(&ctx);
        propose(&ctx, &policy, 0x02, 0);
        let weights = vec![
            &ctx.env,
            (Symbol::new(&ctx.env, "CIRCLE_USDC"), 2000u32),
            (Symbol::new(&ctx.env, "CIRCLE_EURC"), 1000u32),
            (Symbol::new(&ctx.env, "FRANKLIN_BENJI"), 2500u32),
            (Symbol::new(&ctx.env, "SPIKO_EUSTBL"), 2500u32),
            (Symbol::new(&ctx.env, "BLEND_BLND"), 2000u32),
        ];
        let mk = |hash: u8| RebalanceProposal {
            vault: ctx.vault.clone(),
            policy_hash: sample_hash(&ctx.env, hash),
            operations: vec![&ctx.env],
            weights_after: weights.clone(),
            deadline: ctx.env.ledger().timestamp() + 600,
            proposer: ctx.proposer.clone(),
        };
        let old = policy.validate_rebalance(&ctx.vault, &mk(0x01));
        let new = policy.validate_rebalance(&ctx.vault, &mk(0x02));
        assert_eq!(new.code, PolicyError::PolicyHashMismatch as u32, "pending terms are not yet in force");
        assert_ne!(old.code, PolicyError::PolicyHashMismatch as u32, "current terms still govern");
    }

    #[test]
    fn owner_cancels_admin_cannot() {
        let ctx = setup_test_context();
        let policy = registered(&ctx);
        propose(&ctx, &policy, 0x02, 0);
        ctx.env.mock_auths(&[MockAuth {
            address: &ctx.admin,
            invoke: &MockAuthInvoke { contract: &ctx.policy_address, fn_name: "cancel_policy_update", args: (ctx.vault.clone(),).into_val(&ctx.env), sub_invokes: &[] },
        }]);
        assert!(policy.try_cancel_policy_update(&ctx.vault).is_err());
        assert!(policy.get_pending_policy_update(&ctx.vault).is_some());

        ctx.env.mock_auths(&[MockAuth {
            address: &ctx.owner,
            invoke: &MockAuthInvoke { contract: &ctx.policy_address, fn_name: "cancel_policy_update", args: (ctx.vault.clone(),).into_val(&ctx.env), sub_invokes: &[] },
        }]);
        policy.cancel_policy_update(&ctx.vault);
        assert!(policy.get_pending_policy_update(&ctx.vault).is_none());
        warp(&ctx, 48 * H);
        assert_eq!(policy.try_activate_policy_update(&ctx.vault).map(|_| ()), code(PolicyError::NoPendingUpdate));
        assert_eq!(policy.get_policy_hash(&ctx.vault), sample_hash(&ctx.env, 0x01));
    }

    #[test]
    fn new_proposal_replaces_pending_and_restarts_clock() {
        let ctx = setup_test_context();
        let policy = registered(&ctx);
        propose(&ctx, &policy, 0x02, 1500);
        warp(&ctx, 24 * H);
        propose(&ctx, &policy, 0x03, 2000);
        warp(&ctx, 24 * H); // 48h after the FIRST proposal
        assert_eq!(policy.try_activate_policy_update(&ctx.vault).map(|_| ()), code(PolicyError::NoticeNotElapsed));
        warp(&ctx, 24 * H); // 48h after the replacement
        policy.activate_policy_update(&ctx.vault);
        assert_eq!(policy.get_policy_hash(&ctx.vault), sample_hash(&ctx.env, 0x03));
    }

    #[test]
    fn notice_can_only_be_lengthened_by_the_owner() {
        let ctx = setup_test_context();
        let policy = registered(&ctx);
        assert_eq!(policy.try_set_notice_period(&ctx.vault, &(24 * H)).map(|_| ()), code(PolicyError::NoticeTooShort));
        policy.set_notice_period(&ctx.vault, &(7 * 24 * H));
        assert_eq!(policy.get_notice_period(&ctx.vault), 7 * 24 * H);
        assert_eq!(policy.try_set_notice_period(&ctx.vault, &(72 * H)).map(|_| ()), code(PolicyError::NoticeTooShort));

        ctx.env.mock_auths(&[MockAuth {
            address: &ctx.admin,
            invoke: &MockAuthInvoke { contract: &ctx.policy_address, fn_name: "set_notice_period", args: (ctx.vault.clone(), 30 * 24 * H).into_val(&ctx.env), sub_invokes: &[] },
        }]);
        assert!(policy.try_set_notice_period(&ctx.vault, &(30 * 24 * H)).is_err(), "admin cannot change notice");

        ctx.env.mock_all_auths();
        propose(&ctx, &policy, 0x02, 0);
        let p = policy.get_pending_policy_update(&ctx.vault).unwrap();
        assert_eq!(p.effective_at - p.proposed_at, 7 * 24 * H, "longer notice applies to new proposals");
    }
}
