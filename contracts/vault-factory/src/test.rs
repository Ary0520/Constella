//! Factory tests on the REAL compiled vault WASM (Soroban's standard
//! cross-contract pattern: run `stellar contract build` before `cargo test`).
extern crate std;
use super::*;
use constella_nav_oracle::NavOracle;
use constella_policy::PolicyContract;
use constella_registry::{AssetEntry, AssetRegistry, AssetRegistryClient, SleeveType};
use constella_vault_strategy::{StrategyVault, StrategyVaultClient, VaultConfig};
use soroban_sdk::{
    testutils::{Address as _, Ledger, MockAuth, MockAuthInvoke},
    token, vec, Symbol,
};

const VAULT_WASM: &[u8] = include_bytes!("../../target/wasm32v1-none/release/constella_vault_strategy.wasm");
const ONE: i128 = 10_000_000;
const MAX_TVL: i128 = 1_000_000 * ONE; // ROADMAP 3.2 beta cap, Rule 6
const DEFAULT_FEE: u32 = 1_000; // D-D: 10 % of profit, Constella's fee for factory vaults

struct F {
    env: Env,
    factory: VaultFactoryClient<'static>,
    usdc: Address,
    usdc_admin: token::StellarAssetClient<'static>,
    registry: Address,
    tbill: Address,
    keeper: Address,
    router: Address,
}

fn setup() -> F {
    let env = Env::default();
    env.mock_all_auths();
    env.cost_estimate().budget().reset_unlimited();
    env.ledger().set_timestamp(1_731_600_000);
    let admin = Address::generate(&env);
    let usdc = env.register_stellar_asset_contract_v2(Address::generate(&env)).address();
    let registry = env.register(AssetRegistry, (admin.clone(),));
    AssetRegistryClient::new(&env, &registry).add_asset(&AssetEntry {
        contract_address: usdc.clone(),
        asset_id: Symbol::new(&env, "USDC"),
        issuer: Symbol::new(&env, "CIRCLE"),
        sleeve_type: SleeveType::Cash,
        liquidity_tier: 0,
        permissionless: true,
        oracle_sources: vec![&env, Symbol::new(&env, "USDC_BASE")],
        eligible_jurisdictions: vec![&env],
        blocked_jurisdictions: vec![&env],
        investor_classes_required: vec![&env],
        is_active: true,
        added_at: 0,
    });
    // A second, non-cash registry asset so a basket can hold more than USDC.
    let tbill = env.register_stellar_asset_contract_v2(Address::generate(&env)).address();
    AssetRegistryClient::new(&env, &registry).add_asset(&AssetEntry {
        contract_address: tbill.clone(),
        asset_id: Symbol::new(&env, "TBILL"),
        issuer: Symbol::new(&env, "FUNDCO"),
        sleeve_type: SleeveType::UsdGovFund,
        liquidity_tier: 1,
        permissionless: true,
        oracle_sources: vec![&env, Symbol::new(&env, "TBILL_A"), Symbol::new(&env, "TBILL_B")],
        eligible_jurisdictions: vec![&env],
        blocked_jurisdictions: vec![&env],
        investor_classes_required: vec![&env],
        is_active: true,
        added_at: 0,
    });
    let oracle = env.register(NavOracle, (admin.clone(), usdc.clone(), 7u32, 300u32));
    let policy = env.register(PolicyContract, (admin.clone(), oracle.clone()));
    let hash = env.deployer().upload_contract_wasm(VAULT_WASM);
    let treasury = Address::generate(&env);
    let keeper = Address::generate(&env);
    let router = Address::generate(&env);
    let factory = env.register(VaultFactory, (admin, hash, registry.clone(), oracle, policy, treasury, MAX_TVL, DEFAULT_FEE,
                                              keeper.clone(), Some(router.clone())));
    let usdc_admin = token::StellarAssetClient::new(&env, &usdc);
    F { env: env.clone(), factory: VaultFactoryClient::new(&env, &factory), usdc, usdc_admin, registry, tbill, keeper, router }
}

