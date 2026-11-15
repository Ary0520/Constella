#![cfg(test)]

extern crate std;

use super::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env, Symbol, TryFromVal,
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


fn setup_oracle(env: &Env) -> (NavOracleClient<'_>, Address, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let base_asset = Address::generate(env);
    let contract_id = env.register(NavOracle, (admin.clone(), base_asset.clone(), 7u32, 300u32));
    let client = NavOracleClient::new(env, &contract_id);
    (client, admin, base_asset)
}

fn create_source(
    env: &Env,
    source_id: &str,
    asset: &Address,
    sleeve: &str,
    price: i128,
    depth: i128,
    timestamp: u64,
) -> OracleSource {
    OracleSource {
        source_id: Symbol::new(env, source_id),
        asset: asset.clone(),
        sleeve: Symbol::new(env, sleeve),
        price,
        timestamp,
        staleness_limit_seconds: 14400, // 4 hours
        min_liquidity_depth_usd: 100_000,
        current_liquidity_depth_usd: depth,
        is_active: true,
    }
}

#[test]
fn test_t33_dual_nav_and_pricing_invariant() {
    let env = Env::default();
    let (client, _admin, _base) = setup_oracle(&env);

    let sleeve = Symbol::new(&env, "USD_GOV_FUNDS");
    client.configure_sleeve(&sleeve, &300, &2); // 3% deviation threshold to allow 2% spread

    let benji_asset = Address::generate(&env);
    env.ledger().set_timestamp(1000);

    // 3 feeds: $1.00, $1.01, $0.99 (using 6 decimals: 1_000_000, 1_010_000, 990_000)
    let s1 = create_source(&env, "BENJI_NAV", &benji_asset, "USD_GOV_FUNDS", 1_000_000, 200_000, 1000);
    let s2 = create_source(&env, "REFLECTOR_BENJI", &benji_asset, "USD_GOV_FUNDS", 1_010_000, 150_000, 1000);
    let s3 = create_source(&env, "REDSTONE_BENJI", &benji_asset, "USD_GOV_FUNDS", 990_000, 180_000, 1000);

    client.register_source(&s1);
    client.register_source(&s2);
    client.register_source(&s3);

    let collateral_nav = client.get_collateral_nav(&sleeve).expect("collateral nav should exist");
    let deposit_nav = client.get_deposit_nav(&sleeve).expect("deposit nav should exist");

    // Collateral NAV must be MINIMUM: 990_000 ($0.99)
    assert_eq!(collateral_nav.price, 990_000);
    assert_eq!(collateral_nav.source, Symbol::new(&env, "REDSTONE_BENJI"));

    // Deposit NAV must be MEDIAN: 1_000_000 ($1.00)
    assert_eq!(deposit_nav.price, 1_000_000);
    assert_eq!(deposit_nav.source, Symbol::new(&env, "BENJI_NAV"));

    // Invariant: collateral_nav <= deposit_nav
    assert!(collateral_nav.price <= deposit_nav.price);
}

#[test]
fn test_t34_thin_liquidity_depth_exclusion() {
    let env = Env::default();
    let (client, _admin, _base) = setup_oracle(&env);

    let sleeve = Symbol::new(&env, "USD_GOV_FUNDS");
    client.configure_sleeve(&sleeve, &300, &2);

    let benji_asset = Address::generate(&env);
    env.ledger().set_timestamp(1000);

    // Feed 1: deep ($200k >= $100k min), price $1.00
    let s1 = create_source(&env, "BENJI_NAV", &benji_asset, "USD_GOV_FUNDS", 1_000_000, 200_000, 1000);
    // Feed 2: deep ($150k >= $100k min), price $1.01
    let s2 = create_source(&env, "REFLECTOR_BENJI", &benji_asset, "USD_GOV_FUNDS", 1_010_000, 150_000, 1000);
    // Feed 3: thin ($50k < $100k min), manipulated YieldBlox-style price $107.00
    let s3 = create_source(&env, "MANIPULATED_SDEX", &benji_asset, "USD_GOV_FUNDS", 107_000_000, 50_000, 1000);

    client.register_source(&s1);
    client.register_source(&s2);
    client.register_source(&s3);

    // The thin feed is completely ignored. NAV is computed only from s1 and s2.
    let collateral_nav = client.get_collateral_nav(&sleeve).expect("nav should succeed");
    assert_eq!(collateral_nav.price, 1_000_000);
    assert_eq!(collateral_nav.source, Symbol::new(&env, "BENJI_NAV"));

    // If s2 is also thin, leaving only 1 valid deep feed:
    let s2_thin = create_source(&env, "REFLECTOR_BENJI", &benji_asset, "USD_GOV_FUNDS", 1_010_000, 40_000, 1000);
    client.register_source(&s2_thin);

    // Only 1 valid deep feed remains (< min_valid_sources 2), hard stop triggers!
    assert!(client.get_collateral_nav(&sleeve).is_none());
}

#[test]
fn test_t35_staleness_rejection() {
    let env = Env::default();
    let (client, _admin, _base) = setup_oracle(&env);

    let sleeve = Symbol::new(&env, "USD_GOV_FUNDS");
    client.configure_sleeve(&sleeve, &300, &2);

    let benji_asset = Address::generate(&env);
    // Current ledger timestamp is 20_000
    env.ledger().set_timestamp(20_000);

    // Feed 1: timestamp 19_000 (delta = 1000 <= 14400, valid)
    let s1 = create_source(&env, "BENJI_NAV", &benji_asset, "USD_GOV_FUNDS", 1_000_000, 200_000, 19_000);
    // Feed 2: timestamp 18_000 (delta = 2000 <= 14400, valid)
    let s2 = create_source(&env, "REFLECTOR_BENJI", &benji_asset, "USD_GOV_FUNDS", 1_010_000, 200_000, 18_000);
    // Feed 3: timestamp 1_000 (delta = 19000 > 14400, STALE)
    let s3 = create_source(&env, "REDSTONE_BENJI", &benji_asset, "USD_GOV_FUNDS", 980_000, 200_000, 1_000);

    client.register_source(&s1);
    client.register_source(&s2);
    client.register_source(&s3);

    // Stale feed (s3 with 980_000) is rejected. Min of fresh feeds is s1 (1_000_000).
    let collateral_nav = client.get_collateral_nav(&sleeve).expect("nav should succeed");
    assert_eq!(collateral_nav.price, 1_000_000);
    assert_eq!(collateral_nav.source, Symbol::new(&env, "BENJI_NAV"));

    // Advance time to 40_000, so all feeds become stale
    env.ledger().set_timestamp(40_000);
    assert!(client.get_collateral_nav(&sleeve).is_none());
}

