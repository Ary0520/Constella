#![cfg(test)]

extern crate std;

use super::*;
use soroban_sdk::{
    testutils::{Address as _, Events},
    vec, Address, Env, Symbol,
};

fn create_sample_entry(
    env: &Env,
    contract_address: &Address,
    asset_id_str: &str,
    sleeve: SleeveType,
    tier: u32,
    oracle_str: &str,
) -> AssetEntry {
    AssetEntry {
        contract_address: contract_address.clone(),
        asset_id: Symbol::new(env, asset_id_str),
        sleeve_type: sleeve,
        liquidity_tier: tier,
        permissionless: true,
        oracle_sources: vec![env, Symbol::new(env, oracle_str)],
        eligible_jurisdictions: vec![env],
        blocked_jurisdictions: vec![env, Symbol::new(env, "IRN"), Symbol::new(env, "PRK")],
        investor_classes_required: vec![env],
        is_active: true,
        added_at: 1731600000,
    }
}

#[test]
fn test_t21_initialization_and_admin_auth() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(AssetRegistry, ());
    let client = AssetRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    // Verify admin
    assert_eq!(client.get_admin(), admin);

    // Re-initialization must fail
    let duplicate_init = client.try_initialize(&admin);
    assert!(duplicate_init.is_err());

    // Non-admin caller attempting add_asset without mock auths must fail
    let dummy_addr = Address::generate(&env);
    let sample = create_sample_entry(&env, &dummy_addr, "USDC", SleeveType::Cash, 0, "PYTH_USDC");

    // Clear mocked auths to test authorization failure
    env.mock_auths(&[]);
    let unauth_result = client.try_add_asset(&sample);
    assert!(unauth_result.is_err());

    let unauth_deactivate = client.try_deactivate_asset(&Symbol::new(&env, "USDC"));
    assert!(unauth_deactivate.is_err());
}

#[test]
fn test_t22_register_permissionless_assets_phase1() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(AssetRegistry, ());
    let client = AssetRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    let usdc_addr = Address::generate(&env);
    let eurc_addr = Address::generate(&env);
    let blend_addr = Address::generate(&env);

    let usdc_entry = create_sample_entry(&env, &usdc_addr, "USDC", SleeveType::Cash, 0, "PYTH_USDC");
    let eurc_entry = create_sample_entry(&env, &eurc_addr, "EURC", SleeveType::Cash, 0, "PYTH_EURC");
    let blend_entry = create_sample_entry(
        &env,
        &blend_addr,
        "BLEND_USDC_SUPPLY",
        SleeveType::Lending,
        1,
        "BLEND_POOL_V1",
    );

    client.add_asset(&usdc_entry);
    client.add_asset(&eurc_entry);
    client.add_asset(&blend_entry);

    // Verify lookups by symbol
    let fetched_usdc = client.get_asset(&Symbol::new(&env, "USDC")).unwrap();
    assert_eq!(fetched_usdc.contract_address, usdc_addr);
    assert_eq!(fetched_usdc.sleeve_type, SleeveType::Cash);
    assert!(fetched_usdc.is_active);

    let fetched_eurc = client.get_asset(&Symbol::new(&env, "EURC")).unwrap();
    assert_eq!(fetched_eurc.contract_address, eurc_addr);
    assert_eq!(fetched_eurc.sleeve_type, SleeveType::Cash);

    let fetched_blend = client
        .get_asset(&Symbol::new(&env, "BLEND_USDC_SUPPLY"))
        .unwrap();
    assert_eq!(fetched_blend.contract_address, blend_addr);
    assert_eq!(fetched_blend.sleeve_type, SleeveType::Lending);
    assert_eq!(fetched_blend.liquidity_tier, 1);

    // Verify lookups by address
    assert_eq!(
        client.get_asset_by_address(&usdc_addr).unwrap().asset_id,
        Symbol::new(&env, "USDC")
    );
    assert_eq!(
        client.get_asset_by_address(&eurc_addr).unwrap().asset_id,
        Symbol::new(&env, "EURC")
    );
    assert_eq!(
        client.get_asset_by_address(&blend_addr).unwrap().asset_id,
        Symbol::new(&env, "BLEND_USDC_SUPPLY")
    );

    // Verify registration status
    assert!(client.is_registered(&usdc_addr));
    assert!(client.is_registered(&eurc_addr));
    assert!(client.is_registered(&blend_addr));

    // Verify list_assets
    let all_assets = client.list_assets();
    assert_eq!(all_assets.len(), 3);
}

