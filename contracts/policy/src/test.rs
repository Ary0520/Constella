#![cfg(test)]
#![allow(clippy::bool_assert_comparison)]

extern crate std;

use super::*;
use constella_nav_oracle::{NavOracle, NavOracleClient, OracleSource};
use constella_registry::{AssetEntry, AssetRegistry, AssetRegistryClient, SleeveType};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    vec, Address, BytesN, Env, Symbol, Vec,
};

/// SDK 28 returns events as XDR (`ContractEvents`). Convert them back to the
/// `(contract, topics, data)` tuples these assertions use - the same mapping
/// the SDK applies in its own `PartialEq` for `ContractEvents` (W4.1).
fn all_events(env: &soroban_sdk::Env) -> soroban_sdk::Vec<(soroban_sdk::Address, soroban_sdk::Vec<soroban_sdk::Val>, soroban_sdk::Val)> {
    use soroban_sdk::testutils::Events as _;
    use soroban_sdk::{xdr, TryFromVal};
    let mut out = soroban_sdk::Vec::new(env);
    for e in env.events().all().events() {
        let Some(id) = e.contract_id.clone() else { continue };
        let addr = soroban_sdk::Address::try_from_val(env, &xdr::ScAddress::Contract(id)).unwrap();
        let xdr::ContractEventBody::V0(body) = &e.body;
        let mut topics = soroban_sdk::Vec::new(env);
        for t in body.topics.iter() {
            topics.push_back(soroban_sdk::Val::try_from_val(env, t).unwrap());
        }
        out.push_back((addr, topics, soroban_sdk::Val::try_from_val(env, &body.data).unwrap()));
    }
    out
}


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
    let oracle_id = env.register(NavOracle, (admin.clone(), usdc_addr.clone(), 7u32, 300u32));
    let oracle = NavOracleClient::new(&env, &oracle_id);
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
    let policy_id = env.register(PolicyContract, (admin.clone(), oracle_id.clone()));

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
            feeder: Address::generate(env),
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
    let events_after_reg = all_events(&ctx.env);
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
    let events_after_upd = all_events(&ctx.env);
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
    // Audit M3: admin and oracle are set by the constructor, atomically with
    // deployment; there is no uninitialized window and no `initialize`.
    let admin = Address::generate(&env);
    let oracle_addr = Address::generate(&env); // never called in this test
    let policy_id = env.register(PolicyContract, (admin.clone(), oracle_addr.clone()));
    let policy = PolicyContractClient::new(&env, &policy_id);
    assert_eq!(policy.get_admin(), admin);
    let r = env.try_invoke_contract::<(), soroban_sdk::Error>(
        &policy_id,
        &Symbol::new(&env, "initialize"),
        soroban_sdk::vec![
            &env,
            soroban_sdk::IntoVal::into_val(&Address::generate(&env), &env),
            soroban_sdk::IntoVal::into_val(&Address::generate(&env), &env),
        ],
    );
    assert!(!matches!(r, Ok(Ok(()))));
    assert_eq!(policy.get_oracle(), oracle_addr);

    let vault = Address::generate(&env);
    let owner = Address::generate(&env);
    let reg_addr = Address::generate(&env);
    let p_hash = sample_hash(&env, 0x01);
    let sleeves = sample_sleeves(&env);

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
    let events = all_events(&ctx.env);
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

        // A LOOSER floor (D-L: tightening would apply at once) - waits.
        assert!(try_amend(&ctx, &ctx.owner, &sample_hash(&ctx.env, 0x33), 800));
        assert_eq!(ctx.env.auths()[0].0, ctx.owner);
        // Audit P4: amendments apply only after the notice period.
        ctx.env.ledger().set_timestamp(ctx.env.ledger().timestamp() + PROTOCOL_MIN_NOTICE_SECONDS);
        PolicyContractClient::new(&ctx.env, &ctx.policy_address).activate_policy_update(&ctx.vault);
        let cfg = policy.get_policy_config(&ctx.vault).unwrap();
        assert_eq!((cfg.version, cfg.min_liquidity_sleeve_bps), (2, 800));
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

        assert!(try_amend(&ctx, &ctx.owner, &sample_hash(&ctx.env, 0x33), 800)); // looser: waits (D-L)
        // Audit P4: amendments apply only after the notice period.
        ctx.env.ledger().set_timestamp(ctx.env.ledger().timestamp() + PROTOCOL_MIN_NOTICE_SECONDS);
        PolicyContractClient::new(&ctx.env, &ctx.policy_address).activate_policy_update(&ctx.vault);
        let after = policy.get_policy_config(&ctx.vault).unwrap().terms_hash;
        assert_ne!(after, before);
        assert_eq!(after, compute_terms_hash(&ctx.env, &ctx.registry_address, 800, 2500, 3000, 50, 500, &sleeves));
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
        // There is no setter and no `initialize` entrypoint (audit M3).
        let r = ctx.env.try_invoke_contract::<(), soroban_sdk::Error>(
            &ctx.policy_address,
            &Symbol::new(&ctx.env, "initialize"),
            soroban_sdk::vec![
                &ctx.env,
                soroban_sdk::IntoVal::into_val(&ctx.admin, &ctx.env),
                soroban_sdk::IntoVal::into_val(&Address::generate(&ctx.env), &ctx.env),
            ],
        );
        assert!(!matches!(r, Ok(Ok(()))));
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
        // Looser floors (D-L: a tightening would apply at once).
        propose(&ctx, &policy, 0x02, 900);
        warp(&ctx, 24 * H);
        propose(&ctx, &policy, 0x03, 800);
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

// -------------------------------------------------------------------------
// Audit W4.1b (2026-11-15): golden event encodings.
// -------------------------------------------------------------------------
/// Golden event recorder (audit W4.1b). Captures the exact XDR of every event
/// the contract under test emits during a scenario and compares it with the
/// committed `golden/events.txt`, so the `Events::publish` -> `#[contractevent]`
/// migration is proven byte-for-byte. Regenerate ONLY deliberately with
/// `UPDATE_GOLDEN=1 cargo test golden_events`.
struct GoldenEvents {
    contract: soroban_sdk::Address,
    lines: std::vec::Vec<std::string::String>,
    names: std::collections::BTreeSet<std::string::String>,
}

