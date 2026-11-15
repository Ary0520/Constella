#![cfg(test)]

extern crate std;

use super::*;
use soroban_sdk::{
    testutils::Address as _,
    vec, Address, Env, Symbol,
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


/// Issuer per REGISTRY.md (USDC and EURC are both Circle).
fn issuer_of(asset_id: &str) -> &'static str {
    match asset_id {
        "USDC" | "EURC" => "CIRCLE",
        "BENJI" | "FRANKLIN_BENJI" => "FRANKLIN_TEMPLETON",
        _ if asset_id.starts_with("SPIKO") => "SPIKO",
        _ if asset_id.starts_with("BLEND") => "BLEND",
        _ => "TEST_ISSUER",
    }
}

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
        issuer: Symbol::new(env, issuer_of(asset_id_str)),
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

    // Audit M3: the admin is set by the constructor, atomically with deployment.
    let admin = Address::generate(&env);
    let contract_id = env.register(AssetRegistry, (admin.clone(),));
    let client = AssetRegistryClient::new(&env, &contract_id);
    assert_eq!(client.get_admin(), admin);

    // There is no `initialize` entrypoint left to front-run.
    let attacker = Address::generate(&env);
    let r = env.try_invoke_contract::<(), soroban_sdk::Error>(
        &contract_id,
        &Symbol::new(&env, "initialize"),
        soroban_sdk::vec![&env, soroban_sdk::IntoVal::into_val(&attacker, &env)],
    );
    assert!(!matches!(r, Ok(Ok(()))));
    assert_eq!(client.get_admin(), admin);

    let dummy_addr = Address::generate(&env);
    let sample = create_sample_entry(&env, &dummy_addr, "USDC", SleeveType::Cash, 0, "PYTH_USDC");

    // Non-admin caller attempting add_asset without mock auths must fail
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

    let admin = Address::generate(&env);
    let contract_id = env.register(AssetRegistry, (admin.clone(),));
    let client = AssetRegistryClient::new(&env, &contract_id);


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

    let admin = Address::generate(&env);
    let contract_id = env.register(AssetRegistry, (admin.clone(),));
    let client = AssetRegistryClient::new(&env, &contract_id);


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

    // Verify re-activation: admin calls add_asset with is_active = true
    let mut reactivated_usdc = usdc_entry.clone();
    reactivated_usdc.is_active = true;
    client.add_asset(&reactivated_usdc);

    // Verify USDC is active again
    let re_fetched = client.get_asset(&Symbol::new(&env, "USDC")).unwrap();
    assert!(re_fetched.is_active);
    assert!(client.is_registered(&usdc_addr));

    // Verify list_assets count is still 2 (not duplicated)
    assert_eq!(client.list_assets().len(), 2);
}