#[test]
fn test_t36_hard_stop_on_insufficient_sources() {
    let env = Env::default();
    let (client, _admin, _base) = setup_oracle(&env);

    let sleeve = Symbol::new(&env, "USD_GOV_FUNDS");
    client.configure_sleeve(&sleeve, &200, &2); // requires >= 2 valid sources

    let benji_asset = Address::generate(&env);
    env.ledger().set_timestamp(1000);

    // Only 1 source registered
    let s1 = create_source(&env, "BENJI_NAV", &benji_asset, "USD_GOV_FUNDS", 1_000_000, 200_000, 1000);
    client.register_source(&s1);

    // Rule 5 hard stop: < 2 valid sources returns None
    assert!(client.get_collateral_nav(&sleeve).is_none());
    assert!(client.get_deposit_nav(&sleeve).is_none());

    // Register second source with is_active = false
    let mut s2 = create_source(&env, "REFLECTOR_BENJI", &benji_asset, "USD_GOV_FUNDS", 1_010_000, 200_000, 1000);
    s2.is_active = false;
    client.register_source(&s2);

    // Still only 1 active valid source -> returns None
    assert!(client.get_collateral_nav(&sleeve).is_none());

    // Activate second source
    s2.is_active = true;
    client.register_source(&s2);

    // Now 2 valid sources -> returns valid NAV
    assert!(client.get_collateral_nav(&sleeve).is_some());
}

#[test]
fn test_t37_deviation_circuit_breaker() {
    let env = Env::default();
    let (client, _admin, _base) = setup_oracle(&env);

    let sleeve = Symbol::new(&env, "USD_GOV_FUNDS");
    // Deviation threshold: 200 bps (2.0%)
    client.configure_sleeve(&sleeve, &200, &2);

    let benji_asset = Address::generate(&env);
    env.ledger().set_timestamp(1000);

    // 2 feeds: $1.00 and $1.025 (divergence is 2.5% = 250 bps > 200 bps threshold)
    let s1 = create_source(&env, "BENJI_NAV", &benji_asset, "USD_GOV_FUNDS", 1_000_000, 200_000, 1000);
    let s2 = create_source(&env, "REFLECTOR_BENJI", &benji_asset, "USD_GOV_FUNDS", 1_025_000, 200_000, 1000);

    client.register_source(&s1);
    client.register_source(&s2);
    // Audit O4: deviation latches and NAV_PAUSE is emitted in the call that
    // adds the deviating feed (persisted even if a later caller reverts).
    let events = all_events(&env);

    // Query triggers circuit breaker and returns None
    assert!(client.get_collateral_nav(&sleeve).is_none());

    let nav_pause_sym = Symbol::new(&env, "NAV_PAUSE");
    let mut found_pause_event = false;
    for event in events.iter() {
        let (_cid, topics, data) = event;
        if topics.len() >= 2 {
            let t0 = Symbol::try_from_val(&env, &topics.get(0).unwrap());
            let t1 = Symbol::try_from_val(&env, &topics.get(1).unwrap());
            if t0 == Ok(nav_pause_sym.clone()) && t1 == Ok(sleeve.clone()) {
                let pause_data = NavPauseData::try_from_val(&env, &data).unwrap();
                assert_eq!(pause_data.reason, Symbol::new(&env, "DEVIATION"));
                assert_eq!(pause_data.deviation_bps, 250);
                found_pause_event = true;
                break;
            }
        }
    }
    assert!(found_pause_event, "NAV_PAUSE event should be emitted");

    // Verify sleeve is paused with reason DEVIATION
    assert!(client.is_sleeve_paused(&sleeve));
    let cfg = client.get_sleeve_config(&sleeve).unwrap();
    assert!(cfg.is_paused);
    assert_eq!(cfg.pause_reason, Symbol::new(&env, "DEVIATION"));

    // Subsequent calls while paused also return None
    assert!(client.get_collateral_nav(&sleeve).is_none());
    assert!(client.get_deposit_nav(&sleeve).is_none());
}

#[test]
fn test_t38_per_sleeve_isolation() {
    let env = Env::default();
    let (client, admin, _base) = setup_oracle(&env);

    let sleeve_gov = Symbol::new(&env, "USD_GOV_FUNDS");
    let sleeve_cash = Symbol::new(&env, "CASH_STABLE");

    client.configure_sleeve(&sleeve_gov, &200, &2);
    client.configure_sleeve(&sleeve_cash, &200, &2);

    let benji_asset = Address::generate(&env);
    let usdc_asset = Address::generate(&env);
    env.ledger().set_timestamp(1000);

    // USD_GOV_FUNDS feeds
    let g1 = create_source(&env, "BENJI_NAV", &benji_asset, "USD_GOV_FUNDS", 1_000_000, 200_000, 1000);
    let g2 = create_source(&env, "REFLECTOR_BENJI", &benji_asset, "USD_GOV_FUNDS", 1_005_000, 200_000, 1000);
    client.register_source(&g1);
    client.register_source(&g2);

    // CASH_STABLE feeds
    let c1 = create_source(&env, "REFLECTOR_USDC", &usdc_asset, "CASH_STABLE", 1_000_000, 500_000, 1000);
    let c2 = create_source(&env, "REDSTONE_USDC", &usdc_asset, "CASH_STABLE", 1_000_100, 500_000, 1000);
    client.register_source(&c1);
    client.register_source(&c2);

    // Trigger pause exclusively on USD_GOV_FUNDS
    client.trigger_circuit_breaker(&admin, &sleeve_gov, &Symbol::new(&env, "EMERGENCY_HALT"));

    assert!(client.is_sleeve_paused(&sleeve_gov));
    assert!(!client.is_sleeve_paused(&sleeve_cash));

    // USD_GOV_FUNDS is paused and returns None
    assert!(client.get_collateral_nav(&sleeve_gov).is_none());

    // CASH_STABLE continues operating normally without interference
    let cash_nav = client.get_collateral_nav(&sleeve_cash).expect("cash stable should operate");
    assert_eq!(cash_nav.price, 1_000_000);
    assert_eq!(cash_nav.source, Symbol::new(&env, "REFLECTOR_USDC"));

    // Clear circuit breaker on USD_GOV_FUNDS
    client.clear_circuit_breaker(&sleeve_gov);

    // Verify NAV_RESUME event IMMEDIATELY after clear_circuit_breaker
    let events = all_events(&env);
    let resume_sym = Symbol::new(&env, "NAV_RESUME");
    let mut found_resume = false;
    for event in events.iter() {
        let (_cid, topics, data) = event;
        if topics.len() >= 2 {
            let t0 = Symbol::try_from_val(&env, &topics.get(0).unwrap());
            let t1 = Symbol::try_from_val(&env, &topics.get(1).unwrap());
            if t0 == Ok(resume_sym.clone()) && t1 == Ok(sleeve_gov.clone()) {
                let resume_data = NavResumeData::try_from_val(&env, &data).unwrap();
                assert_eq!(resume_data.sleeve, sleeve_gov);
                assert_eq!(resume_data.sources_valid, 2);
                found_resume = true;
                break;
            }
        }
    }
    assert!(found_resume, "NAV_RESUME event should be emitted");

    assert!(!client.is_sleeve_paused(&sleeve_gov));

    // USD_GOV_FUNDS resumes functioning
    let gov_nav = client.get_collateral_nav(&sleeve_gov).expect("gov funds should resume");
    assert_eq!(gov_nav.price, 1_000_000);
}

