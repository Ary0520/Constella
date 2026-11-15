#![cfg(test)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::inconsistent_digit_grouping)]

extern crate std;

use super::*;
use constella_nav_oracle::{NavOracle, NavOracleClient, OracleSource as ExtOracleSource};
use constella_policy::{PolicyContract, PolicyContractClient, SleeveTarget};
use constella_registry::{
    AssetEntry, AssetRegistry, AssetRegistryClient, SleeveType as RegSleeveType,
};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env, String, Symbol, Vec as SdkVec,
};

#[allow(dead_code)]
struct TestContext<'a> {
    env: Env,
    admin: Address,
    pause_authority: Address,
    treasury: Address,
    alice: Address,
    bob: Address,
    numeraire_address: Address,
    numeraire_admin: soroban_sdk::token::StellarAssetClient<'a>,
    numeraire_client: soroban_sdk::token::Client<'a>,
    oracle_client: NavOracleClient<'a>,
    policy_client: PolicyContractClient<'a>,
    registry_client: AssetRegistryClient<'a>,
    vault_client: StrategyVaultClient<'a>,
    vault_id: Address,
}

fn setup_test_context() -> TestContext<'static> {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000_000);
    env.ledger().set_sequence_number(100);

    let admin = Address::generate(&env);
    let pause_authority = Address::generate(&env);
    let treasury = Address::generate(&env);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);

    // Deploy Numéraire (USDC)
    let token_admin = Address::generate(&env);
    let numeraire_sac = env.register_stellar_asset_contract_v2(token_admin);
    let numeraire_address = numeraire_sac.address();
    let numeraire_admin = soroban_sdk::token::StellarAssetClient::new(&env, &numeraire_address);
    let numeraire_client = soroban_sdk::token::Client::new(&env, &numeraire_address);

    // Deploy Registry
    let registry_id = env.register(AssetRegistry, ());
    let registry_client = AssetRegistryClient::new(&env, &registry_id);
    registry_client.initialize(&admin);

    // Register USDC in registry as Cash
    let mut usdc_sources = SdkVec::new(&env);
    usdc_sources.push_back(Symbol::new(&env, "USDC_SRC"));
    registry_client.add_asset(&AssetEntry {
        contract_address: numeraire_address.clone(),
        asset_id: Symbol::new(&env, "USDC"),
        sleeve_type: RegSleeveType::Cash,
        liquidity_tier: 0,
        permissionless: true,
        oracle_sources: usdc_sources,
        eligible_jurisdictions: SdkVec::new(&env),
        blocked_jurisdictions: SdkVec::new(&env),
        investor_classes_required: SdkVec::new(&env),
        is_active: true,
        added_at: 1_000_000,
    });

    // Deploy Policy
    let policy_id = env.register(PolicyContract, ());
    let policy_client = PolicyContractClient::new(&env, &policy_id);
    policy_client.initialize(&admin);

    // Deploy Oracle
    let oracle_id = env.register(NavOracle, ());
    let oracle_client = NavOracleClient::new(&env, &oracle_id);
    oracle_client.initialize(&admin, &numeraire_address, &7, &300);

    // Deploy Vault
    let vault_id = env.register(StrategyVault, ());
    let vault_client = StrategyVaultClient::new(&env, &vault_id);

    let config = VaultConfig {
        admin: admin.clone(),
        pause_authority: pause_authority.clone(),
        numeraire: numeraire_address.clone(),
        policy_contract: policy_id.clone(),
        oracle_contract: oracle_id.clone(),
        registry_contract: registry_id.clone(),
        treasury: treasury.clone(),
        management_fee_bps: 100, // 1% per year
    };

    vault_client.initialize(
        &config,
        &String::from_str(&env, "Constella Strategy Vault"),
        &String::from_str(&env, "CONST"),
        &7,
    );

    TestContext {
        env,
        admin,
        pause_authority,
        treasury,
        alice,
        bob,
        numeraire_address,
        numeraire_admin,
        numeraire_client,
        oracle_client,
        policy_client,
        registry_client,
        vault_client,
        vault_id,
    }
}