#[test]
fn test_t24_duplicate_and_unregistered_queries() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(AssetRegistry, (admin.clone(),));
    let client = AssetRegistryClient::new(&env, &contract_id);


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

    // Address collision rejection: Attempting to register another asset with an already registered address must fail
    let collision_entry = create_sample_entry(
        &env,
        &usdc_addr,
        "FAKE_USDC",
        SleeveType::Cash,
        0,
        "PYTH_FAKE",
    );
    let collision_err = client.try_add_asset(&collision_entry);
    assert_eq!(
        collision_err,
        Err(Ok(soroban_sdk::Error::from_contract_error(
            RegistryError::AddressAlreadyRegistered as u32
        )))
    );

    // Updating contract address for an existing asset:
    let new_usdc_addr = Address::generate(&env);
    let mut moved_entry = initial_entry.clone();
    moved_entry.contract_address = new_usdc_addr.clone();
    client.add_asset(&moved_entry);

    // Old address is no longer registered or queryable
    assert_eq!(client.get_asset_by_address(&usdc_addr), None);
    assert!(!client.is_registered(&usdc_addr));

    // New address is registered and maps to USDC
    assert_eq!(
        client.get_asset_by_address(&new_usdc_addr).unwrap().asset_id,
        Symbol::new(&env, "USDC")
    );
    assert!(client.is_registered(&new_usdc_addr));
    assert_eq!(client.list_assets().len(), 1);

    // Input validation: Invalid liquidity tier (> 3)
    let invalid_tier_entry = create_sample_entry(
        &env,
        &Address::generate(&env),
        "INVALID_TIER",
        SleeveType::Cash,
        4,
        "PYTH_INVALID",
    );
    let tier_err = client.try_add_asset(&invalid_tier_entry);
    assert_eq!(
        tier_err,
        Err(Ok(soroban_sdk::Error::from_contract_error(
            RegistryError::InvalidLiquidityTier as u32
        )))
    );

    // Input validation: Empty oracle sources
    let mut empty_oracle_entry = create_sample_entry(
        &env,
        &Address::generate(&env),
        "EMPTY_ORACLE",
        SleeveType::Cash,
        0,
        "PYTH_EMPTY",
    );
    empty_oracle_entry.oracle_sources = vec![&env];
    let empty_oracle_err = client.try_add_asset(&empty_oracle_entry);
    assert_eq!(
        empty_oracle_err,
        Err(Ok(soroban_sdk::Error::from_contract_error(
            RegistryError::EmptyOracleSources as u32
        )))
    );

    // Unregistered queries
    let unregistered_addr = Address::generate(&env);
    assert_eq!(client.get_asset_by_address(&unregistered_addr), None);
    assert!(!client.is_registered(&unregistered_addr));

    let unregistered_id = Symbol::new(&env, "UNKNOWN_ASSET");
    assert_eq!(client.get_asset(&unregistered_id), None);

    // Deactivating an unregistered asset should fail with AssetNotFound
    let deactivate_err = client.try_deactivate_asset(&unregistered_id);
    assert_eq!(
        deactivate_err,
        Err(Ok(soroban_sdk::Error::from_contract_error(
            RegistryError::AssetNotFound as u32
        )))
    );
}

#[test]
fn test_t25_event_emissions_verification() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(AssetRegistry, (admin.clone(),));
    let client = AssetRegistryClient::new(&env, &contract_id);


    let usdc_addr = Address::generate(&env);
    let usdc_entry = create_sample_entry(&env, &usdc_addr, "USDC", SleeveType::Cash, 0, "PYTH_USDC");

    // Add asset emits ASSET_ADDED
    client.add_asset(&usdc_entry);

    use soroban_sdk::TryFromVal;

    let events = all_events(&env);
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

    let events = all_events(&env);
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

// -------------------------------------------------------------------------
// Audit P2 (2026-11-15): every asset records its issuer; empty is refused.
// -------------------------------------------------------------------------
mod p2 {
    use super::*;

    #[test]
    fn issuer_is_stored_and_required() {
        let env = Env::default();
        env.mock_all_auths();
        let id = env.register(AssetRegistry, (Address::generate(&env),));
        let client = AssetRegistryClient::new(&env, &id);

        let usdc = Address::generate(&env);
        client.add_asset(&create_sample_entry(&env, &usdc, "USDC", SleeveType::Cash, 0, "PYTH_USDC"));
        assert_eq!(client.get_asset(&Symbol::new(&env, "USDC")).unwrap().issuer, Symbol::new(&env, "CIRCLE"));

        let mut nameless = create_sample_entry(&env, &Address::generate(&env), "MYSTERY", SleeveType::Cash, 0, "PYTH_X");
        nameless.issuer = Symbol::new(&env, "");
        assert_eq!(
            client.try_add_asset(&nameless),
            Err(Ok(soroban_sdk::Error::from_contract_error(RegistryError::EmptyIssuer as u32)))
        );
        assert!(client.get_asset(&Symbol::new(&env, "MYSTERY")).is_none());
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
fn golden_events_registry() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let id = env.register(AssetRegistry, (admin.clone(),));
    let client = AssetRegistryClient::new(&env, &id);
    let mut g = GoldenEvents::new(&id);
    let usdc = Address::generate(&env);
    client.add_asset(&create_sample_entry(&env, &usdc, "USDC", SleeveType::Cash, 0, "PYTH_USDC"));
    g.capture(&env, "add_asset");
    client.deactivate_asset(&Symbol::new(&env, "USDC"));
    g.capture(&env, "deactivate_asset");
    g.finish(&["ASSET_ADDED", "ASSET_DEACTIVATED"]);
}