#[test]
fn test_t39_sep40_interface_conformance() {
    let env = Env::default();
    let (client, _admin, base_asset) = setup_oracle(&env);

    // Verify metadata functions
    assert_eq!(client.base(), base_asset);
    assert_eq!(client.decimals(), 7);
    assert_eq!(client.resolution(), 300);

    let benji_asset = Address::generate(&env);
    env.ledger().set_timestamp(1000);

    let sleeve = Symbol::new(&env, "USD_GOV_FUNDS");
    client.configure_sleeve(&sleeve, &300, &2);

    let s1 = create_source(&env, "BENJI_NAV", &benji_asset, "USD_GOV_FUNDS", 1_000_000, 200_000, 1000);
    let s2 = create_source(&env, "REFLECTOR_BENJI", &benji_asset, "USD_GOV_FUNDS", 1_010_000, 200_000, 1000);
    client.register_source(&s1);
    client.register_source(&s2);

    // assets() returns registered assets
    let assets = client.assets();
    assert_eq!(assets.len(), 1);
    assert_eq!(assets.get(0).unwrap(), benji_asset);

    // price() returns conservative NAV (minimum of valid sources)
    let p = client.price(&benji_asset).expect("price should return Some");
    assert_eq!(p.price, 1_000_000);
    assert_eq!(p.source, Symbol::new(&env, "BENJI_NAV"));

    // lastprice() returns valid price
    let lp = client.lastprice(&benji_asset).expect("lastprice should return Some");
    assert_eq!(lp.price, 1_000_000);

    // prices() SEP-40 batch query returns vector of option prices
    let batch_prices = client.prices(&assets, &1).expect("prices batch should return Some");
    assert_eq!(batch_prices.len(), 1);
    assert_eq!(batch_prices.get(0).unwrap().unwrap().price, 1_000_000);

    // prices() with 0 records returns empty vector
    let batch_empty = client.prices(&assets, &0).expect("prices 0 records returns empty");
    assert_eq!(batch_empty.len(), 0);

    // When time advances past staleness limit:
    env.ledger().set_timestamp(50_000);

    // Fresh price returns None due to staleness
    assert!(client.price(&benji_asset).is_none());

    // Batch prices also returns None for stale asset
    let batch_prices_stale = client.prices(&assets, &1).expect("prices batch returns Some");
    assert_eq!(batch_prices_stale.len(), 1);
    assert!(batch_prices_stale.get(0).unwrap().is_none());

    // lastprice still returns the last recorded price (SEP-40 requirement)
    let lp_stale = client.lastprice(&benji_asset).expect("lastprice returns historical");
    assert_eq!(lp_stale.price, 1_000_000);
}

#[test]
fn test_update_price_and_events() {
    let env = Env::default();
    let (client, _admin, _base) = setup_oracle(&env);

    let sleeve = Symbol::new(&env, "USD_GOV_FUNDS");
    client.configure_sleeve(&sleeve, &300, &2);

    let benji_asset = Address::generate(&env);
    env.ledger().set_timestamp(1000);

    let s1 = create_source(&env, "BENJI_NAV", &benji_asset, "USD_GOV_FUNDS", 1_000_000, 200_000, 1000);
    client.register_source(&s1);

    // Update price once ledger time has reached the new observation time
    // (audit O2: future-dated prices are rejected).
    env.ledger().set_timestamp(1500);
    client.update_price(&Symbol::new(&env, "BENJI_NAV"), &1_050_000, &250_000, &1500);

    // Verify PRICE_UPDATED event IMMEDIATELY after update_price
    let events = all_events(&env);
    let price_updated_sym = Symbol::new(&env, "PRICE_UPDATED");
    let mut found_update_event = false;
    for event in events.iter() {
        let (_cid, topics, data) = event;
        if topics.len() >= 2 {
            let t0 = Symbol::try_from_val(&env, &topics.get(0).unwrap());
            let t1 = Symbol::try_from_val(&env, &topics.get(1).unwrap());
            if t0 == Ok(price_updated_sym.clone()) && t1 == Ok(Symbol::new(&env, "BENJI_NAV")) {
                let update_data = PriceUpdatedData::try_from_val(&env, &data).unwrap();
                assert_eq!(update_data.price, 1_050_000);
                assert_eq!(update_data.liquidity_depth_usd, 250_000);
                assert_eq!(update_data.timestamp, 1500);
                found_update_event = true;
                break;
            }
        }
    }
    assert!(found_update_event, "PRICE_UPDATED event should be emitted");

    let updated_source = client.get_source(&Symbol::new(&env, "BENJI_NAV")).unwrap();
    assert_eq!(updated_source.price, 1_050_000);
    assert_eq!(updated_source.current_liquidity_depth_usd, 250_000);
    assert_eq!(updated_source.timestamp, 1500);
}

#[test]
fn test_uninitialized_and_authorization_checks() {
    let env = Env::default();
    // Audit M3: configuration happens in the constructor, atomically with
    // deployment; there is no uninitialized window and no `initialize`.
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let base = Address::generate(&env);
    let contract_id = env.register(NavOracle, (admin.clone(), base.clone(), 7u32, 300u32));
    let client = NavOracleClient::new(&env, &contract_id);
    assert_eq!(client.get_admin(), admin);
    assert_eq!(client.base(), base);
    let r = env.try_invoke_contract::<(), soroban_sdk::Error>(
        &contract_id,
        &Symbol::new(&env, "initialize"),
        soroban_sdk::vec![
            &env,
            soroban_sdk::IntoVal::into_val(&Address::generate(&env), &env),
            soroban_sdk::IntoVal::into_val(&base, &env),
            soroban_sdk::IntoVal::into_val(&7u32, &env),
            soroban_sdk::IntoVal::into_val(&300u32, &env),
        ],
    );
    assert!(!matches!(r, Ok(Ok(()))));
    assert_eq!(client.get_admin(), admin);

    // Trigger circuit breaker on non-existent sleeve fails
    assert_eq!(
        client.try_trigger_circuit_breaker(&admin, &Symbol::new(&env, "NON_EXISTENT"), &Symbol::new(&env, "TEST")),
        Err(Ok(soroban_sdk::Error::from_contract_error(
            OracleError::SleeveNotFound as u32
        )))
    );

    // Invalid threshold fails
    assert_eq!(
        client.try_configure_sleeve(&Symbol::new(&env, "SLEEVE"), &0, &2),
        Err(Ok(soroban_sdk::Error::from_contract_error(
            OracleError::InvalidThreshold as u32
        )))
    );
    assert_eq!(
        client.try_configure_sleeve(&Symbol::new(&env, "SLEEVE"), &15000, &2),
        Err(Ok(soroban_sdk::Error::from_contract_error(
            OracleError::InvalidThreshold as u32
        )))
    );
    assert_eq!(
        client.try_configure_sleeve(&Symbol::new(&env, "SLEEVE"), &200, &0),
        Err(Ok(soroban_sdk::Error::from_contract_error(
            OracleError::InvalidThreshold as u32
        )))
    );

    // Clear mocked auths -> unauthorized calls fail
    env.mock_auths(&[]);
    let unauth_config = client.try_configure_sleeve(&Symbol::new(&env, "SLEEVE"), &200, &2);
    assert!(unauth_config.is_err());

    let dummy_source = create_source(&env, "S1", &base, "SLEEVE", 100, 100_000, 100);
    let unauth_reg = client.try_register_source(&dummy_source);
    assert!(unauth_reg.is_err());

    let unauth_update = client.try_update_price(&Symbol::new(&env, "S1"), &100, &100_000, &100);
    assert!(unauth_update.is_err());

    let unauth_clear = client.try_clear_circuit_breaker(&Symbol::new(&env, "SLEEVE"));
    assert!(unauth_clear.is_err());
}