fn params(f: &F, owner: &Address, salt: u8) -> VaultParams {
    VaultParams {
        admin: owner.clone(),
        pause_authority: owner.clone(),
        numeraire: f.usdc.clone(),
        performance_fee_bps: 500,
        tvl_cap: 500_000 * ONE,
        per_user_cap: 500_000 * ONE,
        name: String::from_str(&f.env, "Acme Treasury"),
        symbol: String::from_str(&f.env, "ACME"),
        salt: BytesN::from_array(&f.env, &[salt; 32]),
    }
}

#[test]
fn a_treasury_creates_a_working_private_vault() {
    let f = setup();
    let treasury_owner = Address::generate(&f.env);
    let vault = f.factory.create_vault(&treasury_owner, &params(&f, &treasury_owner, 1));
    let v = StrategyVaultClient::new(&f.env, &vault);
    let s = f.factory.get_settings();
    // The creator's choices...
    let cfg: VaultConfig = v.get_config();
    assert_eq!((cfg.admin.clone(), cfg.performance_fee_bps, cfg.tvl_cap), (treasury_owner.clone(), 500, 500_000 * ONE));
    // ...and what the factory fixed: shared contracts and the fee treasury.
    assert_eq!((cfg.oracle_contract, cfg.registry_contract, cfg.policy_contract, cfg.treasury), (s.oracle, s.registry, s.policy, s.treasury));
    assert!(f.factory.is_vault(&vault));
    assert_eq!(f.factory.vault_creator(&vault), Some(treasury_owner.clone()));
    assert_eq!(f.factory.vaults_of(&treasury_owner), vec![&f.env, vault.clone()]);
    // It is a real, working vault.
    f.usdc_admin.mint(&treasury_owner, &(1_000 * ONE));
    assert_eq!(v.deposit(&(1_000 * ONE), &treasury_owner, &treasury_owner, &treasury_owner), 1_000 * ONE);
    assert_eq!(v.total_assets(), 1_000 * ONE);
}

#[test]
fn creators_stay_within_protocol_bounds() {
    let f = setup();
    let c = Address::generate(&f.env);
    let mut p = params(&f, &c, 2);
    p.performance_fee_bps = MAX_PERFORMANCE_FEE_BPS + 1;
    assert_eq!(f.factory.try_create_vault(&c, &p), Err(Ok(FactoryError::InvalidParams.into())));
    let mut p = params(&f, &c, 2);
    p.tvl_cap = MAX_TVL + 1;
    p.per_user_cap = MAX_TVL;
    assert_eq!(f.factory.try_create_vault(&c, &p), Err(Ok(FactoryError::TvlCapTooHigh.into())));
    let mut p = params(&f, &c, 2);
    p.per_user_cap = p.tvl_cap + 1;
    assert_eq!(f.factory.try_create_vault(&c, &p), Err(Ok(FactoryError::InvalidParams.into())));
    // A numéraire that is not a registered cash asset: the vault constructor refuses.
    let mut p = params(&f, &c, 2);
    p.numeraire = f.env.register_stellar_asset_contract_v2(Address::generate(&f.env)).address();
    assert!(f.factory.try_create_vault(&c, &p).is_err());
    assert!(f.factory.vaults_of(&c).is_empty());
}

#[test]
fn only_the_creator_can_create_in_their_name() {
    let f = setup();
    let victim = Address::generate(&f.env);
    let mallory = Address::generate(&f.env);
    let p = params(&f, &mallory, 3);
    f.env.mock_auths(&[MockAuth { address: &mallory, invoke: &MockAuthInvoke { contract: &f.factory.address,
        fn_name: "create_vault", args: (victim.clone(), p.clone()).into_val(&f.env), sub_invokes: &[] } }]);
    assert!(f.factory.try_create_vault(&victim, &p).is_err());
    assert!(f.factory.vaults_of(&victim).is_empty());
}