fn configure_vault_oracle_feeds(ctx: &TestContext) {
    let sleeve_sym = Symbol::new(&ctx.env, "VAULT");
    ctx.oracle_client.configure_sleeve(&sleeve_sym, &300, &2);

    let dummy_asset = Address::generate(&ctx.env);

    // 3 feeds: Median = $1.00 (10_000_000), Min = $0.99 (9_900_000), Max = $1.01 (10_100_000)
    let s1 = ExtOracleSource {
        source_id: Symbol::new(&ctx.env, "MEDIAN_SRC"),
        asset: dummy_asset.clone(),
        sleeve: sleeve_sym.clone(),
        price: 10_000_000,
        timestamp: 1_000_000,
        staleness_limit_seconds: 14400,
        min_liquidity_depth_usd: 100_000,
        current_liquidity_depth_usd: 200_000,
        is_active: true,
    };
    let s2 = ExtOracleSource {
        source_id: Symbol::new(&ctx.env, "CONSERVATIVE_SRC"),
        asset: dummy_asset.clone(),
        sleeve: sleeve_sym.clone(),
        price: 9_900_000,
        timestamp: 1_000_000,
        staleness_limit_seconds: 14400,
        min_liquidity_depth_usd: 100_000,
        current_liquidity_depth_usd: 200_000,
        is_active: true,
    };
    let s3 = ExtOracleSource {
        source_id: Symbol::new(&ctx.env, "HIGH_SRC"),
        asset: dummy_asset,
        sleeve: sleeve_sym,
        price: 10_100_000,
        timestamp: 1_000_000,
        staleness_limit_seconds: 14400,
        min_liquidity_depth_usd: 100_000,
        current_liquidity_depth_usd: 200_000,
        is_active: true,
    };

    ctx.oracle_client.register_source(&s1);
    ctx.oracle_client.register_source(&s2);
    ctx.oracle_client.register_source(&s3);
}

// -------------------------------------------------------------------------
// T40: SEP-56 Deposit & Mint with Median NAV Valuation
// -------------------------------------------------------------------------
#[test]
fn test_t40_deposit_and_mint_median_nav() {
    let ctx = setup_test_context();
    configure_vault_oracle_feeds(&ctx);

    // Verify Oracle Dual NAV
    let dep_nav = ctx.vault_client.get_nav_deposit().unwrap();
    let cons_nav = ctx.vault_client.get_nav_conservative().unwrap();
    assert_eq!(dep_nav.price, 10_000_000); // $1.00 median
    assert_eq!(cons_nav.price, 9_900_000);  // $0.99 conservative

    // Mint 1000 USDC to Alice
    ctx.numeraire_admin.mint(&ctx.alice, &1000_0000000);
    assert_eq!(ctx.numeraire_client.balance(&ctx.alice), 1000_0000000);

    // Alice deposits 500 USDC
    let shares_minted = ctx.vault_client.deposit(&500_0000000, &ctx.alice);
    assert_eq!(shares_minted, 500_0000000);

    // Verify balances after deposit
    assert_eq!(ctx.vault_client.balance(&ctx.alice), 500_0000000);
    assert_eq!(ctx.vault_client.total_supply(), 500_0000000);
    assert_eq!(ctx.vault_client.total_assets(), 500_0000000);
    assert_eq!(ctx.numeraire_client.balance(&ctx.alice), 500_0000000);
    assert_eq!(ctx.numeraire_client.balance(&ctx.vault_id), 500_0000000);

    // Alice mints 250 shares
    let assets_paid = ctx.vault_client.mint(&250_0000000, &ctx.alice);
    assert_eq!(assets_paid, 250_0000000);

    // Verify balances after mint
    assert_eq!(ctx.vault_client.balance(&ctx.alice), 750_0000000);
    assert_eq!(ctx.vault_client.total_supply(), 750_0000000);
    assert_eq!(ctx.vault_client.total_assets(), 750_0000000);
    assert_eq!(ctx.numeraire_client.balance(&ctx.alice), 250_0000000);
    assert_eq!(ctx.numeraire_client.balance(&ctx.vault_id), 750_0000000);

    // Test convert functions
    assert_eq!(ctx.vault_client.convert_to_shares(&100_0000000), 100_0000000);
    assert_eq!(ctx.vault_client.asset(), ctx.numeraire_address);
}