#[test]
fn test_thin_feed_cannot_poison_lastprice() {
    let env = Env::default();
    let (client, _admin, _base) = setup_oracle(&env);

    let sleeve = Symbol::new(&env, "USD_GOV_FUNDS");
    client.configure_sleeve(&sleeve, &300, &2);

    let benji_asset = Address::generate(&env);
    env.ledger().set_timestamp(1000);

    // Two deep valid feeds ($1.00 and $1.01)
    let s1 = create_source(&env, "BENJI_NAV", &benji_asset, "USD_GOV_FUNDS", 1_000_000, 200_000, 1000);
    let s2 = create_source(&env, "REFLECTOR_BENJI", &benji_asset, "USD_GOV_FUNDS", 1_010_000, 200_000, 1000);
    client.register_source(&s1);
    client.register_source(&s2);

    // Query price() -> establishes valid conservative NAV of $1.00 and records LastPrice
    let p = client.price(&benji_asset).expect("valid price");
    assert_eq!(p.price, 1_000_000);
    assert_eq!(client.lastprice(&benji_asset).unwrap().price, 1_000_000);

    // YieldBlox exploit attempt: A thin market feed ($107.00, but only $50k depth < $100k min)
    let s3_thin = create_source(&env, "MANIPULATED_SDEX", &benji_asset, "USD_GOV_FUNDS", 107_000_000, 50_000, 1000);
    client.register_source(&s3_thin);

    // Also update a thin source via update_price
    client.update_price(&Symbol::new(&env, "MANIPULATED_SDEX"), &108_000_000, &40_000, &1000);

    // CRITICAL SECURITY ASSERTION: lastprice MUST NOT be poisoned by thin feeds!
    let lp = client.lastprice(&benji_asset).expect("lastprice returns valid cached price");
    assert_eq!(lp.price, 1_000_000, "Thin feed must not poison lastprice");
    assert_eq!(lp.source, Symbol::new(&env, "BENJI_NAV"));
}

#[test]
fn test_extreme_price_difference_no_overflow() {
    let env = Env::default();
    let (client, _admin, _base) = setup_oracle(&env);

    let sleeve = Symbol::new(&env, "USD_GOV_FUNDS");
    client.configure_sleeve(&sleeve, &300, &2);

    let benji_asset = Address::generate(&env);
    env.ledger().set_timestamp(1000);

    // Feed 1: normal price ($1.00)
    let s1 = create_source(&env, "FEED_1", &benji_asset, "USD_GOV_FUNDS", 1_000_000, 200_000, 1000);
    // Feed 2: astronomical price approaching i128::MAX / 2 (diff * 10_000 would overflow i128)
    let extreme_price = i128::MAX / 2;
    let s2 = create_source(&env, "FEED_EXTREME", &benji_asset, "USD_GOV_FUNDS", extreme_price, 200_000, 1000);

    client.register_source(&s1);
    client.register_source(&s2);
    // Audit O4: deviation latches and NAV_PAUSE is emitted in the call that
    // adds the deviating feed (persisted even if a later caller reverts).
    let events = all_events(&env);

    // Calling get_collateral_nav must trip deviation circuit breaker WITHOUT PANICKING from integer overflow
    assert!(client.get_collateral_nav(&sleeve).is_none());

    let nav_pause_sym = Symbol::new(&env, "NAV_PAUSE");
    let mut found_pause = false;
    for event in events.iter() {
        let (_cid, topics, data) = event;
        if topics.len() >= 2 {
            let t0 = Symbol::try_from_val(&env, &topics.get(0).unwrap());
            let t1 = Symbol::try_from_val(&env, &topics.get(1).unwrap());
            if t0 == Ok(nav_pause_sym.clone()) && t1 == Ok(sleeve.clone()) {
                let pause_data = NavPauseData::try_from_val(&env, &data).unwrap();
                assert_eq!(pause_data.reason, Symbol::new(&env, "DEVIATION"));
                assert_eq!(pause_data.deviation_bps, u32::MAX);
                found_pause = true;
                break;
            }
        }
    }
    assert!(found_pause, "NAV_PAUSE event must be emitted on extreme price deviation");

    // Verify sleeve is paused
    assert!(client.is_sleeve_paused(&sleeve));
}

#[test]
fn test_median_price_no_overflow() {
    let env = Env::default();
    let (client, _admin, _base) = setup_oracle(&env);

    let sleeve = Symbol::new(&env, "LARGE_NUMS");
    // Threshold 10,000 bps (100%) to allow small relative spread
    client.configure_sleeve(&sleeve, &10_000, &2);

    let test_asset = Address::generate(&env);
    env.ledger().set_timestamp(1000);

    // Two huge prices whose sum would overflow i128:
    // (i128::MAX - 20) + (i128::MAX - 10) > i128::MAX
    let p1 = i128::MAX - 20;
    let p2 = i128::MAX - 10;
    let s1 = create_source(&env, "FEED_A", &test_asset, "LARGE_NUMS", p1, 200_000, 1000);
    let s2 = create_source(&env, "FEED_B", &test_asset, "LARGE_NUMS", p2, 200_000, 1000);
    client.register_source(&s1);
    client.register_source(&s2);

    // get_deposit_nav must compute midpoint without overflow panic
    let deposit_nav = client.get_deposit_nav(&sleeve).expect("deposit nav computes safely");
    assert_eq!(deposit_nav.price, i128::MAX - 15);
}