impl GoldenEvents {
    fn new(contract: &soroban_sdk::Address) -> Self {
        Self { contract: contract.clone(), lines: std::vec::Vec::new(), names: Default::default() }
    }

    fn capture(&mut self, env: &soroban_sdk::Env, step: &str) {
        use soroban_sdk::testutils::Events as _;
        use soroban_sdk::xdr::{ContractEventBody, Limits, ScVal, WriteXdr};
        for e in env.events().all().filter_by_contract(&self.contract).events() {
            let ContractEventBody::V0(body) = &e.body;
            let name = match body.topics.first() {
                Some(ScVal::Symbol(s)) => std::string::String::from_utf8_lossy(s.0.as_slice()).into_owned(),
                _ => std::string::String::from("?"),
            };
            let b64 = e.body.to_xdr_base64(Limits::none()).unwrap();
            self.lines.push(std::format!("{step} | {name} | {b64}"));
            self.names.insert(name);
        }
    }

    fn finish(self, expected_names: &[&str]) {
        for n in expected_names {
            assert!(self.names.contains(*n), "scenario never emitted `{n}`; coverage incomplete");
        }
        let dir = std::format!("{}/golden", env!("CARGO_MANIFEST_DIR"));
        let path = std::format!("{dir}/events.txt");
        let text = self.lines.join("\n") + "\n";
        if std::env::var("UPDATE_GOLDEN").as_deref() == Ok("1") {
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(&path, &text).unwrap();
        } else {
            let expected = std::fs::read_to_string(&path)
                .unwrap_or_else(|_| panic!("missing golden file {path}"));
            assert_eq!(expected.replace("\r\n", "\n"), text, "event encoding changed vs {path}");
        }
    }
}

#[test]
fn golden_events_policy() {
    let ctx = setup_test_context();
    let env = &ctx.env;
    let p = PolicyContractClient::new(env, &ctx.policy_address);
    let mut g = GoldenEvents::new(&ctx.policy_address);
    let st = |id: &str, max: u32, ty: &str| SleeveTarget {
        asset_id: Symbol::new(env, id), target_weight_bps: 0, min_band_bps: 0, max_band_bps: max, sleeve_type: Symbol::new(env, ty),
    };
    let sleeves = vec![env, st("CIRCLE_USDC", 10000, "cash"), st("CIRCLE_EURC", 10000, "cash"),
        st("FRANKLIN_BENJI", 10000, "usd_gov_fund"), st("SPIKO_EUSTBL", 0, "eur_sovereign")];
    p.register_policy(&ctx.vault, &ctx.owner, &sample_hash(env, 1), &ctx.registry_address,
        &1000, &10000, &10000, &50, &500, &sleeves);
    g.capture(env, "register_policy");

    let w = |pairs: &[(&str, u32)]| {
        let mut v = Vec::new(env);
        for (a, b) in pairs { v.push_back((Symbol::new(env, a), *b)); }
        v
    };
    let op = |a: &Address, b: &Address, amt: i128, min: i128| vec![env, TradeOperation {
        asset_in: a.clone(), asset_out: b.clone(), amount_in: amt, min_amount_out: min, route: TradeRoute::Soroswap }];
    let prop = |vault: &Address, hash: BytesN<32>, ops: Vec<TradeOperation>, weights: Vec<(Symbol, u32)>, deadline: u64| RebalanceProposal {
        vault: vault.clone(), policy_hash: hash, operations: ops, weights_after: weights, deadline, proposer: ctx.proposer.clone() };
    let now = env.ledger().timestamp();
    let ok_w = || w(&[("CIRCLE_USDC", 5000), ("FRANKLIN_BENJI", 5000)]);
    let h1 = sample_hash(env, 1);
    let other = Address::generate(env);
    let cases: std::vec::Vec<(&str, Address, RebalanceProposal)> = std::vec![
        ("valid", ctx.vault.clone(), prop(&ctx.vault, h1.clone(), vec![env], ok_w(), now + 600)),
        ("policy_not_found", other.clone(), prop(&other, h1.clone(), vec![env], ok_w(), now + 600)),
        ("vault_mismatch", ctx.vault.clone(), prop(&other, h1.clone(), vec![env], ok_w(), now + 600)),
        ("hash_mismatch", ctx.vault.clone(), prop(&ctx.vault, sample_hash(env, 9), vec![env], ok_w(), now + 600)),
        ("deadline", ctx.vault.clone(), prop(&ctx.vault, h1.clone(), vec![env], ok_w(), now - 1)),
        ("weights_sum", ctx.vault.clone(), prop(&ctx.vault, h1.clone(), vec![env], w(&[("CIRCLE_USDC", 5000)]), now + 600)),
        ("duplicate", ctx.vault.clone(), prop(&ctx.vault, h1.clone(), vec![env], w(&[("CIRCLE_USDC", 5000), ("CIRCLE_USDC", 5000)]), now + 600)),
        ("band", ctx.vault.clone(), prop(&ctx.vault, h1.clone(), vec![env], w(&[("CIRCLE_USDC", 9000), ("SPIKO_EUSTBL", 1000)]), now + 600)),
        ("undeclared", ctx.vault.clone(), prop(&ctx.vault, h1.clone(), vec![env], w(&[("CIRCLE_USDC", 5000), ("BLEND_BLND", 5000)]), now + 600)),
        ("liquidity_floor", ctx.vault.clone(), prop(&ctx.vault, h1.clone(), vec![env], w(&[("FRANKLIN_BENJI", 10000)]), now + 600)),
        ("invalid_amount", ctx.vault.clone(), prop(&ctx.vault, h1.clone(), op(&ctx.usdc_addr, &ctx.benji_addr, 0, 0), ok_w(), now + 600)),
        ("identical", ctx.vault.clone(), prop(&ctx.vault, h1.clone(), op(&ctx.usdc_addr, &ctx.usdc_addr, 1000, 999), ok_w(), now + 600)),
        ("unregistered", ctx.vault.clone(), prop(&ctx.vault, h1.clone(), op(&ctx.unregistered_addr, &ctx.benji_addr, 1000, 999), ok_w(), now + 600)),
        ("not_in_policy", ctx.vault.clone(), prop(&ctx.vault, h1.clone(), op(&ctx.usdc_addr, &ctx.eustbl_addr, 1000, 999), ok_w(), now + 600)),
        ("slippage", ctx.vault.clone(), prop(&ctx.vault, h1.clone(), op(&ctx.usdc_addr, &ctx.benji_addr, 1000, 900), ok_w(), now + 600)),
        ("valid_with_trade", ctx.vault.clone(), prop(&ctx.vault, h1.clone(), op(&ctx.usdc_addr, &ctx.benji_addr, 1000, 999), ok_w(), now + 600)),
    ];
    for (name, vault, proposal) in cases.iter() {
        p.validate_rebalance(vault, proposal);
        g.capture(env, name);
    }
    NavOracleClient::new(env, &ctx.oracle_address)
        .trigger_circuit_breaker(&ctx.admin, &Symbol::new(env, "P_BENJI"), &Symbol::new(env, "MANUAL"));
    p.validate_rebalance(&ctx.vault, &prop(&ctx.vault, h1.clone(), op(&ctx.usdc_addr, &ctx.benji_addr, 1000, 999), ok_w(), now + 600));
    g.capture(env, "price_unavailable");

    p.update_policy(&ctx.vault, &sample_hash(env, 2), &1000, &10000, &10000, &50, &500, &sleeves);
    g.capture(env, "update_policy");
    p.set_notice_period(&ctx.vault, &(72 * 3600));
    g.capture(env, "set_notice_period");
    p.cancel_policy_update(&ctx.vault);
    g.capture(env, "cancel_policy_update");
    p.update_policy(&ctx.vault, &sample_hash(env, 3), &900, &10000, &10000, &50, &500, &sleeves); // looser: waits (D-L)
    env.ledger().set_timestamp(now + 72 * 3600);
    p.activate_policy_update(&ctx.vault);
    g.capture(env, "activate_policy_update");
    g.finish(&["POLICY_REGISTERED", "REBALANCE_VALIDATED", "POLICY_UPDATE_PROPOSED", "POLICY_NOTICE_SET",
        "POLICY_UPDATE_CANCELLED", "POLICY_UPDATED"]);
}