// -------------------------------------------------------------------------
// T41: Instant Redemption with Conservative NAV Valuation
// -------------------------------------------------------------------------
#[test]
fn test_t41_instant_redemption_conservative_nav() {
    let ctx = setup_test_context();
    configure_vault_oracle_feeds(&ctx);

    // Deposit 1000 USDC by Alice at $1.00 NAV
    ctx.numeraire_admin.mint(&ctx.alice, &1000_0000000);
    ctx.vault_client.deposit(&1000_0000000, &ctx.alice);
    assert_eq!(ctx.vault_client.balance(&ctx.alice), 1000_0000000);

    // Verify Invariant: conservative_nav <= deposit_nav
    let cons_nav = ctx.vault_client.get_nav_conservative().unwrap();
    let dep_nav = ctx.vault_client.get_nav_deposit().unwrap();
    assert!(cons_nav.price <= dep_nav.price);
    assert_eq!(cons_nav.price, 9_900_000); // 0.99 conservative

    // Alice redeems 100 shares for Bob
    // Assets = 100 shares * 0.99 = 99 USDC
    let assets_out = ctx.vault_client.redeem(&100_0000000, &ctx.bob, &ctx.alice);
    assert_eq!(assets_out, 99_0000000);
    assert_eq!(ctx.numeraire_client.balance(&ctx.bob), 99_0000000);
    assert_eq!(ctx.vault_client.balance(&ctx.alice), 900_0000000);
    assert_eq!(ctx.vault_client.total_supply(), 900_0000000);

    // Alice withdraws 99 USDC directly
    // Shares needed = (99 * 10^7 * 10^7 + 9_899_999) / 9_900_000 = 100 shares
    let shares_burned = ctx.vault_client.withdraw(&99_0000000, &ctx.alice, &ctx.alice);
    assert_eq!(shares_burned, 100_0000000);
    assert_eq!(ctx.vault_client.balance(&ctx.alice), 800_0000000);
    assert_eq!(ctx.vault_client.total_supply(), 800_0000000);
    assert_eq!(ctx.numeraire_client.balance(&ctx.alice), 99_0000000);
}

// -------------------------------------------------------------------------
// T42: Fee Dilution Calculation Minting Shares to Treasury Over Time
// -------------------------------------------------------------------------
#[test]
fn test_t42_fee_dilution_calculation() {
    let ctx = setup_test_context();
    configure_vault_oracle_feeds(&ctx);

    // Management fee = 100 bps = 1% / year
    ctx.numeraire_admin.mint(&ctx.alice, &1000_0000000);
    ctx.vault_client.deposit(&1000_0000000, &ctx.alice);

    let initial_shares = ctx.vault_client.total_supply();
    let initial_assets = ctx.vault_client.total_assets();
    assert_eq!(initial_shares, 1000_0000000);
    assert_eq!(initial_assets, 1000_0000000);
    assert_eq!(ctx.vault_client.balance(&ctx.treasury), 0);

    // Advance time by exactly 1 year (31,536,000 seconds)
    ctx.env.ledger().set_timestamp(1_000_000 + 31_536_000);

    // Accrue fees
    let fee_shares = ctx.vault_client.accrue_fees();
    // 1% of 1000 shares = 10 shares
    assert_eq!(fee_shares, 10_0000000);

    // Verify treasury received diluted shares
    assert_eq!(ctx.vault_client.balance(&ctx.treasury), 10_0000000);
    assert_eq!(ctx.vault_client.total_supply(), 1010_0000000);
    assert_eq!(ctx.vault_client.balance(&ctx.alice), 1000_0000000);

    // Critical Invariant: Total underlying assets unchanged!
    assert_eq!(ctx.vault_client.total_assets(), initial_assets);
}

