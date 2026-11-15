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
struct TestContext<'a> {
    env: Env,
    admin: Address,
    policy_owner: Address,
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
    let policy_owner = Address::generate(&env);
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
    let registry_id = env.register(AssetRegistry, (admin.clone(),));
    let registry_client = AssetRegistryClient::new(&env, &registry_id);

    // Register USDC in registry as Cash
    let mut usdc_sources = SdkVec::new(&env);
    usdc_sources.push_back(Symbol::new(&env, "USDC_SRC"));
    registry_client.add_asset(&AssetEntry {
        contract_address: numeraire_address.clone(),
        asset_id: Symbol::new(&env, "USDC"),
        issuer: Symbol::new(&env, "CIRCLE"),
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

    // Deploy Oracle
    let oracle_id = env.register(NavOracle, (admin.clone(), numeraire_address.clone(), 7u32, 300u32));
    let oracle_client = NavOracleClient::new(&env, &oracle_id);
    let policy_id = env.register(PolicyContract, (admin.clone(), oracle_id.clone()));
    let policy_client = PolicyContractClient::new(&env, &policy_id);

    // Deploy Vault (configured by its constructor, atomically - audit M3)
    let config = VaultConfig {
        admin: admin.clone(),
        pause_authority: pause_authority.clone(),
        numeraire: numeraire_address.clone(),
        policy_contract: policy_id.clone(),
        oracle_contract: oracle_id.clone(),
        registry_contract: registry_id.clone(),
        treasury: treasury.clone(),
        management_fee_bps: 100, // 1% per year
        tvl_cap: 1_000_000 * 10_000_000, // FIXTURE: ROADMAP 3.2 $1M beta cap
        per_user_cap: 100_000 * 10_000_000, // FIXTURE: design-partner ticket size (GTM)
    };

    let vault_id = env.register(
        StrategyVault,
        (
            config,
            String::from_str(&env, "Constella Strategy Vault"),
            String::from_str(&env, "CONST"),
            7u32,
        ),
    );
    let vault_client = StrategyVaultClient::new(&env, &vault_id);

    TestContext {
        env,
        admin,
        policy_owner,
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


// -------------------------------------------------------------------------
// T40: SEP-56 Deposit & Mint with Median NAV Valuation
// -------------------------------------------------------------------------
#[test]
fn test_t40_deposit_and_mint_median_nav() {
    let ctx = setup_test_context();

    // Verify Oracle Dual NAV
    let dep_nav = ctx.vault_client.get_nav_deposit().unwrap();
    let cons_nav = ctx.vault_client.get_nav_conservative().unwrap();
    // Audit V5: share prices come from holdings. An empty vault prices shares at
    // exactly $1.00 for both NAVs (no oracle override).
    assert_eq!(dep_nav.price, 10_000_000);
    assert_eq!(cons_nav.price, 10_000_000);

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

    // Deposit 1000 USDC by Alice at $1.00 NAV
    ctx.numeraire_admin.mint(&ctx.alice, &1000_0000000);
    ctx.vault_client.deposit(&1000_0000000, &ctx.alice);
    assert_eq!(ctx.vault_client.balance(&ctx.alice), 1000_0000000);

    // Verify Invariant: conservative_nav <= deposit_nav
    let cons_nav = ctx.vault_client.get_nav_conservative().unwrap();
    let dep_nav = ctx.vault_client.get_nav_deposit().unwrap();
    assert!(cons_nav.price <= dep_nav.price);
    // Audit V5: the vault holds only USDC, so a share is worth exactly $1.00.
    // (Previously an oracle override of $0.99 short-changed redeemers.)
    assert_eq!(cons_nav.price, 10_000_000);

    // Alice redeems 100 shares for Bob: 100 shares * $1.00 = 100 USDC
    let assets_out = ctx.vault_client.redeem(&100_0000000, &ctx.bob, &ctx.alice);
    assert_eq!(assets_out, 100_0000000);
    assert_eq!(ctx.numeraire_client.balance(&ctx.bob), 100_0000000);
    assert_eq!(ctx.vault_client.balance(&ctx.alice), 900_0000000);
    assert_eq!(ctx.vault_client.total_supply(), 900_0000000);

    // Alice withdraws 99 USDC directly: burns exactly 99 shares at $1.00
    let shares_burned = ctx.vault_client.withdraw(&99_0000000, &ctx.alice, &ctx.alice);
    assert_eq!(shares_burned, 99_0000000);
    assert_eq!(ctx.vault_client.balance(&ctx.alice), 801_0000000);
    assert_eq!(ctx.vault_client.total_supply(), 801_0000000);
    assert_eq!(ctx.numeraire_client.balance(&ctx.alice), 99_0000000);
}

// -------------------------------------------------------------------------
// T42: Fee Dilution Calculation Minting Shares to Treasury Over Time
// -------------------------------------------------------------------------
#[test]
fn test_t42_fee_dilution_calculation() {
    let ctx = setup_test_context();

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

    // Deposit initial capital
    ctx.numeraire_admin.mint(&ctx.alice, &500_0000000);
    ctx.vault_client.deposit(&500_0000000, &ctx.alice);

    // 1. Dual NAV
    let cons_nav = ctx.vault_client.get_nav_conservative().unwrap();
    let dep_nav = ctx.vault_client.get_nav_deposit().unwrap();
    // Audit V5: cash-only holdings -> both NAVs are exactly $1.00.
    assert_eq!(cons_nav.price, 10_000_000);
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
    // Audit W7: 100% USDC is a Pillar 3 "High-Cash Basket": 90% LTV / 10% haircut
    // (previously a hard-coded 98% / 2% for every basket).
    assert_eq!(risk.recommended_ltv_bps, 9000);
    assert_eq!(risk.recommended_haircut_bps, 1000);
    assert_eq!(risk.weighted_avg_redemption_hours, 0); // cash instant

    // 4. Liquidator Eligibility
    assert!(ctx.vault_client.is_eligible_liquidator(&ctx.alice));
    assert!(ctx.vault_client.is_eligible_liquidator(&ctx.bob));

    // 5. Listing Kit
    let kit = ctx.vault_client.get_listing_kit();
    assert_eq!(kit.share_token, ctx.vault_id);
    assert_eq!(kit.numeraire, ctx.numeraire_address);
    assert_eq!(kit.recommended_ltv_bps, 9000);
    assert_eq!(kit.recommended_haircut_bps, 1000);
    assert_eq!(kit.liquidation_threshold_bps, 9000); // = LTV (Blend single collateral factor)

    // 6. Oracle Status: reports the sources behind the vault's real sleeves
    // (a cash-only vault needs none: the numeraire is the unit of account).
    assert!(ctx.vault_client.get_oracle_status().is_empty());
    let gov = register_gov_fund(&ctx);
    let mut sleeves = SdkVec::new(&ctx.env);
    sleeves.push_back(sleeve(&ctx, "USDC", &ctx.numeraire_address, SleeveType::CashStable, 5000, 500_0000000));
    sleeves.push_back(sleeve(&ctx, "SPIKO_USD", &gov, SleeveType::UsdGovFunds, 5000, 0));
    ctx.vault_client.set_sleeves(&sleeves);
    let oracle_status = ctx.vault_client.get_oracle_status();
    let ids: std::vec::Vec<Symbol> = oracle_status.iter().map(|s| s.source_id).collect();
    assert!(ids.contains(&Symbol::new(&ctx.env, "GOV_NAV")));
    assert!(ids.contains(&Symbol::new(&ctx.env, "GOV_REFLECTOR")));
    for src in oracle_status.iter() {
        assert_eq!(src.price, 10_000_000);
    }
}

// -------------------------------------------------------------------------
// T44: Emergency Pause & Unpause Authorization Checks
// -------------------------------------------------------------------------
#[test]
fn test_t44_emergency_pause_and_unpause() {
    let ctx = setup_test_context();

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

    // Admin direct pause
    ctx.vault_client.admin_pause();
    assert!(ctx.vault_client.is_paused());
    ctx.vault_client.unpause();
    assert!(!ctx.vault_client.is_paused());

    // Update pause authority
    let new_pause_auth = Address::generate(&ctx.env);
    ctx.vault_client.set_pause_authority(&new_pause_auth);
    assert_eq!(ctx.vault_client.get_config().pause_authority, new_pause_auth);
    ctx.vault_client.pause();
    assert!(ctx.vault_client.is_paused());
    ctx.vault_client.unpause();
    assert!(!ctx.vault_client.is_paused());
}

// -------------------------------------------------------------------------
// T45: SEP-41 Token Standard Conformance
// -------------------------------------------------------------------------
#[test]
fn test_t45_sep41_token_standard() {
    let ctx = setup_test_context();

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
#[ignore = "audit V2: fabricated sleeve holdings via set_sleeves; re-enable when W1 (policy-validated rebalance execution) can acquire real holdings"]
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
        issuer: Symbol::new(&ctx.env, "FRANKLIN_TEMPLETON"),
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
        &ctx.policy_owner,
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

    // Verify oracle status returns BENJI sources via registry lookup
    let oracle_status = ctx.vault_client.get_oracle_status();
    let mut found_benji = false;
    for src in oracle_status.iter() {
        if src.source_id == Symbol::new(&ctx.env, "BENJI_MEDIAN") {
            found_benji = true;
        }
    }
    assert!(found_benji);
}

// -------------------------------------------------------------------------
// Edge Cases & Robustness
// -------------------------------------------------------------------------
#[test]
fn test_edge_cases_and_authorizations() {
    let ctx = setup_test_context();

    // 1. Audit M3: configuration is fixed at deployment; there is no
    //    `initialize` entrypoint a stranger could use to replace it.
    let config = ctx.vault_client.get_config();
    let mut hostile = config.clone();
    hostile.admin = ctx.bob.clone();
    let r = ctx.env.try_invoke_contract::<(), soroban_sdk::Error>(
        &ctx.vault_id,
        &Symbol::new(&ctx.env, "initialize"),
        soroban_sdk::IntoVal::into_val(&(hostile, String::from_str(&ctx.env, "Evil"), String::from_str(&ctx.env, "EVIL"), 7u32), &ctx.env),
    );
    assert!(!matches!(r, Ok(Ok(()))));
    assert_eq!(ctx.vault_client.get_config(), config);

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

// -------------------------------------------------------------------------
// Boundary & Dual NAV Edge Cases
// -------------------------------------------------------------------------
#[test]
#[ignore = "audit V2: fabricated sleeve holdings via set_sleeves; re-enable when W1 (policy-validated rebalance execution) can acquire real holdings"]
fn test_exact_liquidity_floor_boundary() {
    let ctx = setup_test_context();

    let benji_admin = Address::generate(&ctx.env);
    let benji_sac = ctx.env.register_stellar_asset_contract_v2(benji_admin);
    let benji_address = benji_sac.address();

    let mut benji_sources = SdkVec::new(&ctx.env);
    benji_sources.push_back(Symbol::new(&ctx.env, "BENJI_MEDIAN"));
    ctx.registry_client.add_asset(&AssetEntry {
        contract_address: benji_address.clone(),
        asset_id: Symbol::new(&ctx.env, "BENJI"),
        issuer: Symbol::new(&ctx.env, "FRANKLIN_TEMPLETON"),
        sleeve_type: RegSleeveType::UsdGovFund,
        liquidity_tier: 1,
        permissionless: true,
        oracle_sources: benji_sources,
        eligible_jurisdictions: SdkVec::new(&ctx.env),
        blocked_jurisdictions: SdkVec::new(&ctx.env),
        investor_classes_required: SdkVec::new(&ctx.env),
        is_active: true,
        added_at: 1_000_000,
    });

    let gov_sym = Symbol::new(&ctx.env, "USD_GOV_FUNDS");
    ctx.oracle_client.configure_sleeve(&gov_sym, &300, &1);
    ctx.oracle_client.register_source(&ExtOracleSource {
        source_id: Symbol::new(&ctx.env, "BENJI_MEDIAN"),
        asset: benji_address.clone(),
        sleeve: gov_sym,
        price: 10_000_000, // $1.00
        timestamp: 1_000_000,
        staleness_limit_seconds: 14400,
        min_liquidity_depth_usd: 100_000,
        current_liquidity_depth_usd: 200_000,
        is_active: true,
    });

    // 10% liquidity floor (1000 bps)
    let dummy_hash = soroban_sdk::BytesN::from_array(&ctx.env, &[7u8; 32]);
    let mut policy_targets = SdkVec::new(&ctx.env);
    policy_targets.push_back(SleeveTarget {
        asset_id: Symbol::new(&ctx.env, "USDC"),
        target_weight_bps: 1000,
        min_band_bps: 500,
        max_band_bps: 1500,
        sleeve_type: Symbol::new(&ctx.env, "cash"),
    });
    policy_targets.push_back(SleeveTarget {
        asset_id: Symbol::new(&ctx.env, "BENJI"),
        target_weight_bps: 9000,
        min_band_bps: 8500,
        max_band_bps: 9500,
        sleeve_type: Symbol::new(&ctx.env, "usd_gov_fund"),
    });
    ctx.policy_client.register_policy(
        &ctx.vault_id,
        &ctx.policy_owner,
        &dummy_hash,
        &ctx.registry_client.address,
        &1000, // 10% floor
        &9000,
        &9000,
        &50,
        &200,
        &policy_targets,
    );

    // Seed vault with $10,000 cash and $90,000 BENJI = $100,000 total assets
    // Cash is exactly 10,000 / 100,000 = 10.00%
    ctx.numeraire_admin.mint(&ctx.admin, &100_000_0000000);
    ctx.vault_client.deposit(&100_000_0000000, &ctx.admin);

    let mut sleeves = SdkVec::new(&ctx.env);
    sleeves.push_back(SleeveState {
        asset_id: String::from_str(&ctx.env, "USDC"),
        contract_address: ctx.numeraire_address.clone(),
        target_weight_bps: 1000,
        band_bps: 200,
        current_balance: 10_000_0000000, // Exactly 10%
        sleeve_type: SleeveType::CashStable,
        liquidity_tier: 0,
    });
    sleeves.push_back(SleeveState {
        asset_id: String::from_str(&ctx.env, "BENJI"),
        contract_address: benji_address,
        target_weight_bps: 9000,
        band_bps: 200,
        current_balance: 90_000_0000000, // 90%
        sleeve_type: SleeveType::UsdGovFunds,
        liquidity_tier: 1,
    });
    ctx.vault_client.set_sleeves(&sleeves);

    // Exact boundary assertions:
    let cap = ctx.vault_client.get_redemption_capacity();
    assert_eq!(cap.instant, 0); // Zero instant capacity because floor is reached exactly!
    assert_eq!(ctx.vault_client.max_withdraw(&ctx.admin), 0);
    assert_eq!(ctx.vault_client.max_redeem(&ctx.admin), 0);

    // Any withdraw or redeem attempt must be rejected with LiquidityFloorBreached
    let res = ctx.vault_client.try_withdraw(&1_0000000, &ctx.admin, &ctx.admin);
    assert_eq!(res.unwrap_err().unwrap(), VaultError::LiquidityFloorBreached.into());

    let res_redeem = ctx.vault_client.try_redeem(&1_0000000, &ctx.admin, &ctx.admin);
    assert_eq!(res_redeem.unwrap_err().unwrap(), VaultError::LiquidityFloorBreached.into());
}

#[test]
#[ignore = "audit V2: fabricated sleeve holdings via set_sleeves; re-enable when W1 (policy-validated rebalance execution) can acquire real holdings"]
fn test_dual_nav_asymmetric_max_redeem() {
    let ctx = setup_test_context();

    // Deploy an RWA token with 50% conservative haircut vs median
    let benji_admin = Address::generate(&ctx.env);
    let benji_sac = ctx.env.register_stellar_asset_contract_v2(benji_admin);
    let benji_address = benji_sac.address();

    let mut benji_sources = SdkVec::new(&ctx.env);
    benji_sources.push_back(Symbol::new(&ctx.env, "BENJI_MED"));
    ctx.registry_client.add_asset(&AssetEntry {
        contract_address: benji_address.clone(),
        asset_id: Symbol::new(&ctx.env, "BENJI"),
        issuer: Symbol::new(&ctx.env, "FRANKLIN_TEMPLETON"),
        sleeve_type: RegSleeveType::UsdGovFund,
        liquidity_tier: 1,
        permissionless: true,
        oracle_sources: benji_sources,
        eligible_jurisdictions: SdkVec::new(&ctx.env),
        blocked_jurisdictions: SdkVec::new(&ctx.env),
        investor_classes_required: SdkVec::new(&ctx.env),
        is_active: true,
        added_at: 1_000_000,
    });

    let gov_sym = Symbol::new(&ctx.env, "USD_GOV_FUNDS");
    ctx.oracle_client.configure_sleeve(&gov_sym, &10000, &2);
    // Median = $1.00 (10_000_000), Conservative = $0.50 (5_000_000)
    ctx.oracle_client.register_source(&ExtOracleSource {
        source_id: Symbol::new(&ctx.env, "BENJI_MED"),
        asset: benji_address.clone(),
        sleeve: gov_sym.clone(),
        price: 10_000_000,
        timestamp: 1_000_000,
        staleness_limit_seconds: 14400,
        min_liquidity_depth_usd: 100_000,
        current_liquidity_depth_usd: 200_000,
        is_active: true,
    });
    ctx.oracle_client.register_source(&ExtOracleSource {
        source_id: Symbol::new(&ctx.env, "BENJI_LOW"),
        asset: benji_address.clone(),
        sleeve: gov_sym,
        price: 5_000_000,
        timestamp: 1_000_000,
        staleness_limit_seconds: 14400,
        min_liquidity_depth_usd: 100_000,
        current_liquidity_depth_usd: 200_000,
        is_active: true,
    });

    // Policy: 0% floor to isolate capacity check
    let dummy_hash = soroban_sdk::BytesN::from_array(&ctx.env, &[7u8; 32]);
    let mut policy_targets = SdkVec::new(&ctx.env);
    policy_targets.push_back(SleeveTarget {
        asset_id: Symbol::new(&ctx.env, "USDC"),
        target_weight_bps: 5000,
        min_band_bps: 0,
        max_band_bps: 10000,
        sleeve_type: Symbol::new(&ctx.env, "cash"),
    });
    policy_targets.push_back(SleeveTarget {
        asset_id: Symbol::new(&ctx.env, "BENJI"),
        target_weight_bps: 5000,
        min_band_bps: 0,
        max_band_bps: 10000,
        sleeve_type: Symbol::new(&ctx.env, "usd_gov_fund"),
    });
    ctx.policy_client.register_policy(
        &ctx.vault_id,
        &ctx.policy_owner,
        &dummy_hash,
        &ctx.registry_client.address,
        &0, // 0% floor
        &10000,
        &10000,
        &50,
        &200,
        &policy_targets,
    );

    // Initial deposit: 1000 USDC -> 1000 shares
    ctx.numeraire_admin.mint(&ctx.alice, &1000_0000000);
    ctx.vault_client.deposit(&1000_0000000, &ctx.alice);

    // Configure sleeves: 500 USDC cash, 500 BENJI
    let mut sleeves = SdkVec::new(&ctx.env);
    sleeves.push_back(SleeveState {
        asset_id: String::from_str(&ctx.env, "USDC"),
        contract_address: ctx.numeraire_address.clone(),
        target_weight_bps: 5000,
        band_bps: 200,
        current_balance: 500_0000000,
        sleeve_type: SleeveType::CashStable,
        liquidity_tier: 0,
    });
    sleeves.push_back(SleeveState {
        asset_id: String::from_str(&ctx.env, "BENJI"),
        contract_address: benji_address,
        target_weight_bps: 5000,
        band_bps: 200,
        current_balance: 500_0000000,
        sleeve_type: SleeveType::UsdGovFunds,
        liquidity_tier: 1,
    });
    ctx.vault_client.set_sleeves(&sleeves);

    // Conservative NAV: (500 + 500 * 0.50) / 1000 = 0.750 ($0.75)
    let cons_nav = ctx.vault_client.get_nav_conservative().unwrap();
    assert_eq!(cons_nav.price, 7_500_000); // 0.75

    // Cash = 500 USDC. Instant capacity = 500 USDC.
    let cap = ctx.vault_client.get_redemption_capacity();
    assert_eq!(cap.instant, 500_0000000);

    // Shares needed to redeem 500 USDC at 0.75 conservative NAV:
    // 500 / 0.75 = 666.6666666 shares
    let max_red = ctx.vault_client.max_redeem(&ctx.alice);
    assert_eq!(max_red, 666_6666666);

    // Alice redeems exactly max_redeem shares
    let assets_out = ctx.vault_client.redeem(&max_red, &ctx.alice, &ctx.alice);
    assert!(assets_out <= cap.instant);
    assert_eq!(assets_out, 499_9999999);
}

#[test]
fn test_mint_ceiling_rounding() {
    let ctx = setup_test_context();

    ctx.numeraire_admin.mint(&ctx.alice, &100_0000000);

    // When NAV is $1.00 (10_000_000), minting 1 share requires (1 * 10_000_000 + 10_000_000 - 1) / 10_000_000 = 1 stroop of assets
    let assets_paid = ctx.vault_client.mint(&1, &ctx.alice);
    assert_eq!(assets_paid, 1);
    assert_eq!(ctx.vault_client.balance(&ctx.alice), 1);
}

#[test]
fn test_storage_cleanup_zero_balance_and_allowance() {
    let ctx = setup_test_context();

    ctx.numeraire_admin.mint(&ctx.alice, &100_0000000);
    ctx.vault_client.deposit(&100_0000000, &ctx.alice);
    assert_eq!(ctx.vault_client.balance(&ctx.alice), 100_0000000);

    // Transfer entire balance to Bob
    ctx.vault_client.transfer(&ctx.alice, &ctx.bob, &100_0000000);
    assert_eq!(ctx.vault_client.balance(&ctx.alice), 0);
    assert_eq!(ctx.vault_client.balance(&ctx.bob), 100_0000000);

    // Approve Bob 50, then revoke to 0
    ctx.vault_client.approve(&ctx.bob, &ctx.alice, &50_0000000, &200);
    assert_eq!(ctx.vault_client.allowance(&ctx.bob, &ctx.alice), 50_0000000);
    ctx.vault_client.approve(&ctx.bob, &ctx.alice, &0, &200);
    assert_eq!(ctx.vault_client.allowance(&ctx.bob, &ctx.alice), 0);

    // Approve and transfer_from to zero
    ctx.vault_client.approve(&ctx.bob, &ctx.alice, &50_0000000, &200);
    ctx.vault_client.transfer_from(&ctx.alice, &ctx.bob, &ctx.alice, &50_0000000);
    assert_eq!(ctx.vault_client.allowance(&ctx.bob, &ctx.alice), 0);
}

#[test]
fn test_upgrade_and_policy_hash() {
    let ctx = setup_test_context();

    // Verify policy hash query matches policy contract
    let dummy_hash = soroban_sdk::BytesN::from_array(&ctx.env, &[42u8; 32]);
    let mut targets = SdkVec::new(&ctx.env);
    targets.push_back(SleeveTarget {
        asset_id: Symbol::new(&ctx.env, "USDC"),
        target_weight_bps: 10000,
        min_band_bps: 0,
        max_band_bps: 10000,
        sleeve_type: Symbol::new(&ctx.env, "cash"),
    });
    ctx.policy_client.register_policy(
        &ctx.vault_id,
        &ctx.policy_owner,
        &dummy_hash,
        &ctx.registry_client.address,
        &1000,
        &10000,
        &10000,
        &50,
        &200,
        &targets,
    );
    let hash_opt = ctx.vault_client.get_policy_hash();
    assert_eq!(hash_opt, Some(dummy_hash));
}

// -------------------------------------------------------------------------
// Audit V1 (2026-11-15): self-transfer must never change balances or supply.
// Auth for the call under test is mocked EXPLICITLY (not mock_all_auths) and
// the recorded authorization tree is asserted.
// -------------------------------------------------------------------------
fn sum_of_balances(ctx: &TestContext, holders: &[&Address]) -> i128 {
    holders.iter().map(|h| ctx.vault_client.balance(h)).sum()
}

#[test]
fn test_self_transfer_does_not_mint_shares() {
    use soroban_sdk::testutils::{MockAuth, MockAuthInvoke};
    use soroban_sdk::IntoVal;
    let ctx = setup_test_context();
    ctx.numeraire_admin.mint(&ctx.alice, &100_0000000);
    ctx.vault_client.deposit(&100_0000000, &ctx.alice);
    let before = ctx.vault_client.balance(&ctx.alice);
    let supply = ctx.vault_client.total_supply();

    for _ in 0..3 {
        ctx.env.mock_auths(&[MockAuth {
            address: &ctx.alice,
            invoke: &MockAuthInvoke {
                contract: &ctx.vault_id,
                fn_name: "transfer",
                args: (ctx.alice.clone(), ctx.alice.clone(), before).into_val(&ctx.env),
                sub_invokes: &[],
            },
        }]);
        ctx.vault_client.transfer(&ctx.alice, &ctx.alice, &before);
        let auths = ctx.env.auths();
        assert_eq!(auths.len(), 1);
        assert_eq!(auths[0].0, ctx.alice);
    }

    assert_eq!(ctx.vault_client.balance(&ctx.alice), before);
    assert_eq!(ctx.vault_client.total_supply(), supply);
    assert_eq!(sum_of_balances(&ctx, &[&ctx.alice, &ctx.treasury]), supply);
}

#[test]
fn test_self_transfer_more_than_balance_rejected() {
    use soroban_sdk::testutils::{MockAuth, MockAuthInvoke};
    use soroban_sdk::IntoVal;
    let ctx = setup_test_context();
    ctx.numeraire_admin.mint(&ctx.alice, &100_0000000);
    ctx.vault_client.deposit(&100_0000000, &ctx.alice);
    let bal = ctx.vault_client.balance(&ctx.alice);
    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.alice,
        invoke: &MockAuthInvoke {
            contract: &ctx.vault_id,
            fn_name: "transfer",
            args: (ctx.alice.clone(), ctx.alice.clone(), bal + 1).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);
    assert!(ctx.vault_client.try_transfer(&ctx.alice, &ctx.alice, &(bal + 1)).is_err());
    assert_eq!(ctx.vault_client.balance(&ctx.alice), bal);
}

#[test]
fn test_self_transfer_from_does_not_mint_shares() {
    use soroban_sdk::testutils::{MockAuth, MockAuthInvoke};
    use soroban_sdk::IntoVal;
    let ctx = setup_test_context();
    ctx.numeraire_admin.mint(&ctx.alice, &100_0000000);
    ctx.vault_client.deposit(&100_0000000, &ctx.alice);
    let bal = ctx.vault_client.balance(&ctx.alice);
    let supply = ctx.vault_client.total_supply();
    ctx.vault_client.approve(&ctx.alice, &ctx.bob, &bal, &1000);

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.bob,
        invoke: &MockAuthInvoke {
            contract: &ctx.vault_id,
            fn_name: "transfer_from",
            args: (ctx.bob.clone(), ctx.alice.clone(), ctx.alice.clone(), bal).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);
    ctx.vault_client.transfer_from(&ctx.bob, &ctx.alice, &ctx.alice, &bal);
    assert_eq!(ctx.env.auths()[0].0, ctx.bob);

    assert_eq!(ctx.vault_client.balance(&ctx.alice), bal);
    assert_eq!(ctx.vault_client.total_supply(), supply);
    // Allowance is still consumed: the spender used its approval.
    assert_eq!(ctx.vault_client.allowance(&ctx.alice, &ctx.bob), 0);
}

#[test]
fn test_transfer_conserves_total_supply() {
    let ctx = setup_test_context();
    ctx.numeraire_admin.mint(&ctx.alice, &100_0000000);
    ctx.vault_client.deposit(&100_0000000, &ctx.alice);
    let supply = ctx.vault_client.total_supply();
    ctx.vault_client.transfer(&ctx.alice, &ctx.bob, &40_0000000);
    ctx.vault_client.transfer(&ctx.bob, &ctx.alice, &15_0000000);
    assert_eq!(ctx.vault_client.balance(&ctx.alice), 75_0000000);
    assert_eq!(ctx.vault_client.balance(&ctx.bob), 25_0000000);
    assert_eq!(sum_of_balances(&ctx, &[&ctx.alice, &ctx.bob, &ctx.treasury]), supply);
}

// -------------------------------------------------------------------------
// Audit V2 (2026-11-15): admin must not be able to write sleeve holdings.
// Holdings are recorded only by value-moving flows (deposit/redeem) and are
// valued at min(recorded, real token balance held by the vault).
// -------------------------------------------------------------------------
fn register_gov_fund(ctx: &TestContext) -> Address {
    let gov_sac = ctx.env.register_stellar_asset_contract_v2(Address::generate(&ctx.env));
    let gov = gov_sac.address();
    let mut sources = SdkVec::new(&ctx.env);
    sources.push_back(Symbol::new(&ctx.env, "GOV_NAV"));
    sources.push_back(Symbol::new(&ctx.env, "GOV_REFLECTOR"));
    ctx.registry_client.add_asset(&AssetEntry {
        contract_address: gov.clone(),
        asset_id: Symbol::new(&ctx.env, "SPIKO_USD"),
        issuer: Symbol::new(&ctx.env, "SPIKO"),
        sleeve_type: RegSleeveType::UsdGovFund,
        liquidity_tier: 1,
        permissionless: true,
        oracle_sources: sources,
        eligible_jurisdictions: SdkVec::new(&ctx.env),
        blocked_jurisdictions: SdkVec::new(&ctx.env),
        investor_classes_required: SdkVec::new(&ctx.env),
        is_active: true,
        added_at: 1_000_000,
    });
    let gov_sym = Symbol::new(&ctx.env, "USD_GOV_FUNDS");
    ctx.oracle_client.configure_sleeve(&gov_sym, &200, &2);
    for id in ["GOV_NAV", "GOV_REFLECTOR"] {
        ctx.oracle_client.register_source(&ExtOracleSource {
            source_id: Symbol::new(&ctx.env, id),
            asset: gov.clone(),
            sleeve: gov_sym.clone(),
            price: 10_000_000,
            timestamp: 1_000_000,
            staleness_limit_seconds: 14400,
            min_liquidity_depth_usd: 100_000,
            current_liquidity_depth_usd: 200_000,
            is_active: true,
        });
    }
    gov
}

fn sleeve(ctx: &TestContext, id: &str, addr: &Address, st: SleeveType, bps: u32, bal: i128) -> SleeveState {
    SleeveState {
        asset_id: String::from_str(&ctx.env, id),
        contract_address: addr.clone(),
        target_weight_bps: bps,
        band_bps: 500,
        current_balance: bal,
        sleeve_type: st,
        liquidity_tier: if st == SleeveType::CashStable { 0 } else { 1 },
    }
}

#[test]
fn test_v2_set_sleeves_rejects_balance_injection() {
    let ctx = setup_test_context();
    ctx.numeraire_admin.mint(&ctx.alice, &100_0000000);
    ctx.vault_client.deposit(&100_0000000, &ctx.alice);
    let gov = register_gov_fund(&ctx);

    let mut sleeves = SdkVec::new(&ctx.env);
    sleeves.push_back(sleeve(&ctx, "USDC", &ctx.numeraire_address, SleeveType::CashStable, 5000, 1_000_000_0000000));
    sleeves.push_back(sleeve(&ctx, "SPIKO_USD", &gov, SleeveType::UsdGovFunds, 5000, 0));
    let res = ctx.vault_client.try_set_sleeves(&sleeves);
    assert_eq!(res.unwrap_err().unwrap(), VaultError::BalanceNotSettable.into());

    let mut sleeves = SdkVec::new(&ctx.env);
    sleeves.push_back(sleeve(&ctx, "USDC", &ctx.numeraire_address, SleeveType::CashStable, 5000, 100_0000000));
    sleeves.push_back(sleeve(&ctx, "SPIKO_USD", &gov, SleeveType::UsdGovFunds, 5000, 50_0000000));
    let res = ctx.vault_client.try_set_sleeves(&sleeves);
    assert_eq!(res.unwrap_err().unwrap(), VaultError::BalanceNotSettable.into());

    assert_eq!(ctx.vault_client.total_assets(), 100_0000000);
}

#[test]
fn test_v2_set_sleeves_preserves_recorded_balances() {
    let ctx = setup_test_context();
    ctx.numeraire_admin.mint(&ctx.alice, &100_0000000);
    ctx.vault_client.deposit(&100_0000000, &ctx.alice);
    let gov = register_gov_fund(&ctx);

    let mut sleeves = SdkVec::new(&ctx.env);
    sleeves.push_back(sleeve(&ctx, "USDC", &ctx.numeraire_address, SleeveType::CashStable, 3000, 100_0000000));
    sleeves.push_back(sleeve(&ctx, "SPIKO_USD", &gov, SleeveType::UsdGovFunds, 7000, 0));
    ctx.vault_client.set_sleeves(&sleeves);

    let stored = ctx.vault_client.get_sleeves();
    assert_eq!(stored.len(), 2);
    assert_eq!(stored.get(0).unwrap().current_balance, 100_0000000);
    assert_eq!(stored.get(0).unwrap().target_weight_bps, 3000);
    assert_eq!(ctx.vault_client.total_assets(), 100_0000000);
}

#[test]
fn test_v2_cash_sleeve_must_be_the_numeraire() {
    let ctx = setup_test_context();
    let gov = register_gov_fund(&ctx);
    let mut sleeves = SdkVec::new(&ctx.env);
    sleeves.push_back(sleeve(&ctx, "FAKE_CASH", &gov, SleeveType::CashStable, 10000, 0));
    let res = ctx.vault_client.try_set_sleeves(&sleeves);
    assert_eq!(res.unwrap_err().unwrap(), VaultError::InvalidSleeveConfig.into());
}

#[test]
fn test_v2_duplicate_sleeve_rejected() {
    let ctx = setup_test_context();
    let gov = register_gov_fund(&ctx);
    let mut sleeves = SdkVec::new(&ctx.env);
    sleeves.push_back(sleeve(&ctx, "USDC", &ctx.numeraire_address, SleeveType::CashStable, 4000, 0));
    sleeves.push_back(sleeve(&ctx, "SPIKO_USD", &gov, SleeveType::UsdGovFunds, 3000, 0));
    sleeves.push_back(sleeve(&ctx, "SPIKO_USD", &gov, SleeveType::UsdGovFunds, 3000, 0));
    let res = ctx.vault_client.try_set_sleeves(&sleeves);
    assert_eq!(res.unwrap_err().unwrap(), VaultError::InvalidSleeveConfig.into());
}

#[test]
fn test_v2_cannot_remove_sleeve_that_holds_tokens() {
    let ctx = setup_test_context();
    let gov = register_gov_fund(&ctx);
    let mut sleeves = SdkVec::new(&ctx.env);
    sleeves.push_back(sleeve(&ctx, "USDC", &ctx.numeraire_address, SleeveType::CashStable, 5000, 0));
    sleeves.push_back(sleeve(&ctx, "SPIKO_USD", &gov, SleeveType::UsdGovFunds, 5000, 0));
    ctx.vault_client.set_sleeves(&sleeves);

    // The vault really holds gov-fund tokens (e.g. sent to it on-chain).
    soroban_sdk::token::StellarAssetClient::new(&ctx.env, &gov).mint(&ctx.vault_id, &10_0000000);

    let mut cash_only = SdkVec::new(&ctx.env);
    cash_only.push_back(sleeve(&ctx, "USDC", &ctx.numeraire_address, SleeveType::CashStable, 10000, 0));
    let res = ctx.vault_client.try_set_sleeves(&cash_only);
    assert_eq!(res.unwrap_err().unwrap(), VaultError::SleeveHasHoldings.into());
}

#[test]
fn test_v2_unsolicited_transfers_do_not_move_nav() {
    let ctx = setup_test_context();
    ctx.numeraire_admin.mint(&ctx.alice, &100_0000000);
    ctx.vault_client.deposit(&100_0000000, &ctx.alice);
    let gov = register_gov_fund(&ctx);
    let mut sleeves = SdkVec::new(&ctx.env);
    sleeves.push_back(sleeve(&ctx, "USDC", &ctx.numeraire_address, SleeveType::CashStable, 5000, 100_0000000));
    sleeves.push_back(sleeve(&ctx, "SPIKO_USD", &gov, SleeveType::UsdGovFunds, 5000, 0));
    ctx.vault_client.set_sleeves(&sleeves);
    let nav_before = ctx.vault_client.get_nav_deposit().unwrap().price;

    // Donations of numéraire and of a sleeve asset straight to the vault.
    ctx.numeraire_admin.mint(&ctx.vault_id, &1_000_0000000);
    soroban_sdk::token::StellarAssetClient::new(&ctx.env, &gov).mint(&ctx.vault_id, &1_000_0000000);

    assert_eq!(ctx.vault_client.total_assets(), 100_0000000);
    assert_eq!(ctx.vault_client.get_nav_deposit().unwrap().price, nav_before);
}

// -------------------------------------------------------------------------
// Audit P3 (2026-11-15): the vault binds its policy only with BOTH the vault
// admin and the policy owner signing; neither can do it alone.
// -------------------------------------------------------------------------
mod p3_bind {
    use super::*;
    use soroban_sdk::testutils::{MockAuth, MockAuthInvoke};
    use soroban_sdk::{BytesN, IntoVal};

    fn targets(env: &Env) -> SdkVec<PolicySleeveTargetRef> {
        let mut v = SdkVec::new(env);
        v.push_back(PolicySleeveTargetRef {
            asset_id: Symbol::new(env, "USDC"),
            target_weight_bps: 10000,
            min_band_bps: 9000,
            max_band_bps: 10000,
            sleeve_type: Symbol::new(env, "cash"),
        });
        v
    }

    fn try_bind(ctx: &TestContext, signers: &[&Address]) -> bool {
        let hash = BytesN::from_array(&ctx.env, &[0x5A; 32]);
        let t = targets(&ctx.env);
        let bind_args = (ctx.policy_owner.clone(), hash.clone(), 1000u32, 10000u32, 10000u32, 50u32, 500u32, t.clone())
            .into_val(&ctx.env);
        let reg_args = (
            ctx.vault_id.clone(), ctx.policy_owner.clone(), hash.clone(), ctx.registry_client.address.clone(),
            1000u32, 10000u32, 10000u32, 50u32, 500u32, t.clone(),
        )
            .into_val(&ctx.env);
        let bind = MockAuthInvoke { contract: &ctx.vault_id, fn_name: "bind_policy", args: bind_args, sub_invokes: &[] };
        let reg = MockAuthInvoke { contract: &ctx.policy_client.address, fn_name: "register_policy", args: reg_args, sub_invokes: &[] };
        let mut auths = std::vec::Vec::new();
        for s in signers {
            if **s == ctx.admin {
                auths.push(MockAuth { address: s, invoke: &bind });
            } else {
                auths.push(MockAuth { address: s, invoke: &reg });
            }
        }
        ctx.env.mock_auths(&auths);
        ctx.vault_client
            .try_bind_policy(&ctx.policy_owner, &hash, &1000, &10000, &10000, &50, &500, &t)
            .is_ok()
    }

    #[test]
    fn bind_requires_admin_and_owner() {
        let ctx = setup_test_context();
        assert!(!try_bind(&ctx, &[&ctx.admin]), "admin alone");
        assert!(!try_bind(&ctx, &[&ctx.policy_owner]), "owner alone");
        assert_eq!(ctx.vault_client.get_policy_hash(), None);

        assert!(try_bind(&ctx, &[&ctx.admin, &ctx.policy_owner]));
        let signers: std::vec::Vec<Address> = ctx.env.auths().iter().map(|a| a.0.clone()).collect();
        assert!(signers.contains(&ctx.admin) && signers.contains(&ctx.policy_owner));

        assert_eq!(ctx.vault_client.get_policy_hash(), Some(BytesN::from_array(&ctx.env, &[0x5A; 32])));
        let cfg = ctx.policy_client.get_policy_config(&ctx.vault_id).unwrap();
        assert_eq!(cfg.owner, ctx.policy_owner);
        assert_eq!(cfg.registry_address, ctx.registry_client.address);
    }

}

// -------------------------------------------------------------------------
// Audit V3 (2026-11-15): an unpriced sleeve that holds nothing must never
// halt the vault (NAV-ORACLE Rules 4/5: "unaffected sleeves continue").
// -------------------------------------------------------------------------
mod v3 {
    use super::*;

    fn vault_with_empty_gov_sleeve(ctx: &TestContext) -> Address {
        let gov = register_gov_fund(ctx);
        ctx.numeraire_admin.mint(&ctx.alice, &1000_0000000);
        ctx.vault_client.deposit(&500_0000000, &ctx.alice);
        let mut sleeves = SdkVec::new(&ctx.env);
        sleeves.push_back(sleeve(ctx, "USDC", &ctx.numeraire_address, SleeveType::CashStable, 5000, 500_0000000));
        sleeves.push_back(sleeve(ctx, "SPIKO_USD", &gov, SleeveType::UsdGovFunds, 5000, 0));
        ctx.vault_client.set_sleeves(&sleeves);
        gov
    }

    /// The vault must keep pricing, minting and paying out. Exact amounts are
    /// not hard-coded because the 1%/yr management fee legitimately dilutes
    /// NAV when ledger time advances; invariants are checked instead.
    fn assert_vault_operates(ctx: &TestContext) {
        let dep_nav = ctx.vault_client.get_nav_deposit().expect("deposit NAV").price;
        let cons_nav = ctx.vault_client.get_nav_conservative().expect("conservative NAV").price;
        // Only cash is held, so both NAVs equal total_assets / total_supply.
        assert_eq!(dep_nav, cons_nav);
        let expected = ctx.vault_client.total_assets() * 10_000_000 / ctx.vault_client.total_supply();
        assert!((dep_nav - expected).abs() <= 1, "nav {dep_nav} vs assets/supply {expected}");

        let usdc_before = ctx.numeraire_client.balance(&ctx.alice);
        let minted = ctx.vault_client.deposit(&100_0000000, &ctx.alice);
        assert!(minted > 0);
        let paid = ctx.vault_client.redeem(&50_0000000, &ctx.alice, &ctx.alice);
        assert!(paid > 0);
        let burned = ctx.vault_client.withdraw(&10_0000000, &ctx.alice, &ctx.alice);
        assert!(burned > 0);
        assert_eq!(
            ctx.numeraire_client.balance(&ctx.alice),
            usdc_before - 100_0000000 + paid + 10_0000000
        );
    }

    #[test]
    fn paused_empty_sleeve_does_not_halt_vault() {
        let ctx = setup_test_context();
        vault_with_empty_gov_sleeve(&ctx);
        let gov_sym = Symbol::new(&ctx.env, "USD_GOV_FUNDS");
        ctx.oracle_client.trigger_circuit_breaker(&ctx.admin, &gov_sym, &Symbol::new(&ctx.env, "MANUAL"));
        assert!(ctx.oracle_client.is_sleeve_paused(&gov_sym));
        assert_vault_operates(&ctx);
    }

    #[test]
    fn stale_empty_sleeve_does_not_halt_vault() {
        let ctx = setup_test_context();
        vault_with_empty_gov_sleeve(&ctx);
        // Gov-fund feeds have a 4h staleness limit; jump 1 day.
        ctx.env.ledger().set_timestamp(1_000_000 + 86_400);
        assert!(ctx.oracle_client.get_collateral_nav(&Symbol::new(&ctx.env, "USD_GOV_FUNDS")).is_none());
        assert_vault_operates(&ctx);
    }

    #[test]
    fn never_priced_empty_sleeve_does_not_halt_vault() {
        let ctx = setup_test_context();
        // Registered asset with NO oracle sources configured for its sleeve.
        let eur = ctx.env.register_stellar_asset_contract_v2(Address::generate(&ctx.env)).address();
        ctx.registry_client.add_asset(&AssetEntry {
            contract_address: eur.clone(),
            asset_id: Symbol::new(&ctx.env, "SPIKO_EUR"),
            issuer: Symbol::new(&ctx.env, "SPIKO"),
            sleeve_type: RegSleeveType::EurSovereign,
            liquidity_tier: 1,
            permissionless: true,
            oracle_sources: SdkVec::from_array(&ctx.env, [Symbol::new(&ctx.env, "NONE_YET")]),
            eligible_jurisdictions: SdkVec::new(&ctx.env),
            blocked_jurisdictions: SdkVec::new(&ctx.env),
            investor_classes_required: SdkVec::new(&ctx.env),
            is_active: true,
            added_at: 1_000_000,
        });
        ctx.numeraire_admin.mint(&ctx.alice, &1000_0000000);
        ctx.vault_client.deposit(&500_0000000, &ctx.alice);
        let mut sleeves = SdkVec::new(&ctx.env);
        sleeves.push_back(sleeve(&ctx, "USDC", &ctx.numeraire_address, SleeveType::CashStable, 8000, 500_0000000));
        sleeves.push_back(sleeve(&ctx, "SPIKO_EUR", &eur, SleeveType::EurSovereign, 2000, 0));
        ctx.vault_client.set_sleeves(&sleeves);
        assert_vault_operates(&ctx);
    }
}

// -------------------------------------------------------------------------
// Audit V4 (2026-11-15): every valuation path (share NAVs, total_assets,
// redemption capacity, risk sheet) must agree and must never substitute a
// $1.00 price for a missing oracle price. Funded non-cash sleeves cannot
// exist until W1, so these guard the reachable behaviour; the funded
// unpriced-sleeve hard-stop test ships with W1 (tracked in the audit).
// -------------------------------------------------------------------------
mod v4 {
    use super::*;

    fn vault_with_empty_gov(ctx: &TestContext) {
        let gov = register_gov_fund(ctx);
        ctx.numeraire_admin.mint(&ctx.alice, &1000_0000000);
        ctx.vault_client.deposit(&600_0000000, &ctx.alice);
        let mut sleeves = SdkVec::new(&ctx.env);
        sleeves.push_back(sleeve(ctx, "USDC", &ctx.numeraire_address, SleeveType::CashStable, 4000, 600_0000000));
        sleeves.push_back(sleeve(ctx, "SPIKO_USD", &gov, SleeveType::UsdGovFunds, 6000, 0));
        ctx.vault_client.set_sleeves(&sleeves);
    }

    fn assert_views_consistent(ctx: &TestContext) {
        let total = ctx.vault_client.total_assets();
        assert_eq!(total, 600_0000000, "only the real cash holdings count");
        let cons = ctx.vault_client.get_nav_conservative().unwrap().price;
        assert_eq!(cons * ctx.vault_client.total_supply() / 10_000_000, total);
        let cap = ctx.vault_client.get_redemption_capacity();
        assert_eq!((cap.in_24h, cap.in_1w, cap.longer), (0, 0, 0));
        assert!(cap.instant > 0 && cap.instant <= total);
        assert_eq!(ctx.vault_client.get_risk_sheet().weighted_avg_redemption_hours, 0);
    }

    #[test]
    fn views_agree_with_healthy_oracle() {
        let ctx = setup_test_context();
        vault_with_empty_gov(&ctx);
        assert_views_consistent(&ctx);
    }

    #[test]
    fn views_survive_paused_empty_sleeve() {
        let ctx = setup_test_context();
        vault_with_empty_gov(&ctx);
        ctx.oracle_client.trigger_circuit_breaker(&ctx.admin, &Symbol::new(&ctx.env, "USD_GOV_FUNDS"), &Symbol::new(&ctx.env, "MANUAL"));
        assert_views_consistent(&ctx);
    }

    #[test]
    fn views_survive_stale_empty_sleeve() {
        let ctx = setup_test_context();
        vault_with_empty_gov(&ctx);
        ctx.env.ledger().set_timestamp(1_000_000 + 86_400);
        ctx.vault_client.accrue_fees(); // fee dilution changes supply, not assets
        let total = ctx.vault_client.total_assets();
        assert_eq!(total, 600_0000000);
        let cons = ctx.vault_client.get_nav_conservative().unwrap().price;
        let supply = ctx.vault_client.total_supply();
        let implied = cons * supply / 10_000_000;
        // NAV/share is rounded DOWN to 7 decimals (favours the vault), so the
        // implied total can trail real assets by at most ~1 unit per whole share.
        assert!(implied <= total, "rounding must favour the vault: {implied} > {total}");
        assert!(total - implied <= supply / 10_000_000 + 1, "total {total} vs implied {implied}");
        assert!(ctx.vault_client.get_redemption_capacity().instant > 0);
    }
}

// -------------------------------------------------------------------------
// Audit W3 (2026-11-15): deposit caps (AGENT-RULES Rule 6) and a 48h upgrade
// timelock (CONTRACTS-SPEC). Admin calls under test use explicit auth.
// -------------------------------------------------------------------------
mod w3 {
    use super::*;
    use soroban_sdk::testutils::{MockAuth, MockAuthInvoke};
    use soroban_sdk::{BytesN, IntoVal};

    const ONE: i128 = 10_000_000;

    fn err(e: VaultError) -> soroban_sdk::Error {
        soroban_sdk::Error::from_contract_error(e as u32)
    }

    #[test]
    fn deployment_requires_sane_caps() {
        let ctx = setup_test_context();
        let mut cfg = ctx.vault_client.get_config();
        for (tvl, user) in [(0, 0), (100, 0), (0, 100), (100, 101)] {
            cfg.tvl_cap = tvl;
            cfg.per_user_cap = user;
            // With a constructor, invalid caps make the deployment itself fail.
            let env = Env::default();
            let cfg2 = cfg.clone();
            let deployed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                env.register(
                    StrategyVault,
                    (cfg2, String::from_str(&env, "V"), String::from_str(&env, "V"), 7u32),
                )
            }));
            assert!(deployed.is_err(), "tvl {tvl} user {user} must be rejected at deployment");
        }
    }

    #[test]
    fn per_user_cap_enforced_per_receiver() {
        let ctx = setup_test_context();
        ctx.numeraire_admin.mint(&ctx.alice, &(200_000 * ONE));
        ctx.numeraire_admin.mint(&ctx.bob, &(10 * ONE));
        // Feeds price the vault at $1.00 deposit NAV: 100k USDC = the cap.
        assert_eq!(ctx.vault_client.max_deposit(&ctx.alice), 100_000 * ONE);
        ctx.vault_client.deposit(&(100_000 * ONE), &ctx.alice);
        assert_eq!(ctx.vault_client.max_deposit(&ctx.alice), 0);
        assert_eq!(ctx.vault_client.try_deposit(&ONE, &ctx.alice).err().unwrap().unwrap(), err(VaultError::UserCapExceeded));
        assert_eq!(ctx.vault_client.try_mint(&ONE, &ctx.alice).err().unwrap().unwrap(), err(VaultError::UserCapExceeded));
        assert_eq!(ctx.vault_client.deposit(&(10 * ONE), &ctx.bob), 10 * ONE, "other users unaffected");
    }

    #[test]
    fn tvl_cap_enforced_across_users_and_max_views_are_exact() {
        let ctx = setup_test_context();
        ctx.vault_client.lower_caps(&(150 * ONE), &(100 * ONE));
        ctx.numeraire_admin.mint(&ctx.alice, &(1000 * ONE));
        ctx.numeraire_admin.mint(&ctx.bob, &(1000 * ONE));
        ctx.vault_client.deposit(&(100 * ONE), &ctx.alice);
        assert_eq!(ctx.vault_client.max_deposit(&ctx.bob), 50 * ONE, "TVL room binds before bob's user room");
        let m = ctx.vault_client.max_mint(&ctx.bob);
        ctx.vault_client.mint(&m, &ctx.bob);
        assert_eq!(ctx.vault_client.max_deposit(&ctx.bob), 0);
        assert_eq!(ctx.vault_client.try_deposit(&ONE, &ctx.bob).err().unwrap().unwrap(), err(VaultError::TvlCapExceeded));
        // Withdrawals reopen room.
        ctx.vault_client.redeem(&(10 * ONE), &ctx.alice, &ctx.alice);
        assert!(ctx.vault_client.max_deposit(&ctx.bob) > 0);
    }

    #[test]
    fn caps_can_only_be_lowered_and_only_by_admin() {
        let ctx = setup_test_context();
        let cfg = ctx.vault_client.get_config();
        assert_eq!(ctx.vault_client.try_lower_caps(&(cfg.tvl_cap + 1), &cfg.per_user_cap).err().unwrap().unwrap(), err(VaultError::InvalidCaps));
        assert_eq!(ctx.vault_client.try_lower_caps(&cfg.tvl_cap, &(cfg.per_user_cap + 1)).err().unwrap().unwrap(), err(VaultError::InvalidCaps));
        let stranger = Address::generate(&ctx.env);
        ctx.env.mock_auths(&[MockAuth {
            address: &stranger,
            invoke: &MockAuthInvoke { contract: &ctx.vault_id, fn_name: "lower_caps", args: (ONE, ONE).into_val(&ctx.env), sub_invokes: &[] },
        }]);
        assert!(ctx.vault_client.try_lower_caps(&ONE, &ONE).is_err());
        ctx.env.mock_auths(&[MockAuth {
            address: &ctx.admin,
            invoke: &MockAuthInvoke { contract: &ctx.vault_id, fn_name: "lower_caps", args: (500 * ONE, 50 * ONE).into_val(&ctx.env), sub_invokes: &[] },
        }]);
        ctx.vault_client.lower_caps(&(500 * ONE), &(50 * ONE));
        let after = ctx.vault_client.get_config();
        assert_eq!((after.tvl_cap, after.per_user_cap), (500 * ONE, 50 * ONE));
    }

    #[test]
    fn upgrade_waits_48h_and_can_be_cancelled() {
        let ctx = setup_test_context();
        let hash = BytesN::from_array(&ctx.env, &[0xAB; 32]);
        assert_eq!(ctx.vault_client.try_execute_upgrade().err().unwrap().unwrap(), err(VaultError::NoPendingUpgrade));

        ctx.vault_client.propose_upgrade(&hash);
        let p = ctx.vault_client.get_pending_upgrade().unwrap();
        assert_eq!(p.executable_at - p.proposed_at, UPGRADE_TIMELOCK_SECONDS);
        assert_eq!(UPGRADE_TIMELOCK_SECONDS, 48 * 3600);

        ctx.env.ledger().set_timestamp(p.executable_at - 1);
        assert_eq!(ctx.vault_client.try_execute_upgrade().err().unwrap().unwrap(), err(VaultError::UpgradeNotReady));

        ctx.vault_client.cancel_upgrade();
        assert!(ctx.vault_client.get_pending_upgrade().is_none());
        ctx.env.ledger().set_timestamp(p.executable_at + 1);
        assert_eq!(ctx.vault_client.try_execute_upgrade().err().unwrap().unwrap(), err(VaultError::NoPendingUpgrade));
    }

    #[test]
    fn timelock_gate_opens_exactly_at_48h() {
        let ctx = setup_test_context();
        // Hash of WASM never uploaded: once past the gate, the HOST rejects the
        // swap, which proves our timelock let it through (a full swap with a real
        // WASM is covered by the local-network e2e suite, not unit tests).
        let hash = BytesN::from_array(&ctx.env, &[0xCD; 32]);
        ctx.vault_client.propose_upgrade(&hash);
        let p = ctx.vault_client.get_pending_upgrade().unwrap();
        ctx.env.ledger().set_timestamp(p.executable_at);
        let r = ctx.vault_client.try_execute_upgrade();
        assert!(r.is_err());
        assert_ne!(r.err().unwrap().ok(), Some(err(VaultError::UpgradeNotReady)), "gate must be open at 48h");
        assert!(ctx.vault_client.get_pending_upgrade().is_some(), "failed swap reverts; proposal stays");
    }

    #[test]
    fn only_admin_can_propose_execute_or_cancel_upgrades() {
        let ctx = setup_test_context();
        let hash = BytesN::from_array(&ctx.env, &[0xEF; 32]);
        for who in [&ctx.pause_authority, &ctx.policy_owner, &ctx.alice] {
            ctx.env.mock_auths(&[MockAuth {
                address: who,
                invoke: &MockAuthInvoke { contract: &ctx.vault_id, fn_name: "propose_upgrade", args: (hash.clone(),).into_val(&ctx.env), sub_invokes: &[] },
            }]);
            assert!(ctx.vault_client.try_propose_upgrade(&hash).is_err());
        }
        ctx.env.mock_all_auths();
        ctx.vault_client.propose_upgrade(&hash);
        for f in ["cancel_upgrade", "execute_upgrade"] {
            ctx.env.mock_auths(&[MockAuth {
                address: &ctx.pause_authority,
                invoke: &MockAuthInvoke { contract: &ctx.vault_id, fn_name: f, args: ().into_val(&ctx.env), sub_invokes: &[] },
            }]);
            let r = if f == "cancel_upgrade" { ctx.vault_client.try_cancel_upgrade().is_err() } else { ctx.vault_client.try_execute_upgrade().is_err() };
            assert!(r, "{f} by pause authority must fail");
        }
        assert!(ctx.vault_client.get_pending_upgrade().is_some());
    }
}

// -------------------------------------------------------------------------
// Audit W7 (2026-11-15): Collateral Passport derived from the basket.
// Haircuts per COLLATERAL-PASSPORT Pillar 3; liquidators per Pillar 4.
// -------------------------------------------------------------------------
mod w7 {
    use super::*;

    fn asset(ctx: &TestContext, id: &str, st: RegSleeveType, permissionless: bool) -> Address {
        let addr = ctx.env.register_stellar_asset_contract_v2(Address::generate(&ctx.env)).address();
        ctx.registry_client.add_asset(&AssetEntry {
            contract_address: addr.clone(),
            asset_id: Symbol::new(&ctx.env, id),
            issuer: Symbol::new(&ctx.env, "TEST_ISSUER"),
            sleeve_type: st,
            liquidity_tier: 1,
            permissionless,
            oracle_sources: SdkVec::from_array(&ctx.env, [Symbol::new(&ctx.env, "S1")]),
            eligible_jurisdictions: SdkVec::new(&ctx.env),
            blocked_jurisdictions: SdkVec::new(&ctx.env),
            investor_classes_required: SdkVec::new(&ctx.env),
            is_active: true,
            added_at: 1_000_000,
        });
        addr
    }

    /// Empty vault with targets: cash `cash_bps`, other sleeve the remainder.
    fn targets(ctx: &TestContext, cash_bps: u32, other: &Address, id: &str, st: SleeveType) {
        let mut v = SdkVec::new(&ctx.env);
        v.push_back(sleeve(ctx, "USDC", &ctx.numeraire_address, SleeveType::CashStable, cash_bps, 0));
        v.push_back(sleeve(ctx, id, other, st, 10000 - cash_bps, 0));
        ctx.vault_client.set_sleeves(&v);
    }

    fn haircut(ctx: &TestContext) -> u32 {
        let r = ctx.vault_client.get_risk_sheet();
        assert_eq!(r.recommended_ltv_bps + r.recommended_haircut_bps, 10000);
        r.recommended_haircut_bps
    }

    #[test]
    fn pillar3_haircut_classes_from_target_weights() {
        let ctx = setup_test_context();
        let gov = asset(&ctx, "SPIKO_USD", RegSleeveType::UsdGovFund, true);
        targets(&ctx, 4000, &gov, "SPIKO_USD", SleeveType::UsdGovFunds);
        assert_eq!(haircut(&ctx), 1300); // 40% x 10% + 60% x 15%

        let ctx = setup_test_context();
        let blend = asset(&ctx, "BLEND_USDC_SUPPLY", RegSleeveType::Lending, true);
        targets(&ctx, 5000, &blend, "BLEND_USDC_SUPPLY", SleeveType::Lending);
        assert_eq!(haircut(&ctx), 2000); // 50% x 10% + 50% x 30%

        let ctx = setup_test_context();
        let ylds = asset(&ctx, "FIGURE_YLDS", RegSleeveType::YieldUsd, true);
        targets(&ctx, 5000, &ylds, "FIGURE_YLDS", SleeveType::YieldDollar);
        assert_eq!(haircut(&ctx), 1750); // 50% x 10% + 50% x 25%
    }

    #[test]
    fn unspecified_classes_carry_no_borrowing_power() {
        let ctx = setup_test_context();
        let jqqq = asset(&ctx, "JH_JQQQ", RegSleeveType::Equity, true);
        targets(&ctx, 5000, &jqqq, "JH_JQQQ", SleeveType::Equity);
        assert_eq!(haircut(&ctx), 5500); // 50% x 10% + 50% x 100%
    }

    #[test]
    fn haircut_rounds_up() {
        let ctx = setup_test_context();
        let gov = asset(&ctx, "SPIKO_USD", RegSleeveType::UsdGovFund, true);
        targets(&ctx, 3333, &gov, "SPIKO_USD", SleeveType::UsdGovFunds);
        assert_eq!(haircut(&ctx), 1334); // 1333.35 -> 1334
    }

    #[test]
    fn held_value_overrides_targets_once_funded() {
        let ctx = setup_test_context();
        let gov = asset(&ctx, "SPIKO_USD", RegSleeveType::UsdGovFund, true);
        targets(&ctx, 1000, &gov, "SPIKO_USD", SleeveType::UsdGovFunds);
        assert_eq!(haircut(&ctx), 1450); // target-weighted while empty
        ctx.numeraire_admin.mint(&ctx.alice, &(100 * 10_000_000));
        ctx.vault_client.deposit(&(100 * 10_000_000), &ctx.alice);
        assert_eq!(haircut(&ctx), 1000, "the vault actually holds only cash");
        assert_eq!(ctx.vault_client.get_listing_kit().liquidation_threshold_bps, 9000);
    }

    #[test]
    fn liquidators_open_only_for_all_permissionless_baskets() {
        let ctx = setup_test_context();
        let anyone = Address::generate(&ctx.env);
        assert!(ctx.vault_client.is_eligible_liquidator(&anyone), "USDC-only basket is open");

        let blend = asset(&ctx, "BLEND_USDC_SUPPLY", RegSleeveType::Lending, true);
        targets(&ctx, 5000, &blend, "BLEND_USDC_SUPPLY", SleeveType::Lending);
        assert!(ctx.vault_client.is_eligible_liquidator(&anyone), "all permissionless: open");

        let benji = asset(&ctx, "FRANKLIN_BENJI", RegSleeveType::UsdGovFund, false);
        targets(&ctx, 5000, &benji, "FRANKLIN_BENJI", SleeveType::UsdGovFunds);
        assert!(!ctx.vault_client.is_eligible_liquidator(&anyone), "permissioned constituent: gated");
        assert!(!ctx.vault_client.is_eligible_liquidator(&ctx.admin), "not even the admin until SEP-57 exists");
    }
}

// -------------------------------------------------------------------------
// Audit M4 (2026-11-15): sleeve labels come from the registry, never free text.
// -------------------------------------------------------------------------
mod m4 {
    use super::*;

    #[test]
    fn cash_sleeve_label_comes_from_registry() {
        let ctx = setup_test_context();
        let cash = ctx.vault_client.get_sleeves().get(0).unwrap();
        assert_eq!(cash.asset_id, String::from_str(&ctx.env, "USDC"));
        assert_eq!(cash.contract_address, ctx.numeraire_address);
    }

    #[test]
    fn set_sleeves_rejects_labels_that_do_not_match_the_registry() {
        let ctx = setup_test_context();
        let gov = register_gov_fund(&ctx); // registry id SPIKO_USD
        let mut mislabeled = SdkVec::new(&ctx.env);
        mislabeled.push_back(sleeve(&ctx, "USDC", &ctx.numeraire_address, SleeveType::CashStable, 5000, 0));
        mislabeled.push_back(sleeve(&ctx, "FRANKLIN_BENJI", &gov, SleeveType::UsdGovFunds, 5000, 0));
        assert_eq!(ctx.vault_client.try_set_sleeves(&mislabeled).err().unwrap().unwrap(), VaultError::InvalidSleeveConfig.into());

        let mut bad_cash = SdkVec::new(&ctx.env);
        bad_cash.push_back(sleeve(&ctx, "USDT", &ctx.numeraire_address, SleeveType::CashStable, 5000, 0));
        bad_cash.push_back(sleeve(&ctx, "SPIKO_USD", &gov, SleeveType::UsdGovFunds, 5000, 0));
        assert_eq!(ctx.vault_client.try_set_sleeves(&bad_cash).err().unwrap().unwrap(), VaultError::InvalidSleeveConfig.into());

        let mut ok = SdkVec::new(&ctx.env);
        ok.push_back(sleeve(&ctx, "USDC", &ctx.numeraire_address, SleeveType::CashStable, 5000, 0));
        ok.push_back(sleeve(&ctx, "SPIKO_USD", &gov, SleeveType::UsdGovFunds, 5000, 0));
        ctx.vault_client.set_sleeves(&ok);
        assert_eq!(ctx.vault_client.get_sleeves().get(1).unwrap().asset_id, String::from_str(&ctx.env, "SPIKO_USD"));
    }
}

// -------------------------------------------------------------------------
// Audit M2 (2026-11-15): event names follow AGENT-RULES Rule 8 -
// SCREAMING_SNAKE_CASE, except names defined by SEP-41 (transfer, approve,
// mint, burn) and SEP-56 (deposit, withdraw). SEP-56 topic layouts:
//   deposit:  [deposit, operator, from, receiver]   data (assets, shares)
//   withdraw: [withdraw, operator, receiver, owner] data (assets, shares)
// -------------------------------------------------------------------------
mod m2 {
    use super::*;
    use soroban_sdk::{FromVal, SymbolStr, TryFromVal, Val};

    const STANDARD: [&str; 6] = ["transfer", "approve", "mint", "burn", "deposit", "withdraw"];

    fn name_of(env: &Env, v: &Val) -> std::string::String {
        let sym = Symbol::try_from_val(env, v).expect("event topic 0 must be a symbol");
        let s = SymbolStr::from_val(env, &sym.to_symbol_val());
        let bytes: &[u8] = s.as_ref();
        std::string::String::from_utf8(bytes.to_vec()).unwrap()
    }

    /// Every event the vault emitted in the last call follows Rule 8.
    fn check_names(ctx: &TestContext) -> usize {
        let mut seen = 0;
        for (contract, topics, _) in all_events(&ctx.env).iter() {
            if contract != ctx.vault_id {
                continue;
            }
            let name = name_of(&ctx.env, &topics.get(0).unwrap());
            let ok = STANDARD.contains(&name.as_str())
                || name.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_');
            assert!(ok, "event `{name}` violates Rule 8");
            seen += 1;
        }
        seen
    }

    #[test]
    fn every_vault_event_follows_rule_8() {
        let ctx = setup_test_context();
        ctx.numeraire_admin.mint(&ctx.alice, &(1000 * 10_000_000));
        let mut total = 0;
        ctx.vault_client.deposit(&(500 * 10_000_000), &ctx.alice);
        total += check_names(&ctx);
        ctx.vault_client.transfer(&ctx.alice, &ctx.bob, &(10 * 10_000_000));
        total += check_names(&ctx);
        ctx.vault_client.approve(&ctx.alice, &ctx.bob, &(10 * 10_000_000), &500);
        total += check_names(&ctx);
        ctx.vault_client.redeem(&(10 * 10_000_000), &ctx.alice, &ctx.alice);
        total += check_names(&ctx);
        ctx.env.ledger().set_timestamp(1_000_000 + 30 * 86_400);
        ctx.vault_client.accrue_fees();
        total += check_names(&ctx);
        ctx.vault_client.pause();
        total += check_names(&ctx);
        ctx.vault_client.unpause();
        total += check_names(&ctx);
        let gov = register_gov_fund(&ctx);
        let mut sleeves = SdkVec::new(&ctx.env);
        let cash_bal = ctx.vault_client.get_sleeves().get(0).unwrap().current_balance;
        sleeves.push_back(sleeve(&ctx, "USDC", &ctx.numeraire_address, SleeveType::CashStable, 5000, cash_bal));
        sleeves.push_back(sleeve(&ctx, "SPIKO_USD", &gov, SleeveType::UsdGovFunds, 5000, 0));
        ctx.vault_client.set_sleeves(&sleeves);
        total += check_names(&ctx);
        assert!(total >= 8, "expected events to be checked, saw {total}");
    }

    #[test]
    fn deposit_and_withdraw_events_follow_sep56_layout() {
        let ctx = setup_test_context();
        ctx.numeraire_admin.mint(&ctx.alice, &(100 * 10_000_000));
        ctx.vault_client.deposit(&(100 * 10_000_000), &ctx.alice);
        let dep = all_events(&ctx.env).iter()
            .find(|(c, t, _)| *c == ctx.vault_id && name_of(&ctx.env, &t.get(0).unwrap()) == "deposit")
            .expect("deposit event");
        assert_eq!(dep.1.len(), 4, "deposit topics: [deposit, operator, from, receiver]");
        for i in 1..4 {
            assert_eq!(Address::try_from_val(&ctx.env, &dep.1.get(i).unwrap()).unwrap(), ctx.alice);
        }
        let (assets, shares): (i128, i128) = <(i128, i128)>::try_from_val(&ctx.env, &dep.2).unwrap();
        assert_eq!((assets, shares), (100 * 10_000_000, 100 * 10_000_000));

        ctx.vault_client.redeem(&(40 * 10_000_000), &ctx.bob, &ctx.alice);
        let wd = all_events(&ctx.env).iter()
            .find(|(c, t, _)| *c == ctx.vault_id && name_of(&ctx.env, &t.get(0).unwrap()) == "withdraw")
            .expect("withdraw event");
        assert_eq!(wd.1.len(), 4, "withdraw topics: [withdraw, operator, receiver, owner]");
        assert_eq!(Address::try_from_val(&ctx.env, &wd.1.get(1).unwrap()).unwrap(), ctx.alice, "operator");
        assert_eq!(Address::try_from_val(&ctx.env, &wd.1.get(2).unwrap()).unwrap(), ctx.bob, "receiver");
        assert_eq!(Address::try_from_val(&ctx.env, &wd.1.get(3).unwrap()).unwrap(), ctx.alice, "owner");
    }
}