#[test]
fn test_property_collateral_nav_le_deposit_nav() {
    let env = Env::default();
    let (client, _admin, _base) = setup_oracle(&env);

    let sleeve = Symbol::new(&env, "PROP_TEST");
    client.configure_sleeve(&sleeve, &500, &2); // 5% tolerance

    let asset = Address::generate(&env);
    env.ledger().set_timestamp(1000);

    // Test with 2 sources
    let s1 = create_source(&env, "P1", &asset, "PROP_TEST", 1_000_000, 200_000, 1000);
    let s2 = create_source(&env, "P2", &asset, "PROP_TEST", 1_020_000, 200_000, 1000);
    client.register_source(&s1);
    client.register_source(&s2);
    let c = client.get_collateral_nav(&sleeve).unwrap();
    let d = client.get_deposit_nav(&sleeve).unwrap();
    assert!(c.price <= d.price);

    // Add 3rd source (odd count = 3)
    let s3 = create_source(&env, "P3", &asset, "PROP_TEST", 1_010_000, 200_000, 1000);
    client.register_source(&s3);
    let c = client.get_collateral_nav(&sleeve).unwrap();
    let d = client.get_deposit_nav(&sleeve).unwrap();
    assert!(c.price <= d.price);

    // Add 4th source (even count = 4)
    let s4 = create_source(&env, "P4", &asset, "PROP_TEST", 995_000, 200_000, 1000);
    client.register_source(&s4);
    let c = client.get_collateral_nav(&sleeve).unwrap();
    let d = client.get_deposit_nav(&sleeve).unwrap();
    assert!(c.price <= d.price);

    // Add 5th source (odd count = 5)
    let s5 = create_source(&env, "P5", &asset, "PROP_TEST", 1_005_000, 200_000, 1000);
    client.register_source(&s5);
    let c = client.get_collateral_nav(&sleeve).unwrap();
    let d = client.get_deposit_nav(&sleeve).unwrap();
    assert!(c.price <= d.price);
}

// -------------------------------------------------------------------------
// Audit O1 (2026-11-15): only the pause authority or admin may pause a
// sleeve; only the admin may clear it or rotate the pause authority.
// Every call under test uses an explicit auth tree (AGENT-RULES Rule 13).
// -------------------------------------------------------------------------
mod o1 {
    use super::*;
    use soroban_sdk::testutils::{MockAuth, MockAuthInvoke};
    use soroban_sdk::IntoVal;

    fn setup_with_sleeve(env: &Env) -> (NavOracleClient<'_>, Address, Symbol) {
        let (client, admin, _base) = setup_oracle(env);
        let sleeve = Symbol::new(env, "USD_GOV_FUNDS");
        client.configure_sleeve(&sleeve, &200, &2);
        (client, admin, sleeve)
    }

    fn try_trip(env: &Env, client: &NavOracleClient, caller: &Address, sleeve: &Symbol) -> bool {
        let reason = Symbol::new(env, "MANUAL");
        env.mock_auths(&[MockAuth {
            address: caller,
            invoke: &MockAuthInvoke {
                contract: &client.address,
                fn_name: "trigger_circuit_breaker",
                args: (caller.clone(), sleeve.clone(), reason.clone()).into_val(env),
                sub_invokes: &[],
            },
        }]);
        client.try_trigger_circuit_breaker(caller, sleeve, &reason).is_ok()
    }

    #[test]
    fn unsigned_call_is_rejected() {
        let env = Env::default();
        let (client, admin, sleeve) = setup_with_sleeve(&env);
        env.set_auths(&[]);
        // Even naming the admin as caller fails without the admin's signature.
        assert!(client.try_trigger_circuit_breaker(&admin, &sleeve, &Symbol::new(&env, "X")).is_err());
        assert!(!client.is_sleeve_paused(&sleeve));
    }

    #[test]
    fn signed_stranger_is_rejected_with_unauthorized() {
        let env = Env::default();
        let (client, _admin, sleeve) = setup_with_sleeve(&env);
        let stranger = Address::generate(&env);
        assert!(!try_trip(&env, &client, &stranger, &sleeve));
        let reason = Symbol::new(&env, "MANUAL");
        env.mock_auths(&[MockAuth {
            address: &stranger,
            invoke: &MockAuthInvoke {
                contract: &client.address,
                fn_name: "trigger_circuit_breaker",
                args: (stranger.clone(), sleeve.clone(), reason.clone()).into_val(&env),
                sub_invokes: &[],
            },
        }]);
        assert_eq!(
            client.try_trigger_circuit_breaker(&stranger, &sleeve, &reason),
            Err(Ok(soroban_sdk::Error::from_contract_error(OracleError::Unauthorized as u32)))
        );
        assert!(!client.is_sleeve_paused(&sleeve));
    }

    #[test]
    fn admin_and_pause_authority_can_pause() {
        let env = Env::default();
        let (client, admin, sleeve) = setup_with_sleeve(&env);
        assert_eq!(client.get_pause_authority(), admin);
        assert!(try_trip(&env, &client, &admin, &sleeve));
        assert_eq!(env.auths()[0].0, admin);
        assert!(client.is_sleeve_paused(&sleeve));

        env.mock_all_auths();
        client.clear_circuit_breaker(&sleeve);
        let guardian = Address::generate(&env);
        client.set_pause_authority(&guardian);
        assert!(try_trip(&env, &client, &guardian, &sleeve));
        assert_eq!(env.auths()[0].0, guardian);
        assert!(client.is_sleeve_paused(&sleeve));
    }

    #[test]
    fn rotated_out_authority_loses_power() {
        let env = Env::default();
        let (client, _admin, sleeve) = setup_with_sleeve(&env);
        let g1 = Address::generate(&env);
        let g2 = Address::generate(&env);
        env.mock_all_auths();
        client.set_pause_authority(&g1);
        client.set_pause_authority(&g2);
        assert!(!try_trip(&env, &client, &g1, &sleeve));
        assert!(try_trip(&env, &client, &g2, &sleeve));
    }

    #[test]
    fn only_admin_can_rotate_authority_or_clear() {
        let env = Env::default();
        let (client, admin, sleeve) = setup_with_sleeve(&env);
        let guardian = Address::generate(&env);
        env.mock_all_auths();
        client.set_pause_authority(&guardian);
        assert!(try_trip(&env, &client, &guardian, &sleeve));

        // The pause authority can neither clear a pause nor rotate itself.
        env.mock_auths(&[MockAuth {
            address: &guardian,
            invoke: &MockAuthInvoke {
                contract: &client.address,
                fn_name: "clear_circuit_breaker",
                args: (sleeve.clone(),).into_val(&env),
                sub_invokes: &[],
            },
        }]);
        assert!(client.try_clear_circuit_breaker(&sleeve).is_err());
        let other = Address::generate(&env);
        env.mock_auths(&[MockAuth {
            address: &guardian,
            invoke: &MockAuthInvoke {
                contract: &client.address,
                fn_name: "set_pause_authority",
                args: (other.clone(),).into_val(&env),
                sub_invokes: &[],
            },
        }]);
        assert!(client.try_set_pause_authority(&other).is_err());
        assert!(client.is_sleeve_paused(&sleeve));
        assert_eq!(client.get_pause_authority(), guardian);

        env.mock_auths(&[MockAuth {
            address: &admin,
            invoke: &MockAuthInvoke {
                contract: &client.address,
                fn_name: "clear_circuit_breaker",
                args: (sleeve.clone(),).into_val(&env),
                sub_invokes: &[],
            },
        }]);
        client.clear_circuit_breaker(&sleeve);
        assert!(!client.is_sleeve_paused(&sleeve));
    }
}