// -------------------------------------------------------------------------
// T43: Collateral Passport View Functions
// -------------------------------------------------------------------------
#[test]
fn test_t43_collateral_passport_views() {
    let ctx = setup_test_context();
    configure_vault_oracle_feeds(&ctx);

    // Deposit initial capital
    ctx.numeraire_admin.mint(&ctx.alice, &500_0000000);
    ctx.vault_client.deposit(&500_0000000, &ctx.alice);

    // 1. Dual NAV
    let cons_nav = ctx.vault_client.get_nav_conservative().unwrap();
    let dep_nav = ctx.vault_client.get_nav_deposit().unwrap();
    assert_eq!(cons_nav.price, 9_900_000);
    assert_eq!(dep_nav.price, 10_000_000);
    assert!(cons_nav.price <= dep_nav.price);

    // 2. Redemption Capacity
    let cap = ctx.vault_client.get_redemption_capacity();
    assert_eq!(cap.instant, 500_0000000);
    assert_eq!(cap.in_24h, 0);
    assert_eq!(cap.in_1w, 0);
    assert_eq!(cap.longer, 0);
    assert_eq!(cap.computed_at, 1_000_000);

    // 3. Risk Sheet
    let risk = ctx.vault_client.get_risk_sheet();
    assert_eq!(risk.recommended_ltv_bps, 9800); // 98%
    assert_eq!(risk.recommended_haircut_bps, 200); // 2%
    assert_eq!(risk.weighted_avg_redemption_hours, 0); // cash instant

    // 4. Liquidator Eligibility
    assert!(ctx.vault_client.is_eligible_liquidator(&ctx.alice));
    assert!(ctx.vault_client.is_eligible_liquidator(&ctx.bob));

    // 5. Listing Kit
    let kit = ctx.vault_client.get_listing_kit();
    assert_eq!(kit.share_token, ctx.vault_id);
    assert_eq!(kit.numeraire, ctx.numeraire_address);
    assert_eq!(kit.recommended_ltv_bps, 9800);
    assert_eq!(kit.recommended_haircut_bps, 200);
    assert_eq!(kit.liquidation_threshold_bps, 9900);
}

// -------------------------------------------------------------------------
// T44: Emergency Pause & Unpause Authorization Checks
// -------------------------------------------------------------------------
#[test]
fn test_t44_emergency_pause_and_unpause() {
    let ctx = setup_test_context();
    configure_vault_oracle_feeds(&ctx);

    ctx.numeraire_admin.mint(&ctx.alice, &100_0000000);

    // Pause vault via pause authority
    assert!(!ctx.vault_client.is_paused());
    ctx.vault_client.pause();
    assert!(ctx.vault_client.is_paused());

    // Limits must return 0 when paused
    assert_eq!(ctx.vault_client.max_deposit(&ctx.alice), 0);
    assert_eq!(ctx.vault_client.max_mint(&ctx.alice), 0);
    assert_eq!(ctx.vault_client.max_withdraw(&ctx.alice), 0);
    assert_eq!(ctx.vault_client.max_redeem(&ctx.alice), 0);

    // Operations must panic with VaultPaused
    let res = ctx.vault_client.try_deposit(&50_0000000, &ctx.alice);
    assert_eq!(res.unwrap_err().unwrap(), VaultError::VaultPaused.into());

    let res_mint = ctx.vault_client.try_mint(&50_0000000, &ctx.alice);
    assert_eq!(res_mint.unwrap_err().unwrap(), VaultError::VaultPaused.into());

    let res_transfer = ctx.vault_client.try_transfer(&ctx.alice, &ctx.bob, &10_0000000);
    assert_eq!(res_transfer.unwrap_err().unwrap(), VaultError::VaultPaused.into());

    // Unpause vault via admin
    ctx.vault_client.unpause();
    assert!(!ctx.vault_client.is_paused());

    // Operations resume normally
    let shares = ctx.vault_client.deposit(&50_0000000, &ctx.alice);
    assert_eq!(shares, 50_0000000);
    assert_eq!(ctx.vault_client.balance(&ctx.alice), 50_0000000);
}