// ---------------------------------------------------------------------------
// Audit W1b: validate_holdings checks ACTUAL values against the signed policy.
// sample_sleeves bands: USDC 10-25 %, EURC 5-15 %, BENJI 15-25 %,
// EUSTBL 15-25 %, BLND 5-25 %; USDC and EURC are cash, both issued by CIRCLE.
// ---------------------------------------------------------------------------

const K: i128 = 10_000_000; // one bps of a 10_000-unit basket, in 7-decimal units

/// A fresh vault under sample_sleeves with the given floor / caps.
fn holdings_policy(ctx: &TestContext, floor: u32, single: u32, issuer: u32) -> Address {
    let vault = Address::generate(&ctx.env);
    PolicyContractClient::new(&ctx.env, &ctx.policy_address).register_policy(
        &vault, &ctx.owner, &sample_hash(&ctx.env, 0x1b), &ctx.registry_address,
        &floor, &single, &issuer, &50, &500, &sample_sleeves(&ctx.env),
    );
    vault
}

/// Holdings in bps-of-basket units (x K), in sample_sleeves order.
fn basket(env: &Env, usdc: i128, eurc: i128, benji: i128, eustbl: i128, blnd: i128) -> Vec<(Symbol, i128)> {
    vec![
        env,
        (Symbol::new(env, "CIRCLE_USDC"), usdc),
        (Symbol::new(env, "CIRCLE_EURC"), eurc),
        (Symbol::new(env, "FRANKLIN_BENJI"), benji),
        (Symbol::new(env, "SPIKO_EUSTBL"), eustbl),
        (Symbol::new(env, "BLEND_BLND"), blnd),
    ]
}

fn holdings_code(ctx: &TestContext, vault: &Address, h: &Vec<(Symbol, i128)>) -> (bool, u32) {
    let r = PolicyContractClient::new(&ctx.env, &ctx.policy_address).validate_holdings(vault, h);
    (r.is_valid, r.code)
}

#[test]
fn w1b_holdings_inside_every_band_and_cap_are_valid() {
    let ctx = setup_test_context();
    let vault = holdings_policy(&ctx, 1000, 2500, 3500);
    let h = basket(&ctx.env, 2000 * K, 1000 * K, 2500 * K, 2500 * K, 2000 * K);
    assert_eq!(holdings_code(&ctx, &vault, &h), (true, 0));
}

#[test]
fn w1b_band_edges_are_compared_exactly() {
    let ctx = setup_test_context();
    let vault = holdings_policy(&ctx, 1000, 10_000, 10_000);
    // BENJI exactly 25.00 % is allowed; one stroop more (total unchanged) is not.
    let edge = basket(&ctx.env, 2000 * K, 1000 * K, 2500 * K, 2500 * K, 2000 * K);
    assert_eq!(holdings_code(&ctx, &vault, &edge), (true, 0));
    let over = basket(&ctx.env, 2000 * K, 1000 * K, 2500 * K + 1, 2500 * K, 2000 * K - 1);
    assert_eq!(holdings_code(&ctx, &vault, &over), (false, PolicyError::WeightBandExceeded as u32));
    // BENJI one stroop under its 15 % minimum (the stroop goes to BLND, which
    // stays inside its own band, so only the minimum can catch it).
    let under = basket(&ctx.env, 2500 * K, 1500 * K, 1500 * K - 1, 2500 * K, 2000 * K + 1);
    assert_eq!(holdings_code(&ctx, &vault, &under), (false, PolicyError::WeightBandExceeded as u32));
}