#[test]
fn vault_addresses_are_bound_to_their_creator() {
    let f = setup();
    let a = Address::generate(&f.env);
    let b = Address::generate(&f.env);
    let va = f.factory.create_vault(&a, &params(&f, &a, 4));
    // The same creator cannot reuse a salt; another creator with the same salt
    // gets a different address (no squatting / front-running).
    assert!(f.factory.try_create_vault(&a, &params(&f, &a, 4)).is_err());
    let vb = f.factory.create_vault(&b, &params(&f, &b, 4));
    assert_ne!(va, vb);
    assert_eq!(f.factory.vault_creator(&vb), Some(b));
}

#[test]
fn a_vault_not_deployed_by_the_factory_is_not_recognised() {
    let f = setup();
    let s = f.factory.get_settings();
    let owner = Address::generate(&f.env);
    let outside = f.env.register(StrategyVault, (VaultConfig {
        admin: owner.clone(), pause_authority: owner.clone(), numeraire: f.usdc.clone(), policy_contract: s.policy,
        oracle_contract: s.oracle, registry_contract: s.registry, treasury: owner, performance_fee_bps: 0,
        tvl_cap: ONE, per_user_cap: ONE }, String::from_str(&f.env, "X"), String::from_str(&f.env, "X"), 7u32));
    assert!(!f.factory.is_vault(&outside));
    assert!(!f.factory.is_vault(&f.registry));
}

#[test]
fn changing_the_vault_code_waits_48_hours_and_never_touches_existing_vaults() {
    let f = setup();
    let s = f.factory.get_settings();
    let c = Address::generate(&f.env);
    let existing = f.factory.create_vault(&c, &params(&f, &c, 5));
    let new_hash = BytesN::from_array(&f.env, &[0xAB; 32]);
    // A stranger cannot propose.
    let stranger = Address::generate(&f.env);
    f.env.mock_auths(&[MockAuth { address: &stranger, invoke: &MockAuthInvoke { contract: &f.factory.address,
        fn_name: "propose_vault_wasm", args: (new_hash.clone(),).into_val(&f.env), sub_invokes: &[] } }]);
    assert!(f.factory.try_propose_vault_wasm(&new_hash).is_err());
    f.env.mock_all_auths();
    f.factory.propose_vault_wasm(&new_hash);
    assert_eq!(f.factory.try_activate_vault_wasm(), Err(Ok(FactoryError::WasmChangeNotReady.into())));
    assert_eq!(f.factory.get_settings().vault_wasm_hash, s.vault_wasm_hash, "not yet");
    f.factory.cancel_vault_wasm();
    assert_eq!(f.factory.try_activate_vault_wasm(), Err(Ok(FactoryError::NoPendingWasm.into())));
    f.factory.propose_vault_wasm(&new_hash);
    f.env.ledger().set_timestamp(1_731_600_000 + WASM_CHANGE_DELAY_SECONDS);
    f.factory.activate_vault_wasm();
    assert_eq!(f.factory.get_settings().vault_wasm_hash, new_hash);
    // The existing vault keeps working on its code.
    f.usdc_admin.mint(&c, &ONE);
    assert_eq!(StrategyVaultClient::new(&f.env, &existing).deposit(&ONE, &c, &c, &c), ONE);
}

#[test]
fn each_creator_has_a_bounded_index() {
    let f = setup();
    let c = Address::generate(&f.env);
    for i in 0..MAX_VAULTS_PER_CREATOR {
        let mut p = params(&f, &c, 0);
        p.salt = BytesN::from_array(&f.env, &[(i % 256) as u8; 32]);
        let mut b = p.salt.to_array();
        b[31] = (i / 256) as u8;
        b[30] = 0x77;
        p.salt = BytesN::from_array(&f.env, &b);
        f.factory.create_vault(&c, &p);
    }
    let mut p = params(&f, &c, 0xEE);
    p.salt = BytesN::from_array(&f.env, &[0xEE; 32]);
    assert_eq!(f.factory.try_create_vault(&c, &p), Err(Ok(FactoryError::TooManyVaults.into())));
}