#[test]
fn test_t23_deactivate_asset() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(AssetRegistry, ());
    let client = AssetRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    let usdc_addr = Address::generate(&env);
    let eurc_addr = Address::generate(&env);

    client.add_asset(&create_sample_entry(
        &env,
        &usdc_addr,
        "USDC",
        SleeveType::Cash,
        0,
        "PYTH_USDC",
    ));
    client.add_asset(&create_sample_entry(
        &env,
        &eurc_addr,
        "EURC",
        SleeveType::Cash,
        0,
        "PYTH_EURC",
    ));

    assert!(client.is_registered(&usdc_addr));
    assert!(client.is_registered(&eurc_addr));

    // Deactivate USDC
    client.deactivate_asset(&Symbol::new(&env, "USDC"));

    // Verify get_asset reflects is_active == false
    let usdc_entry = client.get_asset(&Symbol::new(&env, "USDC")).unwrap();
    assert!(!usdc_entry.is_active);

    // Verify get_asset_by_address reflects is_active == false
    let usdc_by_addr = client.get_asset_by_address(&usdc_addr).unwrap();
    assert!(!usdc_by_addr.is_active);

    // is_registered MUST return false for deactivated asset
    assert!(!client.is_registered(&usdc_addr));

    // Other active assets remain registered
    assert!(client.is_registered(&eurc_addr));
}

#[test]
fn test_t24_duplicate_and_unregistered_queries() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(AssetRegistry, ());
    let client = AssetRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    let usdc_addr = Address::generate(&env);
    let initial_entry = create_sample_entry(&env, &usdc_addr, "USDC", SleeveType::Cash, 0, "PYTH_USDC");
    client.add_asset(&initial_entry);

    assert_eq!(client.list_assets().len(), 1);

    // Duplicate addition updates/overwrites cleanly without duplicating list entry
    let mut updated_entry = initial_entry.clone();
    updated_entry.liquidity_tier = 1;
    client.add_asset(&updated_entry);

    assert_eq!(client.list_assets().len(), 1);
    assert_eq!(
        client.get_asset(&Symbol::new(&env, "USDC")).unwrap().liquidity_tier,
        1
    );

    // Unregistered queries
    let unregistered_addr = Address::generate(&env);
    assert_eq!(client.get_asset_by_address(&unregistered_addr), None);
    assert!(!client.is_registered(&unregistered_addr));

    let unregistered_id = Symbol::new(&env, "UNKNOWN_ASSET");
    assert_eq!(client.get_asset(&unregistered_id), None);

    // Deactivating an unregistered asset should fail
    let deactivate_err = client.try_deactivate_asset(&unregistered_id);
    assert!(deactivate_err.is_err());
}

#[test]
fn test_t25_event_emissions_verification() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(AssetRegistry, ());
    let client = AssetRegistryClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    let usdc_addr = Address::generate(&env);
    let usdc_entry = create_sample_entry(&env, &usdc_addr, "USDC", SleeveType::Cash, 0, "PYTH_USDC");

    // Add asset emits ASSET_ADDED
    client.add_asset(&usdc_entry);

    use soroban_sdk::TryFromVal;

    let events = env.events().all();
    assert!(!events.is_empty());

    let asset_added_sym = Symbol::new(&env, "ASSET_ADDED");
    let usdc_sym = Symbol::new(&env, "USDC");

    let mut found_added = false;
    for event in events.iter() {
        let (c_id, topics, data) = event;
        if c_id == contract_id && topics.len() >= 2 {
            let topic_0 = Symbol::try_from_val(&env, &topics.get(0).unwrap());
            let topic_1 = Symbol::try_from_val(&env, &topics.get(1).unwrap());
            if topic_0 == Ok(asset_added_sym.clone()) && topic_1 == Ok(usdc_sym.clone()) {
                let data_addr = Address::try_from_val(&env, &data);
                assert_eq!(data_addr, Ok(usdc_addr.clone()));
                found_added = true;
                break;
            }
        }
    }
    assert!(found_added, "ASSET_ADDED event not found");

    // Deactivate asset emits ASSET_DEACTIVATED
    client.deactivate_asset(&Symbol::new(&env, "USDC"));

    let events = env.events().all();
    let asset_deactivated_sym = Symbol::new(&env, "ASSET_DEACTIVATED");

    let mut found_deactivated = false;
    for event in events.iter() {
        let (c_id, topics, data) = event;
        if c_id == contract_id && topics.len() >= 2 {
            let topic_0 = Symbol::try_from_val(&env, &topics.get(0).unwrap());
            let topic_1 = Symbol::try_from_val(&env, &topics.get(1).unwrap());
            if topic_0 == Ok(asset_deactivated_sym.clone()) && topic_1 == Ok(usdc_sym.clone()) {
                let data_addr = Address::try_from_val(&env, &data);
                assert_eq!(data_addr, Ok(usdc_addr.clone()));
                found_deactivated = true;
                break;
            }
        }
    }
    assert!(found_deactivated, "ASSET_DEACTIVATED event not found");
}
