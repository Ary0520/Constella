#![cfg(test)]
#![allow(clippy::bool_assert_comparison)]

extern crate std;

use super::*;
use constella_registry::{AssetEntry, AssetRegistry, AssetRegistryClient, SleeveType};
use soroban_sdk::{
    testutils::{Address as _, Events, Ledger},
    vec, Address, BytesN, Env, Symbol, Vec,
};

#[allow(dead_code)]
struct TestContext {
    env: Env,
    admin: Address,
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
}

fn register_registry_asset(
    env: &Env,
    registry_client: &AssetRegistryClient,
    contract_addr: &Address,
    asset_id: &str,
    sleeve: SleeveType,
) {
    let entry = AssetEntry {
        contract_address: contract_addr.clone(),
        asset_id: Symbol::new(env, asset_id),
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
    let vault = Address::generate(&env);
    let proposer = Address::generate(&env);

    // Register closed universe asset registry
    let reg_id = env.register(AssetRegistry, ());
    let reg_client = AssetRegistryClient::new(&env, &reg_id);
    reg_client.initialize(&admin);

    let usdc_addr = Address::generate(&env);
    let eurc_addr = Address::generate(&env);
    let benji_addr = Address::generate(&env);
    let eustbl_addr = Address::generate(&env);
    let blend_addr = Address::generate(&env);
    let usdf_addr = Address::generate(&env);
    let deactivated_addr = Address::generate(&env);
    let unregistered_addr = Address::generate(&env);

    register_registry_asset(&env, &reg_client, &usdc_addr, "CIRCLE_USDC", SleeveType::Cash);
    register_registry_asset(&env, &reg_client, &eurc_addr, "CIRCLE_EURC", SleeveType::Cash);
    register_registry_asset(&env, &reg_client, &benji_addr, "FRANKLIN_BENJI", SleeveType::UsdGovFund);
    register_registry_asset(&env, &reg_client, &eustbl_addr, "SPIKO_EUSTBL", SleeveType::EurSovereign);
    register_registry_asset(&env, &reg_client, &blend_addr, "BLEND_BLND", SleeveType::UsdGovFund);
    register_registry_asset(&env, &reg_client, &usdf_addr, "FRANKLIN_USDF", SleeveType::UsdGovFund);
    register_registry_asset(&env, &reg_client, &deactivated_addr, "RISKY_TOKEN", SleeveType::YieldUsd);

    // Deactivate the deactivated asset
    reg_client.deactivate_asset(&Symbol::new(&env, "RISKY_TOKEN"));

    // Register policy contract
    let policy_id = env.register(PolicyContract, ());
    let policy_client = PolicyContractClient::new(&env, &policy_id);
    policy_client.initialize(&admin);

    TestContext {
        env,
        admin,
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
    }
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
    assert_eq!(config.admin, ctx.admin);

    let stored_sleeves = policy.get_sleeve_targets(&ctx.vault);
    assert_eq!(stored_sleeves.len(), 5);

    // 5. Update policy
    let new_hash = sample_hash(&ctx.env, 0xBB);
    policy.update_policy(&ctx.vault, &new_hash, &sleeves);

    // Verify update event IMMEDIATELY after update_policy
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
    let reg_addr = Address::generate(&env);
    let p_hash = sample_hash(&env, 0x01);
    let sleeves = sample_sleeves(&env);

    let reg_res = policy.try_register_policy(
        &vault,
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
    policy.initialize(&admin);

    // Double initialization must fail
    let double_init = policy.try_initialize(&admin);
    assert_eq!(
        double_init,
        Err(Ok(soroban_sdk::Error::from_contract_error(
            PolicyError::AlreadyInitialized as u32
        )))
    );

    // Non-admin caller attempting register_policy without admin auth fails
    env.mock_auths(&[]);
    let unauth_reg = policy.try_register_policy(
        &vault,
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
    let unauth_upd = policy.try_update_policy(&vault, &new_hash, &sleeves);
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