// -------------------------------------------------------------------------
// Decision D-O (24 Nov 2026): Constella's fee on a factory vault is set by
// Constella (the factory admin), never by the creator - the pattern of Yearn
// v3's factory-held protocol fee config with governance-set per-vault custom
// fees. A creator gets the factory default; any other rate (a negotiated
// Treasury-plan discount) needs the factory admin's co-signature. Changing
// the default affects future vaults only; a vault's fee is fixed for life.
// -------------------------------------------------------------------------
mod protocol_fee {
    use super::*;

    fn creator_only(f: &F, creator: &Address, p: &VaultParams) {
        f.env.mock_auths(&[MockAuth { address: creator, invoke: &MockAuthInvoke { contract: &f.factory.address,
            fn_name: "create_vault", args: (creator.clone(), p.clone()).into_val(&f.env), sub_invokes: &[] } }]);
    }

    #[test]
    fn a_creator_alone_gets_exactly_the_default_fee() {
        let f = setup();
        let c = Address::generate(&f.env);
        let mut p = params(&f, &c, 10);
        p.performance_fee_bps = DEFAULT_FEE;
        creator_only(&f, &c, &p);
        let v = f.factory.create_vault(&c, &p);
        assert_eq!(StrategyVaultClient::new(&f.env, &v).get_config().performance_fee_bps, DEFAULT_FEE);
    }

    #[test]
    fn a_creator_alone_cannot_choose_constellas_fee() {
        let f = setup();
        let c = Address::generate(&f.env);
        for fee in [0u32, 1, 500, DEFAULT_FEE - 1] {
            let mut p = params(&f, &c, 11);
            p.performance_fee_bps = fee;
            creator_only(&f, &c, &p);
            assert!(f.factory.try_create_vault(&c, &p).is_err(), "creator set Constella's fee to {fee}");
        }
        assert!(f.factory.vaults_of(&c).is_empty());
    }

    #[test]
    fn a_custom_fee_needs_the_factory_admins_cosignature() {
        let f = setup();
        let admin = f.factory.get_settings().admin;
        let c = Address::generate(&f.env);
        let mut p = params(&f, &c, 12);
        p.performance_fee_bps = 500; // Treasury plan: 5 % of profit
        f.env.mock_auths(&[
            MockAuth { address: &c, invoke: &MockAuthInvoke { contract: &f.factory.address,
                fn_name: "create_vault", args: (c.clone(), p.clone()).into_val(&f.env), sub_invokes: &[] } },
            MockAuth { address: &admin, invoke: &MockAuthInvoke { contract: &f.factory.address,
                fn_name: "create_vault", args: (c.clone(), p.clone()).into_val(&f.env), sub_invokes: &[] } },
        ]);
        let v = f.factory.create_vault(&c, &p);
        assert_eq!(StrategyVaultClient::new(&f.env, &v).get_config().performance_fee_bps, 500);
    }

    #[test]
    fn the_default_changes_only_future_vaults_and_only_by_the_admin() {
        let f = setup();
        let c = Address::generate(&f.env);
        let mut p = params(&f, &c, 13);
        p.performance_fee_bps = DEFAULT_FEE;
        let before = f.factory.create_vault(&c, &p);
        // A stranger cannot change it.
        let stranger = Address::generate(&f.env);
        f.env.mock_auths(&[MockAuth { address: &stranger, invoke: &MockAuthInvoke { contract: &f.factory.address,
            fn_name: "set_default_fee", args: (700u32,).into_val(&f.env), sub_invokes: &[] } }]);
        assert!(f.factory.try_set_default_fee(&700).is_err());
        f.env.mock_all_auths();
        // Never above the D-D ceiling.
        assert_eq!(f.factory.try_set_default_fee(&(MAX_PERFORMANCE_FEE_BPS + 1)), Err(Ok(FactoryError::InvalidParams.into())));
        f.factory.set_default_fee(&700);
        assert_eq!(f.factory.get_settings().default_fee_bps, 700);
        // Existing vault unchanged; a new creator-only vault gets the new default.
        assert_eq!(StrategyVaultClient::new(&f.env, &before).get_config().performance_fee_bps, DEFAULT_FEE);
        let mut q = params(&f, &c, 14);
        q.performance_fee_bps = 700;
        creator_only(&f, &c, &q);
        let after = f.factory.create_vault(&c, &q);
        assert_eq!(StrategyVaultClient::new(&f.env, &after).get_config().performance_fee_bps, 700);
    }