// -------------------------------------------------------------------------
// T45: SEP-41 Token Standard Conformance
// -------------------------------------------------------------------------
#[test]
fn test_t45_sep41_token_standard() {
    let ctx = setup_test_context();
    configure_vault_oracle_feeds(&ctx);

    ctx.numeraire_admin.mint(&ctx.alice, &500_0000000);
    ctx.vault_client.deposit(&500_0000000, &ctx.alice);

    // Metadata
    assert_eq!(ctx.vault_client.decimals(), 7);
    assert_eq!(ctx.vault_client.symbol(), String::from_str(&ctx.env, "CONST"));
    assert_eq!(
        ctx.vault_client.name(),
        String::from_str(&ctx.env, "Constella Strategy Vault")
    );

    // Transfer
    ctx.vault_client.transfer(&ctx.alice, &ctx.bob, &100_0000000);
    assert_eq!(ctx.vault_client.balance(&ctx.alice), 400_0000000);
    assert_eq!(ctx.vault_client.balance(&ctx.bob), 100_0000000);
    assert_eq!(ctx.vault_client.total_supply(), 500_0000000);

    // Approve & Allowance
    ctx.vault_client.approve(&ctx.alice, &ctx.bob, &50_0000000, &200);
    assert_eq!(ctx.vault_client.allowance(&ctx.alice, &ctx.bob), 50_0000000);

    // Transfer from
    let charlie = Address::generate(&ctx.env);
    ctx.vault_client
        .transfer_from(&ctx.bob, &ctx.alice, &charlie, &30_0000000);
    assert_eq!(ctx.vault_client.balance(&ctx.alice), 370_0000000);
    assert_eq!(ctx.vault_client.balance(&charlie), 30_0000000);
    assert_eq!(ctx.vault_client.allowance(&ctx.alice, &ctx.bob), 20_0000000);

    // Allowance expiration
    ctx.env.ledger().set_sequence_number(201);
    assert_eq!(ctx.vault_client.allowance(&ctx.alice, &ctx.bob), 0);
    let expired_res = ctx
        .vault_client
        .try_transfer_from(&ctx.bob, &ctx.alice, &charlie, &10_0000000);
    assert_eq!(
        expired_res.unwrap_err().unwrap(),
        VaultError::InsufficientAllowance.into()
    );
}