#[test]
fn w1b_a_declared_sleeve_missing_from_holdings_counts_as_zero() {
    let ctx = setup_test_context();
    let vault = holdings_policy(&ctx, 1000, 10_000, 10_000);
    // BLND (min 5 %) omitted entirely.
    let mut h = basket(&ctx.env, 2500 * K, 1500 * K, 2500 * K, 2500 * K, 0);
    h.pop_back();
    h.push_back((Symbol::new(&ctx.env, "CIRCLE_USDC"), 0)); // duplicate id is malformed
    assert_eq!(holdings_code(&ctx, &vault, &h), (false, PolicyError::InvalidHoldings as u32));
    h.pop_back();
    assert_eq!(holdings_code(&ctx, &vault, &h), (false, PolicyError::WeightBandExceeded as u32));
}

#[test]
fn w1b_an_undeclared_holding_is_rejected_unless_empty() {
    let ctx = setup_test_context();
    let vault = holdings_policy(&ctx, 1000, 10_000, 10_000);
    let mut h = basket(&ctx.env, 2000 * K, 1000 * K, 2500 * K, 2500 * K, 2000 * K);
    h.push_back((Symbol::new(&ctx.env, "FRANKLIN_USDF"), 0));
    assert_eq!(holdings_code(&ctx, &vault, &h), (true, 0), "a zero balance of an undeclared asset is harmless");
    h.set(5, (Symbol::new(&ctx.env, "FRANKLIN_USDF"), 1));
    assert_eq!(holdings_code(&ctx, &vault, &h), (false, PolicyError::AssetNotInPolicy as u32));
}

#[test]
fn w1b_single_asset_issuer_and_floor_are_checked_on_values() {
    let ctx = setup_test_context();
    let h = basket(&ctx.env, 2000 * K, 1000 * K, 2500 * K, 2500 * K, 2000 * K);
    let single = holdings_policy(&ctx, 1000, 2000, 10_000);
    assert_eq!(holdings_code(&ctx, &single, &h), (false, PolicyError::SingleAssetCapExceeded as u32));
    // CIRCLE (USDC + EURC) holds 30 %.
    let issuer = holdings_policy(&ctx, 1000, 10_000, 2999);
    assert_eq!(holdings_code(&ctx, &issuer, &h), (false, PolicyError::IssuerCapExceeded as u32));
    let issuer_ok = holdings_policy(&ctx, 1000, 10_000, 3000);
    assert_eq!(holdings_code(&ctx, &issuer_ok, &h), (true, 0));
    // Cash (USDC + EURC) is 30 %; a 30.01 % floor fails, 30 % passes.
    let floor = holdings_policy(&ctx, 3001, 10_000, 10_000);
    assert_eq!(holdings_code(&ctx, &floor, &h), (false, PolicyError::LiquidityFloorBreached as u32));
    let floor_ok = holdings_policy(&ctx, 3000, 10_000, 10_000);
    assert_eq!(holdings_code(&ctx, &floor_ok, &h), (true, 0));
}

#[test]
fn w1b_malformed_or_unknown_inputs_fail_closed() {
    let ctx = setup_test_context();
    let vault = holdings_policy(&ctx, 1000, 10_000, 10_000);
    let neg = basket(&ctx.env, -1, 1000 * K, 2500 * K, 2500 * K, 2000 * K);
    assert_eq!(holdings_code(&ctx, &vault, &neg), (false, PolicyError::InvalidHoldings as u32));
    let empty = basket(&ctx.env, 0, 0, 0, 0, 0);
    assert_eq!(holdings_code(&ctx, &vault, &empty), (false, PolicyError::InvalidHoldings as u32));
    let huge = basket(&ctx.env, i128::MAX / 4, i128::MAX / 8, i128::MAX / 4, i128::MAX / 4, i128::MAX / 8);
    assert_eq!(holdings_code(&ctx, &vault, &huge), (false, PolicyError::InvalidHoldings as u32));
    let stranger = Address::generate(&ctx.env);
    let h = basket(&ctx.env, 2000 * K, 1000 * K, 2500 * K, 2500 * K, 2000 * K);
    assert_eq!(holdings_code(&ctx, &stranger, &h), (false, PolicyError::PolicyNotFound as u32));
}

// ---------------------------------------------------------------------------
// Decision D-H: Blend legs (route Blend) on the REAL Blend v2 pool. The
// position is priced by the oracle from the pool's live b_rate; its bTokens
// use the underlying's decimals.
// ---------------------------------------------------------------------------
mod blend_route {
    use super::*;
    use constella_testkit::blend::{self, pool::Request, BORROW, RATE_SCALE, SCALAR_7, SUPPLY, SUPPLY_COLLATERAL};
    use soroban_sdk::token::StellarAssetClient;

    struct B {
        ctx: TestContext,
        pool: Address,
        factory: blend::pool_factory::Client<'static>,
        policy: PolicyContractClient<'static>,
    }