    #[test]
    #[should_panic]
    fn the_constructor_refuses_a_default_above_the_ceiling() {
        let f = setup();
        let s = f.factory.get_settings();
        f.env.register(VaultFactory, (s.admin, s.vault_wasm_hash, s.registry, s.oracle, s.policy, s.treasury,
                                      MAX_TVL, MAX_PERFORMANCE_FEE_BPS + 1, s.keeper, s.router));
    }
}

// -------------------------------------------------------------------------
// Step 3: a global, append-only index so services (keeper, relayer, console)
// can find every factory vault without scanning events.
// -------------------------------------------------------------------------
mod index {
    use super::*;

    #[test]
    fn every_factory_vault_is_indexed_in_creation_order() {
        let f = setup();
        assert_eq!(f.factory.vault_count(), 0);
        assert_eq!(f.factory.vault_at(&0), None);
        let (a, b) = (Address::generate(&f.env), Address::generate(&f.env));
        let mut p = params(&f, &a, 20);
        p.performance_fee_bps = DEFAULT_FEE;
        let va = f.factory.create_vault(&a, &p);
        let mut q = params(&f, &b, 21);
        q.performance_fee_bps = DEFAULT_FEE;
        let vb = f.factory.create_vault(&b, &q);
        assert_eq!(f.factory.vault_count(), 2);
        assert_eq!(f.factory.vault_at(&0), Some(va));
        assert_eq!(f.factory.vault_at(&1), Some(vb));
        assert_eq!(f.factory.vault_at(&2), None);
    }

    #[test]
    fn a_failed_creation_is_not_indexed() {
        let f = setup();
        let c = Address::generate(&f.env);
        let mut p = params(&f, &c, 22);
        p.tvl_cap = MAX_TVL + 1;
        assert!(f.factory.try_create_vault(&c, &p).is_err());
        assert_eq!(f.factory.vault_count(), 0);
    }
}

// -------------------------------------------------------------------------
// Step 3: a signed thesis becomes the creator's own live vault in ONE
// transaction under ONE signature: deploy, bind the policy (creator = owner),
// sleeves from the registry, Constella's keeper and router.
// -------------------------------------------------------------------------
mod with_policy {
    use super::*;
    use constella_policy::PolicyContractClient;
    use constella_vault_strategy::TradeRoute;
    use soroban_sdk::testutils::AuthorizedFunction;

    fn target(f: &F, id: &str, w: u32, lo: u32, hi: u32, kind: &str) -> PolicySleeveTarget {
        PolicySleeveTarget { asset_id: Symbol::new(&f.env, id), target_weight_bps: w, min_band_bps: lo, max_band_bps: hi,
                             sleeve_type: Symbol::new(&f.env, kind) }
    }

    fn terms(f: &F) -> PolicyTerms {
        PolicyTerms {
            policy_hash: BytesN::from_array(&f.env, &[7; 32]),
            min_liquidity_sleeve_bps: 1000,
            max_single_asset_bps: 2500,
            max_issuer_bps: 3000,
            max_slippage_bps: 50,
            rebalance_band_bps: 200,
            sleeves: vec![&f.env, target(f, "USDC", 8500, 8300, 8700, "numeraire"), target(f, "TBILL", 1500, 1300, 1700, "usd_gov_fund")],
        }
    }

    fn own(f: &F, c: &Address, salt: u8) -> VaultParams {
        let mut p = params(f, c, salt);
        p.performance_fee_bps = DEFAULT_FEE;
        p
    }