// -------------------------------------------------------------------------
// T46: Liquidity Sleeve Floor & Capacity Boundary Protection
// -------------------------------------------------------------------------
#[test]
fn test_t46_liquidity_sleeve_floor_protection() {
    let ctx = setup_test_context();

    // Deploy an RWA token (BENJI)
    let benji_admin = Address::generate(&ctx.env);
    let benji_sac = ctx.env.register_stellar_asset_contract_v2(benji_admin);
    let benji_address = benji_sac.address();

    // Register BENJI in Asset Registry
    let mut benji_sources = SdkVec::new(&ctx.env);
    benji_sources.push_back(Symbol::new(&ctx.env, "BENJI_MEDIAN"));
    ctx.registry_client.add_asset(&AssetEntry {
        contract_address: benji_address.clone(),
        asset_id: Symbol::new(&ctx.env, "BENJI"),
        sleeve_type: RegSleeveType::UsdGovFund,
        liquidity_tier: 1, // 24h liquidity
        permissionless: true,
        oracle_sources: benji_sources,
        eligible_jurisdictions: SdkVec::new(&ctx.env),
        blocked_jurisdictions: SdkVec::new(&ctx.env),
        investor_classes_required: SdkVec::new(&ctx.env),
        is_active: true,
        added_at: 1_000_000,
    });

    // Configure oracle for USD_GOV_FUNDS sleeve
    let gov_sym = Symbol::new(&ctx.env, "USD_GOV_FUNDS");
    ctx.oracle_client.configure_sleeve(&gov_sym, &300, &2);
    let s1 = ExtOracleSource {
        source_id: Symbol::new(&ctx.env, "BENJI_MEDIAN"),
        asset: benji_address.clone(),
        sleeve: gov_sym.clone(),
        price: 10_000_000,
        timestamp: 1_000_000,
        staleness_limit_seconds: 14400,
        min_liquidity_depth_usd: 100_000,
        current_liquidity_depth_usd: 200_000,
        is_active: true,
    };
    let s2 = ExtOracleSource {
        source_id: Symbol::new(&ctx.env, "BENJI_MIN"),
        asset: benji_address.clone(),
        sleeve: gov_sym,
        price: 9_900_000,
        timestamp: 1_000_000,
        staleness_limit_seconds: 14400,
        min_liquidity_depth_usd: 100_000,
        current_liquidity_depth_usd: 200_000,
        is_active: true,
    };
    ctx.oracle_client.register_source(&s1);
    ctx.oracle_client.register_source(&s2);

    // Register policy for vault: 10% liquidity floor (1000 bps)
    let dummy_hash = soroban_sdk::BytesN::from_array(&ctx.env, &[7u8; 32]);
    let mut policy_targets = SdkVec::new(&ctx.env);
    policy_targets.push_back(SleeveTarget {
        asset_id: Symbol::new(&ctx.env, "USDC"),
        target_weight_bps: 2000,
        min_band_bps: 1000,
        max_band_bps: 3000,
        sleeve_type: Symbol::new(&ctx.env, "cash"),
    });
    policy_targets.push_back(SleeveTarget {
        asset_id: Symbol::new(&ctx.env, "BENJI"),
        target_weight_bps: 8000,
        min_band_bps: 7000,
        max_band_bps: 9000,
        sleeve_type: Symbol::new(&ctx.env, "usd_gov_fund"),
    });
    ctx.policy_client.register_policy(
        &ctx.vault_id,
        &dummy_hash,
        &ctx.registry_client.address,
        &1000, // 10% min liquidity sleeve floor
        &8000,
        &8000,
        &50,
        &200,
        &policy_targets,
    );

    // Initial setup: Admin deposits $100,000 to seed the vault
    ctx.numeraire_admin.mint(&ctx.admin, &100_000_0000000);
    ctx.vault_client.deposit(&100_000_0000000, &ctx.admin);
    assert_eq!(ctx.vault_client.total_supply(), 100_000_0000000);

    // Set multi-sleeve configuration on vault:
    // CASH_STABLE: 20% target, $20,000 balance
    // USD_GOV_FUNDS: 80% target, $80,000 balance
    let mut sleeves = SdkVec::new(&ctx.env);
    sleeves.push_back(SleeveState {
        asset_id: String::from_str(&ctx.env, "USDC"),
        contract_address: ctx.numeraire_address.clone(),
        target_weight_bps: 2000,
        band_bps: 200,
        current_balance: 20_000_0000000,
        sleeve_type: SleeveType::CashStable,
        liquidity_tier: 0,
    });
    sleeves.push_back(SleeveState {
        asset_id: String::from_str(&ctx.env, "BENJI"),
        contract_address: benji_address,
        target_weight_bps: 8000,
        band_bps: 200,
        current_balance: 80_000_0000000,
        sleeve_type: SleeveType::UsdGovFunds,
        liquidity_tier: 1,
    });
    ctx.vault_client.set_sleeves(&sleeves);

    // Total assets = 20,000 cash + 80,000 * 0.99 conservative = 99,200 USDC
    let total_a = ctx.vault_client.total_assets();
    assert_eq!(total_a, 99_200_0000000);

    // Check Redemption Capacity:
    // Cash = 20,000. Floor = 10%.
    // max_instant = (20,000 * 10000 - 99,200 * 1000) / 9000
    // = (200,000,000 - 99,200,000) / 9000 = 100,800,000 / 9000 = 11,200 USDC
    let cap = ctx.vault_client.get_redemption_capacity();
    assert_eq!(cap.instant, 11_200_0000000);
    assert_eq!(cap.in_24h, 79_200_0000000); // 80,000 * 0.99

    // Deposit 1,000 USDC by Alice
    ctx.numeraire_admin.mint(&ctx.alice, &1000_0000000);
    ctx.vault_client.deposit(&1000_0000000, &ctx.alice);

    // Alice redeems within allowable instant capacity: 500 shares (~496 USDC < 11,200 USDC)
    let alice_bal = ctx.vault_client.balance(&ctx.alice);
    let shares_to_redeem = alice_bal / 2;
    let redeemed = ctx.vault_client.redeem(&shares_to_redeem, &ctx.alice, &ctx.alice);
    assert!(redeemed > 0);

    // Attempt redemption that would breach the 10% liquidity floor:
    // Admin has ~100,000 shares. Trying to redeem 15,000 shares (~14,880 USDC > 11,200 max instant capacity)
    let res = ctx.vault_client.try_redeem(&15_000_0000000, &ctx.admin, &ctx.admin);
    assert_eq!(res.unwrap_err().unwrap(), VaultError::LiquidityFloorBreached.into());

    // Attempt redemption that exceeds total cash physical balance ($21,000 - $496 = $20,504):
    // Trying to redeem 30,000 shares (~29,760 USDC > 20,504 cash)
    let res_cap = ctx.vault_client.try_redeem(&30_000_0000000, &ctx.admin, &ctx.admin);
    assert_eq!(res_cap.unwrap_err().unwrap(), VaultError::ExceedsRedemptionCapacity.into());
}