    fn setup() -> B {
        let ctx = setup_test_context();
        ctx.env.cost_estimate().budget().reset_unlimited();
        let usdc_admin = StellarAssetClient::new(&ctx.env, &ctx.usdc_addr);
        let b = blend::deploy(&ctx.env, &usdc_admin, &ctx.oracle_address);
        let pool = b.pool.address.clone();
        let registry = AssetRegistryClient::new(&ctx.env, &ctx.registry_address);
        register_registry_asset(&ctx.env, &registry, &pool, "BLEND_USDC_SUPPLY", "BLEND", SleeveType::Lending);
        let oracle = NavOracleClient::new(&ctx.env, &ctx.oracle_address);
        oracle.set_blend_factory(&b.factory.address);
        oracle.register_blend_position(&pool, &ctx.usdc_addr);
        let policy = PolicyContractClient::new(&ctx.env, &ctx.policy_address);
        let sleeves = vec![
            &ctx.env,
            SleeveTarget { asset_id: Symbol::new(&ctx.env, "CIRCLE_USDC"), target_weight_bps: 5000, min_band_bps: 0,
                max_band_bps: 10_000, sleeve_type: Symbol::new(&ctx.env, "cash") },
            SleeveTarget { asset_id: Symbol::new(&ctx.env, "BLEND_USDC_SUPPLY"), target_weight_bps: 5000, min_band_bps: 0,
                max_band_bps: 10_000, sleeve_type: Symbol::new(&ctx.env, "lending") },
        ];
        policy.register_policy(&ctx.vault, &ctx.owner, &sample_hash(&ctx.env, 0xb1), &ctx.registry_address,
            &1000, &10_000, &10_000, &50, &500, &sleeves);
        // Real borrowing so the reserve earns interest (b_rate > 1).
        let lender = Address::generate(&ctx.env);
        usdc_admin.mint(&lender, &(100_000 * SCALAR_7));
        b.pool.submit(&lender, &lender, &lender, &vec![&ctx.env, Request { request_type: SUPPLY, address: ctx.usdc_addr.clone(), amount: 100_000 * SCALAR_7 }]);
        let borrower = Address::generate(&ctx.env);
        usdc_admin.mint(&borrower, &(80_000 * SCALAR_7));
        b.pool.submit(&borrower, &borrower, &borrower, &vec![&ctx.env,
            Request { request_type: SUPPLY_COLLATERAL, address: ctx.usdc_addr.clone(), amount: 80_000 * SCALAR_7 },
            Request { request_type: BORROW, address: ctx.usdc_addr.clone(), amount: 40_000 * SCALAR_7 }]);
        // Health: 80k x 0.75 = 60k >= 40k / 0.75 = 53.3k. At this utilization
        // suppliers earn ~0.5 %/year (measured: b_rate 1.00496 after 1 year);
        // 4 years separate b_rate valuation clearly from the 0.5 % slippage cap.
        ctx.env.ledger().set_timestamp(ctx.env.ledger().timestamp() + 4 * 365 * 24 * 3600);
        assert!(b.pool.get_reserve(&ctx.usdc_addr).data.b_rate > RATE_SCALE * 101 / 100, "over 1 % earned");
        B { ctx, pool, factory: b.factory, policy }
    }

    fn leg(b: &B, asset_in: &Address, asset_out: &Address, amount_in: i128, min_out: i128) -> RebalanceProposal {
        let env = &b.ctx.env;
        RebalanceProposal {
            vault: b.ctx.vault.clone(),
            policy_hash: current_policy_hash(env, &b.ctx.policy_address, &b.ctx.vault),
            operations: vec![env, TradeOperation { asset_in: asset_in.clone(), asset_out: asset_out.clone(), amount_in,
                min_amount_out: min_out, route: TradeRoute::Blend }],
            weights_after: vec![env, (Symbol::new(env, "CIRCLE_USDC"), 5000u32), (Symbol::new(env, "BLEND_USDC_SUPPLY"), 5000u32)],
            deadline: env.ledger().timestamp() + 600,
            proposer: b.ctx.proposer.clone(),
        }
    }

    fn b_rate(b: &B) -> i128 {
        NavOracleClient::new(&b.ctx.env, &b.ctx.oracle_address).asset_price(&b.pool).unwrap().price * RATE_SCALE / SCALAR_7
    }

    #[test]
    fn a_supply_leg_is_valued_at_the_live_b_rate() {
        let b = setup();
        let usdc = b.ctx.usdc_addr.clone();
        // 1,000 USDC buys 1,000 / b_rate bTokens; 0.5 % slippage allowed.
        let fair = 1_000 * SCALAR_7 * RATE_SCALE / b_rate(&b);
        assert!(fair < 990 * SCALAR_7, "bTokens are worth more than 1 USDC: not valued 1:1");
        let ok = leg(&b, &usdc, &b.pool, 1_000 * SCALAR_7, fair * 9_951 / 10_000);
        let r = b.policy.validate_rebalance(&b.ctx.vault, &ok);
        assert!(r.is_valid, "{:?}", r.message);
        let greedy = leg(&b, &usdc, &b.pool, 1_000 * SCALAR_7, fair * 9_949 / 10_000);
        assert_eq!(b.policy.validate_rebalance(&b.ctx.vault, &greedy).code, PolicyError::SlippageLimitExceeded as u32);
    }

    #[test]
    fn a_withdraw_leg_is_valued_at_the_live_b_rate() {
        let b = setup();
        let usdc = b.ctx.usdc_addr.clone();
        let worth = 1_000 * SCALAR_7 * b_rate(&b) / RATE_SCALE;
        let ok = leg(&b, &b.pool, &usdc, 1_000 * SCALAR_7, worth * 9_951 / 10_000);
        assert!(b.policy.validate_rebalance(&b.ctx.vault, &ok).is_valid);
        let greedy = leg(&b, &b.pool, &usdc, 1_000 * SCALAR_7, worth * 9_949 / 10_000);
        assert_eq!(b.policy.validate_rebalance(&b.ctx.vault, &greedy).code, PolicyError::SlippageLimitExceeded as u32);
    }

    #[test]
    fn a_blend_position_the_policy_does_not_declare_cannot_be_bought() {
        let b = setup();
        let env = &b.ctx.env;
        // A second real pool of the official factory, registered and priced,
        // but not in the signed policy.
        let other = b.factory.deploy(&Address::generate(env), &soroban_sdk::String::from_str(env, "other"),
            &BytesN::from_array(env, &[9u8; 32]), &b.ctx.oracle_address, &1_000_000, &6, &SCALAR_7);
        blend::add_setup_reserve(env, &other, &b.ctx.usdc_addr);
        let registry = AssetRegistryClient::new(env, &b.ctx.registry_address);
        register_registry_asset(env, &registry, &other, "BLEND_OTHER", "BLEND", SleeveType::Lending);
        NavOracleClient::new(env, &b.ctx.oracle_address).register_blend_position(&other, &b.ctx.usdc_addr);
        let p = leg(&b, &b.ctx.usdc_addr, &other, 1_000 * SCALAR_7, 990 * SCALAR_7);
        assert_eq!(b.policy.validate_rebalance(&b.ctx.vault, &p).code, PolicyError::AssetNotInPolicy as u32);
    }
}

// ---------------------------------------------------------------------------
// Decision D-A / audit A1b: the vault currency, declared as the sleeve with
// type "numeraire", is exempt from the single-asset and issuer caps (counted
// as cash for the floor); every other asset - EURC included - still counts.
// ---------------------------------------------------------------------------
mod a1b {
    use super::*;