// -------------------------------------------------------------------------
// Audit O4 (2026-11-15): deviation must latch (and NAV_PAUSE must be emitted)
// in the successful call that changes prices, not only on reads that may be
// rolled back by a reverting caller.
// -------------------------------------------------------------------------
mod o4 {
    use super::*;

    fn deviation_event_bps(env: &Env, sleeve: &Symbol) -> Option<u32> {
        let nav_pause = Symbol::new(env, "NAV_PAUSE");
        for (_cid, topics, data) in all_events(env).iter() {
            if topics.len() >= 2
                && Symbol::try_from_val(env, &topics.get(0).unwrap()) == Ok(nav_pause.clone())
                && Symbol::try_from_val(env, &topics.get(1).unwrap()) == Ok(sleeve.clone())
            {
                let d = NavPauseData::try_from_val(env, &data).unwrap();
                assert_eq!(d.reason, Symbol::new(env, "DEVIATION"));
                return Some(d.deviation_bps);
            }
        }
        None
    }

    fn two_feed_sleeve(env: &Env) -> (NavOracleClient<'_>, Address, Symbol) {
        let (client, admin, _base) = setup_oracle(env);
        let sleeve = Symbol::new(env, "USD_GOV_FUNDS");
        client.configure_sleeve(&sleeve, &200, &2);
        env.ledger().set_timestamp(1000);
        let asset = Address::generate(env);
        client.register_source(&create_source(env, "NAV", &asset, "USD_GOV_FUNDS", 1_000_000, 200_000, 1000));
        client.register_source(&create_source(env, "RFL", &asset, "USD_GOV_FUNDS", 1_000_000, 200_000, 1000));
        (client, admin, sleeve)
    }

    #[test]
    fn deviating_price_update_latches_pause_and_emits_event() {
        let env = Env::default();
        let (client, _admin, sleeve) = two_feed_sleeve(&env);
        client.update_price(&Symbol::new(&env, "RFL"), &1_030_000, &200_000, &1000);
        assert_eq!(deviation_event_bps(&env, &sleeve), Some(300));
        assert!(client.is_sleeve_paused(&sleeve));
        // The new price is still recorded so data stays current while paused.
        assert_eq!(client.get_source(&Symbol::new(&env, "RFL")).unwrap().price, 1_030_000);
        assert!(client.get_collateral_nav(&sleeve).is_none());
    }

    #[test]
    fn update_within_threshold_does_not_pause() {
        let env = Env::default();
        let (client, _admin, sleeve) = two_feed_sleeve(&env);
        client.update_price(&Symbol::new(&env, "RFL"), &1_015_000, &200_000, &1000);
        assert_eq!(deviation_event_bps(&env, &sleeve), None);
        assert!(!client.is_sleeve_paused(&sleeve));
        assert_eq!(client.get_collateral_nav(&sleeve).unwrap().price, 1_000_000);
    }

    #[test]
    fn registering_a_deviating_source_latches_pause() {
        let env = Env::default();
        let (client, _admin, sleeve) = two_feed_sleeve(&env);
        let asset = client.get_source(&Symbol::new(&env, "NAV")).unwrap().asset;
        client.register_source(&create_source(&env, "THIRD", &asset, "USD_GOV_FUNDS", 1_050_000, 200_000, 1000));
        assert_eq!(deviation_event_bps(&env, &sleeve), Some(500));
        assert!(client.is_sleeve_paused(&sleeve));
    }

    #[test]
    fn clearing_while_still_deviating_relatches_on_next_update() {
        let env = Env::default();
        let (client, _admin, sleeve) = two_feed_sleeve(&env);
        client.update_price(&Symbol::new(&env, "RFL"), &1_030_000, &200_000, &1000);
        client.clear_circuit_breaker(&sleeve);
        assert!(!client.is_sleeve_paused(&sleeve));
        client.update_price(&Symbol::new(&env, "NAV"), &1_000_000, &200_000, &1000);
        assert!(client.is_sleeve_paused(&sleeve));
    }

    #[test]
    fn insufficient_sources_at_update_time_does_not_latch() {
        let env = Env::default();
        let (client, _admin, sleeve) = two_feed_sleeve(&env);
        // Depth drops below the floor: the feed becomes invalid (live hard stop),
        // but that is not a latched deviation pause.
        client.update_price(&Symbol::new(&env, "RFL"), &1_030_000, &10_000, &1000);
        assert!(!client.is_sleeve_paused(&sleeve));
        assert!(client.get_collateral_nav(&sleeve).is_none());
        client.update_price(&Symbol::new(&env, "RFL"), &1_000_000, &200_000, &1000);
        assert_eq!(client.get_collateral_nav(&sleeve).unwrap().price, 1_000_000);
    }
}

// -------------------------------------------------------------------------
// Audit O2 (2026-11-15): timestamps must not be in the future, updates must
// not go backwards, and any future-dated stored source is invalid.
// -------------------------------------------------------------------------
mod o2 {
    use super::*;

    fn err(e: OracleError) -> Result<(), Result<soroban_sdk::Error, soroban_sdk::InvokeError>> {
        Err(Ok(soroban_sdk::Error::from_contract_error(e as u32)))
    }

    fn feed(env: &Env) -> (NavOracleClient<'_>, Symbol, Address) {
        let (client, _admin, _base) = setup_oracle(env);
        let sleeve = Symbol::new(env, "USD_GOV_FUNDS");
        client.configure_sleeve(&sleeve, &200, &2);
        env.ledger().set_timestamp(10_000);
        let asset = Address::generate(env);
        client.register_source(&create_source(env, "NAV", &asset, "USD_GOV_FUNDS", 1_000_000, 200_000, 10_000));
        client.register_source(&create_source(env, "RFL", &asset, "USD_GOV_FUNDS", 1_000_000, 200_000, 10_000));
        (client, sleeve, asset)
    }

    #[test]
    fn update_with_future_timestamp_rejected() {
        let env = Env::default();
        let (client, _sleeve, _asset) = feed(&env);
        let nav = Symbol::new(&env, "NAV");
        assert_eq!(
            client.try_update_price(&nav, &1_000_000, &200_000, &10_001).map(|_| ()),
            err(OracleError::FutureTimestamp)
        );
        assert_eq!(client.get_source(&nav).unwrap().timestamp, 10_000);
    }