// -------------------------------------------------------------------------
// Edge Cases & Robustness
// -------------------------------------------------------------------------
#[test]
fn test_edge_cases_and_authorizations() {
    let ctx = setup_test_context();

    // 1. Double initialization rejected
    let config = ctx.vault_client.get_config();
    let reinit_res = ctx.vault_client.try_initialize(
        &config,
        &String::from_str(&ctx.env, "New Name"),
        &String::from_str(&ctx.env, "NEW"),
        &7,
    );
    assert_eq!(
        reinit_res.unwrap_err().unwrap(),
        VaultError::AlreadyInitialized.into()
    );

    // 2. Closed universe check: adding unregistered asset in set_sleeves fails
    let unregistered_asset = Address::generate(&ctx.env);
    let mut sleeves = SdkVec::new(&ctx.env);
    sleeves.push_back(SleeveState {
        asset_id: String::from_str(&ctx.env, "USDC"),
        contract_address: ctx.numeraire_address.clone(),
        target_weight_bps: 5000,
        band_bps: 0,
        current_balance: 0,
        sleeve_type: SleeveType::CashStable,
        liquidity_tier: 0,
    });
    sleeves.push_back(SleeveState {
        asset_id: String::from_str(&ctx.env, "UNREG"),
        contract_address: unregistered_asset,
        target_weight_bps: 5000,
        band_bps: 0,
        current_balance: 0,
        sleeve_type: SleeveType::YieldDollar,
        liquidity_tier: 1,
    });
    let unreg_res = ctx.vault_client.try_set_sleeves(&sleeves);
    assert_eq!(
        unreg_res.unwrap_err().unwrap(),
        VaultError::AssetNotRegistered.into()
    );

    // 3. Sleeve weights must sum to 10000
    let mut invalid_sleeves = SdkVec::new(&ctx.env);
    invalid_sleeves.push_back(SleeveState {
        asset_id: String::from_str(&ctx.env, "USDC"),
        contract_address: ctx.numeraire_address.clone(),
        target_weight_bps: 9000, // only 90%
        band_bps: 0,
        current_balance: 0,
        sleeve_type: SleeveType::CashStable,
        liquidity_tier: 0,
    });
    let weights_res = ctx.vault_client.try_set_sleeves(&invalid_sleeves);
    assert_eq!(weights_res.unwrap_err().unwrap(), VaultError::InvalidBps.into());

    // 4. Zero amount operations rejected
    let zero_dep = ctx.vault_client.try_deposit(&0, &ctx.alice);
    assert_eq!(zero_dep.unwrap_err().unwrap(), VaultError::ZeroAmount.into());

    let zero_redeem = ctx.vault_client.try_redeem(&0, &ctx.alice, &ctx.alice);
    assert_eq!(zero_redeem.unwrap_err().unwrap(), VaultError::ZeroAmount.into());

    let zero_transfer = ctx.vault_client.try_transfer(&ctx.alice, &ctx.bob, &0);
    assert_eq!(zero_transfer.unwrap_err().unwrap(), VaultError::ZeroAmount.into());
}
