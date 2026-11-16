//! Smoke test of the fixture itself: the real pool is active and plain
//! (non-collateral) supply / withdraw behave as Blend's source says.
use super::*;
use soroban_sdk::token::TokenClient;

fn setup() -> (Env, BlendEnv<'static>, StellarAssetClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    env.cost_estimate().budget().reset_unlimited();
    env.ledger().set_timestamp(1_731_600_000);
    let usdc = env.register_stellar_asset_contract_v2(Address::generate(&env)).address();
    let usdc_admin = StellarAssetClient::new(&env, &usdc);
    // Supply / withdraw never read the oracle; any address will do here.
    let oracle = Address::generate(&env);
    let b = deploy(&env, &usdc_admin, &oracle);
    (env, b, usdc_admin)
}

#[test]
fn the_pool_is_active_and_accepts_supply() {
    let (env, b, usdc_admin) = setup();
    assert!(b.pool.get_config().status <= 1, "active after the backstop deposit");
    let user = Address::generate(&env);
    usdc_admin.mint(&user, &(1_000 * SCALAR_7));
    let req = soroban_sdk::vec![&env, pool::Request { request_type: SUPPLY, address: b.usdc.clone(), amount: 1_000 * SCALAR_7 }];
    let positions = b.pool.submit(&user, &user, &user, &req);
    let reserve = b.pool.get_reserve(&b.usdc);
    assert_eq!(reserve.data.b_rate, RATE_SCALE, "fresh reserve: 1 bToken = 1 USDC");
    assert_eq!(positions.supply.get(b.usdc_index), Some(1_000 * SCALAR_7));
    assert!(positions.collateral.is_empty() && positions.liabilities.is_empty());
    assert_eq!(TokenClient::new(&env, &b.usdc).balance(&user), 0);
}

#[test]
fn withdrawing_more_than_the_position_pays_out_the_whole_position() {
    let (env, b, usdc_admin) = setup();
    let user = Address::generate(&env);
    usdc_admin.mint(&user, &(1_000 * SCALAR_7));
    b.pool.submit(&user, &user, &user, &soroban_sdk::vec![&env, pool::Request { request_type: SUPPLY, address: b.usdc.clone(), amount: 1_000 * SCALAR_7 }]);
    let after = b.pool.submit(&user, &user, &user, &soroban_sdk::vec![&env, pool::Request { request_type: WITHDRAW, address: b.usdc.clone(), amount: 5_000 * SCALAR_7 }]);
    assert_eq!(after.supply.get(b.usdc_index), None);
    assert_eq!(TokenClient::new(&env, &b.usdc).balance(&user), 1_000 * SCALAR_7);
}