    #[test]
    fn one_call_creates_a_fully_configured_vault_owned_by_the_creator() {
        let f = setup();
        let c = Address::generate(&f.env);
        let t = terms(&f);
        let vault = f.factory.create_vault_with_policy(&c, &own(&f, &c, 30), &t);
        let v = StrategyVaultClient::new(&f.env, &vault);
        // Policy bound: the creator is the owner, the signed hash and terms are on chain.
        let policy = PolicyContractClient::new(&f.env, &f.factory.get_settings().policy);
        let cfg = policy.get_policy_config(&vault).unwrap();
        assert_eq!((cfg.owner.clone(), cfg.policy_hash.clone()), (c.clone(), t.policy_hash.clone()));
        assert_eq!((cfg.min_liquidity_sleeve_bps, cfg.rebalance_band_bps), (1000, 200));
        // Sleeves built from the registry, equal to the signed targets.
        let sl = v.get_sleeves();
        assert_eq!(sl.len(), 2);
        let usdc = sl.iter().find(|s| s.contract_address == f.usdc).unwrap();
        let tbill = sl.iter().find(|s| s.contract_address == f.tbill).unwrap();
        assert_eq!((usdc.target_weight_bps, tbill.target_weight_bps), (8500, 1500));
        assert_eq!(tbill.liquidity_tier, 1, "liquidity tier from the registry");
        assert_eq!(usdc.asset_id, String::from_str(&f.env, "USDC"));
        // Constella's keeper and router; the creator is admin and pause authority.
        assert_eq!(v.get_keeper(), Some(f.keeper.clone()));
        assert_eq!(v.get_router(&TradeRoute::Soroswap), Some(f.router.clone()));
        let vc = v.get_config();
        assert_eq!((vc.admin.clone(), vc.pause_authority.clone()), (c.clone(), c.clone()));
        // Indexed and usable at once.
        assert_eq!(f.factory.vault_at(&0), Some(vault.clone()));
        assert!(f.factory.is_vault(&vault));
        f.usdc_admin.mint(&c, &(100 * ONE));
        assert_eq!(v.deposit(&(100 * ONE), &c, &c, &c), 100 * ONE);
    }

    #[test]
    fn the_creator_signs_once_for_every_step() {
        let f = setup();
        let c = Address::generate(&f.env);
        let vault = f.factory.create_vault_with_policy(&c, &own(&f, &c, 31), &terms(&f));
        let auths = f.env.auths();
        let mine: std::vec::Vec<_> = auths.iter().filter(|(a, _)| *a == c).collect();
        assert_eq!(mine.len(), 1, "one authorization tree for the creator");
        let root = &mine[0].1;
        let name = |inv: &soroban_sdk::testutils::AuthorizedInvocation| match &inv.function {
            AuthorizedFunction::Contract((addr, f, _)) => (addr.clone(), f.clone()),
            _ => panic!("unexpected"),
        };
        let sym = |n: &str| Symbol::new(&f.env, n);
        assert_eq!(name(root), (f.factory.address.clone(), sym("create_vault_with_policy")));
        let subs: std::vec::Vec<_> = root.sub_invocations.iter().map(name).collect();
        for fname in ["bind_policy", "set_sleeves", "set_keeper", "propose_router"] {
            assert!(subs.contains(&(vault.clone(), sym(fname))), "{fname} covered by the one signature");
        }
        let bind = root.sub_invocations.iter().find(|s| name(s).1 == sym("bind_policy")).unwrap();
        assert!(bind.sub_invocations.iter().any(|s| name(s).1 == sym("register_policy")), "owner consent inside the same tree");
    }

    #[test]
    fn the_creator_must_be_the_vaults_admin_and_pause_authority() {
        let f = setup();
        let c = Address::generate(&f.env);
        let other = Address::generate(&f.env);
        let mut p = own(&f, &c, 32);
        p.admin = other.clone();
        assert_eq!(f.factory.try_create_vault_with_policy(&c, &p, &terms(&f)), Err(Ok(FactoryError::InvalidParams.into())));
        let mut p = own(&f, &c, 32);
        p.pause_authority = other;
        assert_eq!(f.factory.try_create_vault_with_policy(&c, &p, &terms(&f)), Err(Ok(FactoryError::InvalidParams.into())));
        assert_eq!(f.factory.vault_count(), 0);
    }

