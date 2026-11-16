//! The real Blend v2 environment, built the same way as Blend's own test suite
//! (`blend-contracts-v2/test-suites/src/test_fixture.rs` + `setup.rs`) but from
//! the WASMs deployed on mainnet: BLND token, Comet 80/20 BLND:USDC LP, emitter,
//! backstop v2, pool factory v2 and a pool v2 with one USDC reserve, activated
//! through a real backstop deposit (status set to 3, then `update_status`).
//!
//! The pool's oracle is whatever the caller passes: Constella's NAV oracle in
//! the integration tests, so the real Blend pool reads Constella's SEP-40 feed.
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, BytesN, Env, String,
};

#[allow(clippy::too_many_arguments)]
pub mod pool {
    soroban_sdk::contractimport!(file = "testdata/blend/blend_pool_v2_fixed_mainnet.wasm");
}
#[allow(clippy::too_many_arguments)]
pub mod backstop {
    soroban_sdk::contractimport!(file = "testdata/blend/blend_backstop_v2_mainnet.wasm");
}
#[allow(clippy::too_many_arguments)]
pub mod pool_factory {
    soroban_sdk::contractimport!(file = "testdata/blend/blend_pool_factory_v2_mainnet.wasm");
}
#[allow(clippy::too_many_arguments)]
pub mod emitter {
    soroban_sdk::contractimport!(file = "testdata/blend/blend_emitter_mainnet.wasm");
}
#[allow(clippy::too_many_arguments)]
pub mod comet {
    soroban_sdk::contractimport!(file = "testdata/blend/blend_comet_mainnet.wasm");
}

/// 7-decimal fixed point (Blend's SCALAR_7).
pub const SCALAR_7: i128 = 10_000_000;
/// Blend v2 request types (`pool/src/pool/actions.rs`).
pub const SUPPLY: u32 = 0;
pub const WITHDRAW: u32 = 1;
pub const SUPPLY_COLLATERAL: u32 = 2;
pub const BORROW: u32 = 4;
pub const REPAY: u32 = 5;
/// Blend v2 b_rate / d_rate scale (12 decimals).
pub const RATE_SCALE: i128 = 1_000_000_000_000;

pub struct BlendEnv<'a> {
    pub pool: pool::Client<'a>,
    pub backstop: backstop::Client<'a>,
    pub admin: Address,
    pub blnd: Address,
    pub usdc: Address,
    /// Index of the USDC reserve in the pool.
    pub usdc_index: u32,
}

/// Blend's default reserve parameters (`test-suites/src/pool.rs`), 7 decimals.
pub fn default_reserve(decimals: u32) -> pool::ReserveConfig {
    pool::ReserveConfig {
        decimals,
        c_factor: 7_500_000,
        l_factor: 7_500_000,
        util: 7_500_000,
        max_util: 9_500_000,
        r_base: 100_000,
        r_one: 500_000,
        r_two: 5_000_000,
        r_three: 15_000_000,
        reactivity: 20,
        index: 0,
        supply_cap: 1_000_000_000_000_000_000,
        enabled: true,
    }
}

/// Deploy and activate a full Blend v2 environment with one USDC reserve.
/// Requires `env.mock_all_auths()` (setup only). `usdc_admin` must be able to
/// mint `usdc`. Advances the ledger by one week (backstop drop schedule).
pub fn deploy<'a>(env: &Env, usdc_admin: &StellarAssetClient<'a>, oracle: &Address) -> BlendEnv<'a> {
    let usdc = usdc_admin.address.clone();
    let admin = Address::generate(env);
    let depositor = Address::generate(env);

    let blnd_sac = env.register_stellar_asset_contract_v2(admin.clone());
    let blnd = blnd_sac.address();
    let blnd_admin = StellarAssetClient::new(env, &blnd);

    // Comet 80/20 BLND:USDC LP (backstop token), as Blend's create_lp_pool.
    let lp = env.register(comet::WASM, ());
    let comet = comet::Client::new(env, &lp);
    blnd_admin.mint(&admin, &(1_000 * SCALAR_7));
    usdc_admin.mint(&admin, &(25 * SCALAR_7));
    comet.init(
        &admin,
        &vec![env, blnd.clone(), usdc.clone()],
        &vec![env, 8_000_000, 2_000_000],
        &vec![env, 1_000 * SCALAR_7, 25 * SCALAR_7],
        &30_000,
    );

    // Backstop and pool factory reference each other: fix their addresses first.
    let backstop_id = Address::generate(env);
    let factory_id = Address::generate(env);

    let emitter_id = env.register(emitter::WASM, ());
    let emitter = emitter::Client::new(env, &emitter_id);
    blnd_admin.set_admin(&emitter_id);
    emitter.initialize(&blnd, &backstop_id, &lp);

    let drop_list = vec![env, (admin.clone(), 10_000_000 * SCALAR_7), (depositor.clone(), 30_000_000 * SCALAR_7)];
    env.register_at(
        &backstop_id,
        backstop::WASM,
        (lp.clone(), emitter_id.clone(), blnd.clone(), usdc.clone(), factory_id.clone(), drop_list),
    );
    let backstop = backstop::Client::new(env, &backstop_id);
    let pool_hash = env.deployer().upload_contract_wasm(pool::WASM);
    env.register_at(
        &factory_id,
        pool_factory::WASM,
        (pool_factory::PoolInitMeta { backstop: backstop_id.clone(), pool_hash, blnd_id: blnd.clone() },),
    );
    let factory = pool_factory::Client::new(env, &factory_id);
    backstop.drop();
    backstop.distribute();
    let now = env.ledger().timestamp();
    env.ledger().set_timestamp(now + 7 * 24 * 3600);

    // Pool: 10 % backstop take, 6 positions, min collateral 1.0 (setup.rs values).
    let pool_id = factory.deploy(
        &admin,
        &String::from_str(env, "Constella test pool"),
        &BytesN::from_array(env, &[7u8; 32]),
        oracle,
        &1_000_000,
        &6,
        &SCALAR_7,
    );
    let pool = pool::Client::new(env, &pool_id);
    pool.queue_set_reserve(&usdc, &default_reserve(7));
    let usdc_index = pool.set_reserve(&usdc);

    // Activate through a real backstop deposit: 30M BLND from the drop + 70M
    // minted (setup.rs), joined into 10M LP shares; 50k shares backstop the pool.
    blnd_admin.mint(&depositor, &(70_000_000 * SCALAR_7));
    usdc_admin.mint(&depositor, &(2_600_000 * SCALAR_7));
    comet.join_pool(
        &(10_000_000 * SCALAR_7),
        &vec![env, 110_000_000 * SCALAR_7, 2_600_000 * SCALAR_7],
        &depositor,
    );
    backstop.deposit(&depositor, &pool_id, &(50_000 * SCALAR_7));
    backstop.add_reward(&pool_id, &None);
    pool.set_status(&3);
    pool.update_status();

    BlendEnv { pool, backstop, admin, blnd, usdc, usdc_index }
}

/// Add a reserve for `asset` to a live pool. Blend requires a one-week
/// timelock for reserve changes once the pool has left setup status
/// (`pool/src/pool/config.rs`), so this advances the ledger by a week + 1 s.
pub fn add_reserve(env: &Env, b: &BlendEnv, asset: &Address, decimals: u32) -> u32 {
    b.pool.queue_set_reserve(asset, &default_reserve(decimals));
    let now = env.ledger().timestamp();
    env.ledger().set_timestamp(now + 7 * 24 * 3600 + 1);
    b.pool.set_reserve(asset)
}

#[cfg(test)]
mod test;