    #[test]
    fn register_with_future_timestamp_rejected() {
        let env = Env::default();
        let (client, _sleeve, asset) = feed(&env);
        let src = create_source(&env, "LATE", &asset, "USD_GOV_FUNDS", 1_000_000, 200_000, 99_999);
        assert_eq!(client.try_register_source(&src).map(|_| ()), err(OracleError::FutureTimestamp));
        assert!(client.get_source(&Symbol::new(&env, "LATE")).is_none());
    }

    #[test]
    fn out_of_order_update_rejected_but_equal_and_newer_accepted() {
        let env = Env::default();
        let (client, _sleeve, _asset) = feed(&env);
        let nav = Symbol::new(&env, "NAV");
        env.ledger().set_timestamp(20_000);
        client.update_price(&nav, &1_001_000, &200_000, &15_000);
        assert_eq!(
            client.try_update_price(&nav, &1_002_000, &200_000, &14_999).map(|_| ()),
            err(OracleError::OutOfOrderUpdate)
        );
        client.update_price(&nav, &1_002_000, &200_000, &15_000); // same-time correction
        client.update_price(&nav, &1_003_000, &200_000, &20_000); // ledger "now"
        assert_eq!(client.get_source(&nav).unwrap().price, 1_003_000);
    }

    #[test]
    fn stored_future_dated_source_is_treated_as_invalid() {
        let env = Env::default();
        let (client, sleeve, _asset) = feed(&env);
        assert!(client.get_collateral_nav(&sleeve).is_some());
        // Ledger time moves behind the stored timestamps (e.g. data written
        // before this fix or a misconfigured network): sources are invalid.
        env.ledger().set_timestamp(9_000);
        assert!(client.get_collateral_nav(&sleeve).is_none());
        assert!(client.get_deposit_nav(&sleeve).is_none());
    }
}

// -------------------------------------------------------------------------
// Audit O3 (2026-11-15): single-source configurations are refused.
// -------------------------------------------------------------------------
mod o3 {
    use super::*;

    #[test]
    fn min_valid_sources_below_two_rejected() {
        let env = Env::default();
        let (client, _admin, _base) = setup_oracle(&env);
        let sleeve = Symbol::new(&env, "XLM");
        for n in [0u32, 1] {
            assert_eq!(
                client.try_configure_sleeve(&sleeve, &200, &n),
                Err(Ok(soroban_sdk::Error::from_contract_error(OracleError::InvalidThreshold as u32)))
            );
        }
        assert!(client.get_sleeve_config(&sleeve).is_none());
    }

    #[test]
    fn two_or_more_accepted() {
        let env = Env::default();
        let (client, _admin, _base) = setup_oracle(&env);
        let sleeve = Symbol::new(&env, "XLM");
        client.configure_sleeve(&sleeve, &200, &2);
        assert_eq!(client.get_sleeve_config(&sleeve).unwrap().min_valid_sources, 2);
        client.configure_sleeve(&sleeve, &200, &3);
        assert_eq!(client.get_sleeve_config(&sleeve).unwrap().min_valid_sources, 3);
    }

    #[test]
    fn cannot_lower_an_existing_sleeve_to_one() {
        let env = Env::default();
        let (client, _admin, _base) = setup_oracle(&env);
        let sleeve = Symbol::new(&env, "XLM");
        client.configure_sleeve(&sleeve, &200, &3);
        assert!(client.try_configure_sleeve(&sleeve, &200, &1).is_err());
        assert_eq!(client.get_sleeve_config(&sleeve).unwrap().min_valid_sources, 3);
    }

    #[test]
    fn one_valid_source_never_prices() {
        let env = Env::default();
        let (client, _admin, _base) = setup_oracle(&env);
        env.ledger().set_timestamp(1000);
        let asset = Address::generate(&env);
        client.register_source(&create_source(&env, "ONLY", &asset, "XLM", 107_000_000, 200_000, 1000));
        assert!(client.price(&asset).is_none());
        assert!(client.get_collateral_nav(&Symbol::new(&env, "XLM")).is_none());
        assert!(client.get_deposit_nav(&Symbol::new(&env, "XLM")).is_none());
    }
}

// -------------------------------------------------------------------------
// asset_price (needed by audit P1): fair per-asset price = median of the
// asset's own valid sources; the numéraire (base) is exactly 1.
// -------------------------------------------------------------------------
mod asset_price {
    use super::*;

    #[test]
    fn median_of_the_assets_own_sources() {
        let env = Env::default();
        let (client, _admin, _base) = setup_oracle(&env);
        env.ledger().set_timestamp(1000);
        client.configure_sleeve(&Symbol::new(&env, "EURC"), &300, &2);
        let eurc = Address::generate(&env);
        client.register_source(&create_source(&env, "E1", &eurc, "EURC", 1_080_000, 200_000, 1000));
        client.register_source(&create_source(&env, "E2", &eurc, "EURC", 1_090_000, 200_000, 1000));
        client.register_source(&create_source(&env, "E3", &eurc, "EURC", 1_085_000, 200_000, 1000));
        assert_eq!(client.asset_price(&eurc).unwrap().price, 1_085_000);
    }

    #[test]
    fn base_asset_is_exactly_one() {
        let env = Env::default();
        let (client, _admin, base) = setup_oracle(&env);
        assert_eq!(client.asset_price(&base).unwrap().price, 10_000_000); // 7 decimals
    }