    fn sleeve(env: &Env, id: &str, kind: &str) -> SleeveTarget {
        SleeveTarget { asset_id: Symbol::new(env, id), target_weight_bps: 0, min_band_bps: 0, max_band_bps: 10_000,
            sleeve_type: Symbol::new(env, kind) }
    }

    /// A fresh vault with USDC typed `usdc_kind`, EURC cash, BENJI gov fund.
    fn register(ctx: &TestContext, usdc_kind: &str, floor: u32, single: u32, issuer: u32) -> Address {
        let env = &ctx.env;
        let vault = Address::generate(env);
        let sleeves = vec![env, sleeve(env, "CIRCLE_USDC", usdc_kind), sleeve(env, "CIRCLE_EURC", "cash"),
            sleeve(env, "FRANKLIN_BENJI", "usd_gov_fund")];
        PolicyContractClient::new(env, &ctx.policy_address).register_policy(&vault, &ctx.owner, &sample_hash(env, 0xa1),
            &ctx.registry_address, &floor, &single, &issuer, &50, &500, &sleeves);
        vault
    }

    fn holdings(env: &Env, usdc: i128, eurc: i128, benji: i128) -> Vec<(Symbol, i128)> {
        vec![env, (Symbol::new(env, "CIRCLE_USDC"), usdc), (Symbol::new(env, "CIRCLE_EURC"), eurc),
            (Symbol::new(env, "FRANKLIN_BENJI"), benji)]
    }

    fn verdict(ctx: &TestContext, vault: &Address, h: &Vec<(Symbol, i128)>) -> (bool, u32) {
        let r = PolicyContractClient::new(&ctx.env, &ctx.policy_address).validate_holdings(vault, h);
        (r.is_valid, r.code)
    }

    #[test]
    fn a_cash_reserve_vault_passes_only_with_its_currency_declared() {
        let ctx = setup_test_context();
        let h = holdings(&ctx.env, 8_500, 1_500, 0);
        let declared = register(&ctx, "numeraire", 1000, 2500, 3000);
        assert_eq!(verdict(&ctx, &declared, &h), (true, 0));
        // Plain "cash" keeps today's stricter rule: 85 % in one asset breaches the caps.
        let plain = register(&ctx, "cash", 1000, 2500, 3000);
        assert!(!verdict(&ctx, &plain, &h).0);
    }

    #[test]
    fn every_other_asset_still_counts_eurc_included() {
        let ctx = setup_test_context();
        let v = register(&ctx, "numeraire", 1000, 10_000, 3000);
        // EURC alone is 35 % of CIRCLE's non-numéraire share: issuer cap.
        assert_eq!(verdict(&ctx, &v, &holdings(&ctx.env, 6_500, 3_500, 0)), (false, PolicyError::IssuerCapExceeded as u32));
        let s = register(&ctx, "numeraire", 1000, 2500, 10_000);
        assert_eq!(verdict(&ctx, &s, &holdings(&ctx.env, 6_500, 0, 3_500)), (false, PolicyError::SingleAssetCapExceeded as u32));
    }

    #[test]
    fn the_claimed_weights_check_applies_the_same_exemption() {
        let ctx = setup_test_context();
        let env = &ctx.env;
        let policy = PolicyContractClient::new(env, &ctx.policy_address);
        let proposal = |vault: &Address| RebalanceProposal {
            vault: vault.clone(),
            policy_hash: current_policy_hash(env, &ctx.policy_address, vault),
            operations: vec![env],
            weights_after: vec![env, (Symbol::new(env, "CIRCLE_USDC"), 8500u32), (Symbol::new(env, "CIRCLE_EURC"), 1500u32),
                (Symbol::new(env, "FRANKLIN_BENJI"), 0u32)],
            deadline: env.ledger().timestamp() + 3600,
            proposer: ctx.proposer.clone(),
        };
        let declared = register(&ctx, "numeraire", 1000, 2500, 3000);
        assert!(policy.validate_rebalance(&declared, &proposal(&declared)).is_valid);
        let plain = register(&ctx, "cash", 1000, 2500, 3000);
        assert!(!policy.validate_rebalance(&plain, &proposal(&plain)).is_valid);
    }

    #[test]
    fn the_currency_counts_as_cash_for_the_liquidity_floor() {
        let ctx = setup_test_context();
        let ok = register(&ctx, "numeraire", 8500, 10_000, 10_000);
        assert_eq!(verdict(&ctx, &ok, &holdings(&ctx.env, 8_500, 0, 1_500)), (true, 0));
        let high = register(&ctx, "numeraire", 9000, 10_000, 10_000);
        assert_eq!(verdict(&ctx, &high, &holdings(&ctx.env, 8_500, 0, 1_500)), (false, PolicyError::LiquidityFloorBreached as u32));
    }

    #[test]
    fn registration_admits_one_active_cash_currency_spelled_exactly() {
        let ctx = setup_test_context();
        let env = &ctx.env;
        let policy = PolicyContractClient::new(env, &ctx.policy_address);
        let try_reg = |sleeves: Vec<SleeveTarget>| policy.try_register_policy(&Address::generate(env), &ctx.owner,
            &sample_hash(env, 0xa2), &ctx.registry_address, &1000, &2500, &3000, &50, &500, &sleeves);
        let bad = Err(Ok(PolicyError::NumeraireSleeveInvalid.into()));
        assert_eq!(try_reg(vec![env, sleeve(env, "CIRCLE_USDC", "numeraire"), sleeve(env, "CIRCLE_EURC", "numeraire")]), bad,
            "only one vault currency");
        assert_eq!(try_reg(vec![env, sleeve(env, "FRANKLIN_BENJI", "numeraire")]), bad, "must be a registry CASH asset");
        assert_eq!(try_reg(vec![env, sleeve(env, "RISKY_TOKEN", "numeraire")]), bad, "must be active");
        // Any other spelling is an ordinary sleeve - no exemption.
        let v = Address::generate(env);
        policy.register_policy(&v, &ctx.owner, &sample_hash(env, 0xa3), &ctx.registry_address, &1000, &2500, &3000, &50, &500,
            &vec![env, sleeve(env, "CIRCLE_USDC", "Numeraire"), sleeve(env, "CIRCLE_EURC", "cash"), sleeve(env, "FRANKLIN_BENJI", "usd_gov_fund")]);
        assert!(!verdict(&ctx, &v, &holdings(env, 8_500, 1_500, 0)).0);
    }