    #[test]
    fn sleeves_must_be_active_registry_assets() {
        let f = setup();
        let c = Address::generate(&f.env);
        let mut t = terms(&f);
        t.sleeves = vec![&f.env, target(&f, "USDC", 8500, 8300, 8700, "numeraire"), target(&f, "NOPE", 1500, 1300, 1700, "lending")];
        assert_eq!(f.factory.try_create_vault_with_policy(&c, &own(&f, &c, 33), &t), Err(Ok(FactoryError::AssetNotRegistered.into())));
        assert_eq!(f.factory.vault_count(), 0);
    }

    #[test]
    fn a_deactivated_registry_asset_is_refused_by_the_factory() {
        let f = setup();
        let c = Address::generate(&f.env);
        AssetRegistryClient::new(&f.env, &f.registry).deactivate_asset(&Symbol::new(&f.env, "TBILL"));
        assert_eq!(f.factory.try_create_vault_with_policy(&c, &own(&f, &c, 38), &terms(&f)),
                   Err(Ok(FactoryError::AssetNotRegistered.into())));
        assert_eq!(f.factory.vault_count(), 0);
    }

    #[test]
    fn a_policy_the_vault_rejects_rolls_everything_back() {
        let f = setup();
        let c = Address::generate(&f.env);
        let mut t = terms(&f);
        // Targets that do not sum to 100 %: the vault's own set_sleeves refuses.
        t.sleeves = vec![&f.env, target(&f, "USDC", 8000, 7800, 8200, "numeraire"), target(&f, "TBILL", 1500, 1300, 1700, "usd_gov_fund")];
        assert!(f.factory.try_create_vault_with_policy(&c, &own(&f, &c, 34), &t).is_err());
        assert_eq!(f.factory.vault_count(), 0);
        assert!(f.factory.vaults_of(&c).is_empty());
    }

    #[test]
    fn the_fee_rule_and_creator_authorization_still_apply() {
        let f = setup();
        let c = Address::generate(&f.env);
        let mut p = own(&f, &c, 35);
        p.performance_fee_bps = 0;
        let t = terms(&f);
        f.env.mock_auths(&[MockAuth { address: &c, invoke: &MockAuthInvoke { contract: &f.factory.address,
            fn_name: "create_vault_with_policy", args: (c.clone(), p.clone(), t.clone()).into_val(&f.env), sub_invokes: &[] } }]);
        assert!(f.factory.try_create_vault_with_policy(&c, &p, &t).is_err(), "creator alone cannot set Constella's fee");
        // A stranger cannot create a vault in someone else's name.
        let mallory = Address::generate(&f.env);
        let p = own(&f, &c, 36);
        f.env.mock_auths(&[MockAuth { address: &mallory, invoke: &MockAuthInvoke { contract: &f.factory.address,
            fn_name: "create_vault_with_policy", args: (c.clone(), p.clone(), t.clone()).into_val(&f.env), sub_invokes: &[] } }]);
        assert!(f.factory.try_create_vault_with_policy(&c, &p, &t).is_err());
    }

    #[test]
    fn the_whole_setup_fits_the_network_limits() {
        // Testnet limits read 24 Nov 2026 (`stellar network settings`):
        // 400M instructions, 40 MiB memory per transaction.
        let f = setup();
        let c = Address::generate(&f.env);
        let p = own(&f, &c, 37);
        let t = terms(&f);
        f.env.cost_estimate().budget().reset_default();
        f.factory.create_vault_with_policy(&c, &p, &t);
        let b = f.env.cost_estimate().budget();
        assert!(b.cpu_instruction_cost() < 400_000_000, "cpu {}", b.cpu_instruction_cost());
        assert!(b.memory_bytes_cost() < 41_943_040, "mem {}", b.memory_bytes_cost());
    }
}