    #[test]
    fn unknown_single_source_or_paused_asset_has_no_price() {
        let env = Env::default();
        let (client, admin, _base) = setup_oracle(&env);
        env.ledger().set_timestamp(1000);
        assert!(client.asset_price(&Address::generate(&env)).is_none());

        let a = Address::generate(&env);
        client.register_source(&create_source(&env, "A1", &a, "A", 1_000_000, 200_000, 1000));
        assert!(client.asset_price(&a).is_none(), "single source");

        client.register_source(&create_source(&env, "A2", &a, "A", 1_000_000, 200_000, 1000));
        assert!(client.asset_price(&a).is_some());
        client.trigger_circuit_breaker(&admin, &Symbol::new(&env, "A"), &Symbol::new(&env, "MANUAL"));
        assert!(client.asset_price(&a).is_none(), "paused");
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
fn golden_events_oracle() {
    let env = Env::default();
    let (client, admin, _base) = setup_oracle(&env);
    let mut g = GoldenEvents::new(&client.address);
    let sleeve = Symbol::new(&env, "USD_GOV_FUNDS");
    client.configure_sleeve(&sleeve, &200, &2);
    env.ledger().set_timestamp(1000);
    let asset = Address::generate(&env);
    client.register_source(&create_source(&env, "NAV", &asset, "USD_GOV_FUNDS", 1_000_000, 200_000, 1000));
    client.register_source(&create_source(&env, "RFL", &asset, "USD_GOV_FUNDS", 1_000_000, 200_000, 1000));
    g.capture(&env, "register_sources");
    client.update_price(&Symbol::new(&env, "NAV"), &1_001_000, &250_000, &1000);
    g.capture(&env, "update_price");
    client.update_price(&Symbol::new(&env, "RFL"), &1_050_000, &250_000, &1000);
    g.capture(&env, "update_price_deviation");
    client.clear_circuit_breaker(&sleeve);
    g.capture(&env, "clear_circuit_breaker");
    client.trigger_circuit_breaker(&admin, &sleeve, &Symbol::new(&env, "MANUAL"));
    g.capture(&env, "trigger_circuit_breaker");
    client.set_pause_authority(&Address::generate(&env));
    g.capture(&env, "set_pause_authority");
    g.finish(&["PRICE_UPDATED", "NAV_PAUSE", "NAV_RESUME", "PAUSE_AUTHORITY_SET"]);
}

// -------------------------------------------------------------------------
// Audit O6 (2026-11-15): prices are evaluated per ASSET. A sleeve may hold
// several assets with different prices (USDC and EURC in cash; BENJI and USDY
// in USD gov funds). Deviation, the 2-source minimum and median/minimum are
// computed among the same asset's sources only (NAV-ORACLE Rules 3-5); the
// sleeve remains the configuration and pause domain.
// -------------------------------------------------------------------------
mod o6 {
    use super::*;

    fn err(e: OracleError) -> Result<(), Result<soroban_sdk::Error, soroban_sdk::InvokeError>> {
        Err(Ok(soroban_sdk::Error::from_contract_error(e as u32)))
    }

    /// Sleeve "CASH" (2% threshold, >= 2 sources) holding X at $1.00 and Y at $1.08.
    fn two_asset_sleeve(env: &Env) -> (NavOracleClient<'_>, Symbol, Address, Address) {
        let (client, _admin, _base) = setup_oracle(env);
        env.ledger().set_timestamp(1000);
        let sleeve = Symbol::new(env, "CASH");
        client.configure_sleeve(&sleeve, &200, &2);
        let (x, y) = (Address::generate(env), Address::generate(env));
        client.register_source(&create_source(env, "X_A", &x, "CASH", 1_000_000, 200_000, 1000));
        client.register_source(&create_source(env, "X_B", &x, "CASH", 1_001_000, 200_000, 1000));
        client.register_source(&create_source(env, "Y_A", &y, "CASH", 1_080_000, 200_000, 1000));
        client.register_source(&create_source(env, "Y_B", &y, "CASH", 1_082_000, 200_000, 1000));
        (client, sleeve, x, y)
    }

    #[test]
    fn different_assets_in_one_sleeve_do_not_trip_the_breaker() {
        let env = Env::default();
        let (client, sleeve, x, y) = two_asset_sleeve(&env);
        assert!(!client.is_sleeve_paused(&sleeve), "8% between two healthy assets is not a deviation");
        assert_eq!(client.asset_price(&x).unwrap().price, 1_000_500); // median of X only
        assert_eq!(client.asset_price(&y).unwrap().price, 1_081_000); // median of Y only
        assert_eq!(client.price(&x).unwrap().price, 1_000_000); // conservative = min of X
        assert_eq!(client.price(&y).unwrap().price, 1_080_000);
        // A price update within one asset's own band keeps the sleeve live.
        client.update_price(&Symbol::new(&env, "Y_B"), &1_085_000, &200_000, &1000);
        assert!(!client.is_sleeve_paused(&sleeve));
    }

    #[test]
    fn a_deviating_asset_still_pauses_its_sleeve() {
        let env = Env::default();
        let (client, sleeve, x, y) = two_asset_sleeve(&env);
        assert!(!client.is_sleeve_paused(&sleeve), "precondition: healthy sleeve is live");
        client.update_price(&Symbol::new(&env, "Y_B"), &1_200_000, &200_000, &1000); // Y sources now 11% apart
        assert!(client.is_sleeve_paused(&sleeve), "Rule 4: deviation pauses the affected sleeve");
        assert_eq!(client.asset_price(&y), None);
        assert_eq!(client.asset_price(&x), None, "the whole affected sleeve halts");
    }

    #[test]
    fn the_two_source_minimum_is_per_asset() {
        let env = Env::default();
        let (client, _admin, _base) = setup_oracle(&env);
        env.ledger().set_timestamp(1000);
        let sleeve = Symbol::new(&env, "GOV");
        client.configure_sleeve(&sleeve, &200, &2);
        let (x, y) = (Address::generate(&env), Address::generate(&env));
        client.register_source(&create_source(&env, "X_A", &x, "GOV", 1_000_000, 200_000, 1000));
        client.register_source(&create_source(&env, "X_B", &x, "GOV", 1_000_000, 200_000, 1000));
        client.register_source(&create_source(&env, "Y_ONLY", &y, "GOV", 1_000_000, 200_000, 1000));
        assert!(client.asset_price(&x).is_some());
        assert_eq!(client.asset_price(&y), None, "one source for Y must never price Y");
        assert_eq!(client.price(&y), None);
    }

    #[test]
    fn sleeve_level_nav_refuses_a_multi_asset_sleeve() {
        let env = Env::default();
        let (client, sleeve, x, _y) = two_asset_sleeve(&env);
        assert!(!client.is_sleeve_paused(&sleeve), "precondition: refused for being multi-asset, not paused");
        assert!(client.asset_price(&x).is_some());
        assert_eq!(client.get_deposit_nav(&sleeve), None, "a blended price across assets is meaningless");
        assert_eq!(client.get_collateral_nav(&sleeve), None);
        // Reading must not freeze the sleeve (pre-fix, this read compared all
        // assets' sources and latched a DEVIATION pause anyone could trigger).
        assert!(!client.is_sleeve_paused(&sleeve), "a read paused the sleeve");
        assert!(client.asset_price(&x).is_some());
    }

    #[test]
    fn an_asset_lives_in_exactly_one_sleeve_and_sources_cannot_be_repointed() {
        let env = Env::default();
        let (client, _sleeve, x, y) = two_asset_sleeve(&env);
        client.configure_sleeve(&Symbol::new(&env, "OTHER"), &200, &2);
        let moved = create_source(&env, "X_C", &x, "OTHER", 1_000_000, 200_000, 1000);
        assert_eq!(client.try_register_source(&moved).map(|_| ()), err(OracleError::AssetSleeveMismatch));
        // Re-registering an existing source id for another asset is refused.
        let hijack = create_source(&env, "X_A", &y, "CASH", 1_080_000, 200_000, 1000);
        assert_eq!(client.try_register_source(&hijack).map(|_| ()), err(OracleError::AssetSleeveMismatch));
        assert_eq!(client.get_source(&Symbol::new(&env, "X_A")).unwrap().asset, x);
    }
}