    #[test]
    fn an_amendment_cannot_change_add_or_remove_the_currency() {
        let ctx = setup_test_context();
        let env = &ctx.env;
        let policy = PolicyContractClient::new(env, &ctx.policy_address);
        let amend = |vault: &Address, sleeves: Vec<SleeveTarget>| policy.try_update_policy(vault, &sample_hash(env, 0xa4),
            &1000, &2500, &3000, &50, &500, &sleeves);
        let bad = Err(Ok(PolicyError::NumeraireSleeveInvalid.into()));
        let v = register(&ctx, "numeraire", 1000, 2500, 3000);
        assert_eq!(amend(&v, vec![env, sleeve(env, "CIRCLE_USDC", "cash"), sleeve(env, "CIRCLE_EURC", "numeraire")]), bad, "change");
        assert_eq!(amend(&v, vec![env, sleeve(env, "CIRCLE_USDC", "cash"), sleeve(env, "CIRCLE_EURC", "cash")]), bad, "remove");
        assert!(amend(&v, vec![env, sleeve(env, "CIRCLE_USDC", "numeraire"), sleeve(env, "CIRCLE_EURC", "cash")]).is_ok(), "keep");
        let plain = register(&ctx, "cash", 1000, 2500, 3000);
        assert_eq!(amend(&plain, vec![env, sleeve(env, "CIRCLE_USDC", "numeraire"), sleeve(env, "CIRCLE_EURC", "cash")]), bad, "add");
    }
}

// ---------------------------------------------------------------------------
// A1 (storage rent): per-vault records live in their own persistent entries
// (the shared instance entry would grow with every factory vault and is
// loaded on every call), every record touched in a transaction is renewed
// for 30 days, and so is the instance - so a vault in regular use never
// finds its policy archived.
// ---------------------------------------------------------------------------
mod a1_rent {
    use super::*;
    use soroban_sdk::testutils::storage::{Instance as _, Persistent as _};

    const DAY: u32 = 17_280;

    fn register(ctx: &TestContext, vault: &Address) {
        PolicyContractClient::new(&ctx.env, &ctx.policy_address).register_policy(vault, &ctx.owner, &sample_hash(&ctx.env, 0x51),
            &ctx.registry_address, &1000, &10_000, &10_000, &50, &500, &sample_sleeves(&ctx.env));
    }

    #[test]
    fn per_vault_records_are_persistent_entries_not_instance_data() {
        let ctx = setup_test_context();
        let vault = Address::generate(&ctx.env);
        register(&ctx, &vault);
        ctx.env.as_contract(&ctx.policy_address, || {
            for key in [DataKey::PolicyConfig(vault.clone()), DataKey::SleeveTargets(vault.clone())] {
                assert!(ctx.env.storage().persistent().has(&key), "{key:?} must be its own entry");
                assert!(!ctx.env.storage().instance().has(&key), "{key:?} must not grow the instance");
                assert!(ctx.env.storage().persistent().get_ttl(&key) >= 29 * DAY);
            }
        });
    }

    #[test]
    fn a_vault_in_regular_use_never_finds_its_policy_archived() {
        let ctx = setup_test_context();
        let env = &ctx.env;
        let vault = Address::generate(env);
        register(&ctx, &vault);
        let policy = PolicyContractClient::new(env, &ctx.policy_address);
        let holdings = vec![env, (Symbol::new(env, "CIRCLE_USDC"), 1i128)];
        // One check a month for a year. Without renewal the TTL runs out and
        // the entries are archived (on a real network every access then needs
        // a paid restore); the TTLs below prove each use renews them.
        for _ in 0..12 {
            env.ledger().with_mut(|l| { l.sequence_number += 30 * DAY; l.timestamp += 30 * 86_400; });
            let _ = policy.validate_holdings(&vault, &holdings);
        }
        env.as_contract(&ctx.policy_address, || {
            assert!(env.storage().instance().get_ttl() >= 29 * DAY);
            assert!(env.storage().persistent().get_ttl(&DataKey::PolicyConfig(vault.clone())) >= 29 * DAY);
        });
    }
}

// ---------------------------------------------------------------------------
// Decision D-L (resolves P5): no short-notice mode for anyone. A strictly
// risk-REDUCING amendment applies immediately (and cancels any pending one);
// anything else - looser, mixed, new assets, changed types or bands - waits
// the full notice. Owner auth is required either way.
// ---------------------------------------------------------------------------
mod dl {
    use super::*;

    fn st(env: &Env, id: &str, target: u32, lo: u32, hi: u32, kind: &str) -> SleeveTarget {
        SleeveTarget { asset_id: Symbol::new(env, id), target_weight_bps: target, min_band_bps: lo, max_band_bps: hi,
            sleeve_type: Symbol::new(env, kind) }
    }

    /// USDC numeraire 85 % [80,90], EURC 10 % [5,15], BENJI 5 % [0,10].
    fn base(env: &Env) -> Vec<SleeveTarget> {
        vec![env, st(env, "CIRCLE_USDC", 8500, 8000, 9000, "numeraire"), st(env, "CIRCLE_EURC", 1000, 500, 1500, "cash"),
            st(env, "FRANKLIN_BENJI", 500, 0, 1000, "usd_gov_fund")]
    }

    struct T { ctx: TestContext, vault: Address }

    fn setup() -> T {
        let ctx = setup_test_context();
        let vault = Address::generate(&ctx.env);
        PolicyContractClient::new(&ctx.env, &ctx.policy_address).register_policy(&vault, &ctx.owner, &sample_hash(&ctx.env, 0xd0),
            &ctx.registry_address, &1000, &2500, &3000, &50, &500, &base(&ctx.env));
        T { ctx, vault }
    }

    /// (applied immediately?, pending?) after amending with these terms.
    fn amend(t: &T, floor: u32, single: u32, issuer: u32, slip: u32, band: u32, sleeves: Vec<SleeveTarget>) -> (bool, bool) {
        let p = PolicyContractClient::new(&t.ctx.env, &t.ctx.policy_address);
        let before = p.get_policy_config(&t.vault).unwrap().version;
        p.update_policy(&t.vault, &sample_hash(&t.ctx.env, 0xd1), &floor, &single, &issuer, &slip, &band, &sleeves);
        (p.get_policy_config(&t.vault).unwrap().version > before, p.get_pending_policy_update(&t.vault).is_some())
    }

    #[test]
    fn tighter_limits_apply_at_once() {
        let t = setup();
        assert_eq!(amend(&t, 1500, 2000, 2500, 30, 500, base(&t.ctx.env)), (true, false));
        let c = PolicyContractClient::new(&t.ctx.env, &t.ctx.policy_address).get_policy_config(&t.vault).unwrap();
        assert_eq!((c.min_liquidity_sleeve_bps, c.max_single_asset_bps, c.max_issuer_bps, c.max_slippage_bps), (1500, 2000, 2500, 30));
    }

    #[test]
    fn looser_or_mixed_limits_wait_the_full_notice() {
        // Each case also tightens the issuer cap (3000 -> 2900), so the one
        // looser term is the ONLY reason it must wait.
        let e = setup();
        assert_eq!(amend(&e, 1000, 2600, 2900, 50, 500, base(&e.ctx.env)), (false, true), "looser single-asset cap");
        let m = setup();
        assert_eq!(amend(&m, 2000, 2500, 3100, 50, 500, base(&m.ctx.env)), (false, true), "tighter floor but looser issuer cap");
        let b = setup();
        assert_eq!(amend(&b, 1000, 2500, 2900, 50, 400, base(&b.ctx.env)), (false, true), "any band change waits");
        let f = setup();
        assert_eq!(amend(&f, 800, 2500, 2900, 50, 500, base(&f.ctx.env)), (false, true), "lower cash floor");
        let l = setup();
        assert_eq!(amend(&l, 1000, 2500, 2900, 60, 500, base(&l.ctx.env)), (false, true), "looser slippage");
    }

    #[test]
    fn de_risking_the_sleeves_applies_at_once() {
        let t = setup();
        let env = &t.ctx.env;
        // Drop BENJI, cut EURC; the vault currency takes the weight.
        let safer = vec![env, st(env, "CIRCLE_USDC", 9500, 9000, 10000, "numeraire"), st(env, "CIRCLE_EURC", 500, 0, 1000, "cash")];
        assert_eq!(amend(&t, 1000, 2500, 3000, 50, 500, safer.clone()), (true, false));
        assert_eq!(PolicyContractClient::new(env, &t.ctx.policy_address).get_sleeve_targets(&t.vault), safer);
    }

    #[test]
    fn adding_raising_or_retyping_a_risk_asset_waits() {
        type Make = fn(&Env) -> Vec<SleeveTarget>;
        let cases: [(&str, Make); 5] = [
            ("raise BENJI", |e: &Env| vec![e, st(e, "CIRCLE_USDC", 8000, 7500, 9000, "numeraire"), st(e, "CIRCLE_EURC", 1000, 500, 1500, "cash"), st(e, "FRANKLIN_BENJI", 1000, 0, 1500, "usd_gov_fund")]),
            ("new asset", |e: &Env| vec![e, st(e, "CIRCLE_USDC", 8500, 8000, 9000, "numeraire"), st(e, "CIRCLE_EURC", 1000, 500, 1500, "cash"), st(e, "FRANKLIN_BENJI", 0, 0, 0, "usd_gov_fund"), st(e, "SPIKO_EUSTBL", 500, 0, 1000, "eur_sovereign")]),
            ("retype EURC", |e: &Env| vec![e, st(e, "CIRCLE_USDC", 8500, 8000, 9000, "numeraire"), st(e, "CIRCLE_EURC", 1000, 500, 1500, "eur_sovereign"), st(e, "FRANKLIN_BENJI", 500, 0, 1000, "usd_gov_fund")]),
            ("less cash allowed", |e: &Env| vec![e, st(e, "CIRCLE_USDC", 8500, 8000, 8900, "numeraire"), st(e, "CIRCLE_EURC", 1000, 500, 1500, "cash"), st(e, "FRANKLIN_BENJI", 500, 0, 1000, "usd_gov_fund")]),
            ("raise a risk floor", |e: &Env| vec![e, st(e, "CIRCLE_USDC", 8500, 8000, 9000, "numeraire"), st(e, "CIRCLE_EURC", 1000, 500, 1500, "cash"), st(e, "FRANKLIN_BENJI", 500, 200, 1000, "usd_gov_fund")]),
        ];
        for (why, sleeves) in cases {
            let t = setup();
            let s = sleeves(&t.ctx.env);
            // With a genuine tightening alongside (issuer cap 2900): still waits.
            assert_eq!(amend(&t, 1000, 2500, 2900, 50, 500, s), (false, true), "{why}");
        }
    }

    #[test]
    fn an_immediate_tightening_cancels_a_pending_loosening() {
        let t = setup();
        assert_eq!(amend(&t, 1000, 2600, 3000, 50, 500, base(&t.ctx.env)), (false, true));
        assert_eq!(amend(&t, 1200, 2500, 3000, 50, 500, base(&t.ctx.env)), (true, false));
        let p = PolicyContractClient::new(&t.ctx.env, &t.ctx.policy_address);
        t.ctx.env.ledger().set_timestamp(t.ctx.env.ledger().timestamp() + PROTOCOL_MIN_NOTICE_SECONDS);
        assert!(p.try_activate_policy_update(&t.vault).is_err(), "the old loosening cannot land later");
        assert_eq!(p.get_policy_config(&t.vault).unwrap().max_single_asset_bps, 2500);
    }

    #[test]
    fn identical_terms_change_nothing_immediately_only_by_notice() {
        // Same limits and sleeves with a new hash is not "risk-reducing": it waits.
        let t = setup();
        assert_eq!(amend(&t, 1000, 2500, 3000, 50, 500, base(&t.ctx.env)), (false, true));
    }
}
