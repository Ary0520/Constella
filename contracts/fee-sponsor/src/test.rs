#![cfg(test)]

extern crate std;

use super::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    Address, Env, Symbol,
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


#[test]
fn test_t52_sponsorship_whitelist() {
    let env = Env::default();

    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let vault_contract = Address::generate(&env);
    let foreign_contract = Address::generate(&env);

    // Initialize sponsor contract with 10 ops/hour, 50 ops/day, 1,000,000 stroops daily budget
    let contract_id = env.register(FeeSponsorContract, (admin.clone(), 10u32, 50u32, 1_000_000u64));
    let client = FeeSponsorContractClient::new(&env, &contract_id);

    // Verify initial config
    let config = client.get_config();
    assert_eq!(config.admin, admin);
    assert_eq!(config.max_ops_per_hour, 10);
    assert_eq!(config.max_ops_per_day, 50);
    assert_eq!(config.daily_budget_stroops, 1_000_000);
    assert!(!config.is_paused);

    // Whitelist vault contract using admin auth
    env.mock_all_auths();
    assert!(!client.is_whitelisted(&vault_contract));
    client.whitelist_contract(&vault_contract);
    assert!(client.is_whitelisted(&vault_contract));

    // Whitelisted contract passes sponsorship validation
    let valid_res = client.try_validate_sponsorship(&user, &vault_contract, &5000);
    assert_eq!(valid_res, Ok(Ok(())));

    // Foreign / unapproved contract fails validation with TargetNotWhitelisted
    let foreign_res = client.try_validate_sponsorship(&user, &foreign_contract, &5000);
    assert_eq!(foreign_res, Err(Ok(SponsorError::TargetNotWhitelisted)));

    // Adding already whitelisted contract fails with ContractAlreadyWhitelisted
    let dup_res = client.try_whitelist_contract(&vault_contract);
    assert_eq!(dup_res, Err(Ok(SponsorError::ContractAlreadyWhitelisted)));

    // Removing contract
    client.remove_contract(&vault_contract);
    assert!(!client.is_whitelisted(&vault_contract));

    // Calls to removed contract now fail
    let removed_res = client.try_validate_sponsorship(&user, &vault_contract, &5000);
    assert_eq!(removed_res, Err(Ok(SponsorError::TargetNotWhitelisted)));

    // Removing un-whitelisted contract fails
    let rem_unwhitelisted = client.try_remove_contract(&vault_contract);
    assert_eq!(
        rem_unwhitelisted,
        Err(Ok(SponsorError::ContractNotWhitelisted))
    );
}

#[test]
fn test_t53_sponsorship_rate_limits() {
    let env = Env::default();

    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let vault_contract = Address::generate(&env);

    // Set tight rate limits: 3 ops/hour, 5 ops/day, 1,000,000 stroops budget
    let contract_id = env.register(FeeSponsorContract, (admin.clone(), 3u32, 5u32, 1_000_000u64));
    let client = FeeSponsorContractClient::new(&env, &contract_id);

    env.mock_all_auths();
    client.whitelist_contract(&vault_contract);

    // Set starting timestamp (e.g. 100,000 seconds)
    let start_ts: u64 = 100_000;
    env.ledger().set_timestamp(start_ts);

    // Operation 1
    assert_eq!(
        client.try_validate_sponsorship(&user, &vault_contract, &1000),
        Ok(Ok(()))
    );
    let relayer = Address::generate(&env);
    client.add_relayer(&relayer);
    client.record_sponsored_op(&relayer, &user, &vault_contract, &1000);

    // Operation 2 (same hour)
    assert_eq!(
        client.try_validate_sponsorship(&user, &vault_contract, &1000),
        Ok(Ok(()))
    );
    client.record_sponsored_op(&relayer, &user, &vault_contract, &1000);

    // Operation 3 (same hour)
    assert_eq!(
        client.try_validate_sponsorship(&user, &vault_contract, &1000),
        Ok(Ok(()))
    );
    client.record_sponsored_op(&relayer, &user, &vault_contract, &1000);

    // Check user stats: hourly_ops == 3, daily_ops == 3
    let stats = client.get_user_stats(&user);
    assert_eq!(stats.hourly_ops, 3);
    assert_eq!(stats.daily_ops, 3);

    // Operation 4 in same hour: HourlyLimitExceeded!
    let hourly_breach = client.try_validate_sponsorship(&user, &vault_contract, &1000);
    assert_eq!(hourly_breach, Err(Ok(SponsorError::HourlyLimitExceeded)));

    // Advance timestamp by 1 hour (3600 seconds)
    env.ledger().set_timestamp(start_ts + 3600);

    // Operation 4 succeeds in the new hour!
    assert_eq!(
        client.try_validate_sponsorship(&user, &vault_contract, &1000),
        Ok(Ok(()))
    );
    client.record_sponsored_op(&relayer, &user, &vault_contract, &1000);

    // Operation 5 succeeds in the new hour! (Daily ops now reaches 5)
    assert_eq!(
        client.try_validate_sponsorship(&user, &vault_contract, &1000),
        Ok(Ok(()))
    );
    client.record_sponsored_op(&relayer, &user, &vault_contract, &1000);

    // Daily limit reached (5 ops/day). 6th operation fails with DailyLimitExceeded!
    let daily_breach = client.try_validate_sponsorship(&user, &vault_contract, &1000);
    assert_eq!(daily_breach, Err(Ok(SponsorError::DailyLimitExceeded)));

    // Advance to next day (+86,400 seconds)
    env.ledger().set_timestamp(start_ts + 86400);

    // Dynamic stats check: get_user_stats automatically resets effective hourly and daily ops on new day
    let rolled_stats = client.get_user_stats(&user);
    assert_eq!(rolled_stats.hourly_ops, 0);
    assert_eq!(rolled_stats.daily_ops, 0);
    assert_eq!(rolled_stats.last_op_timestamp, start_ts + 3600);

    // New day resets daily quota: operations succeed again!
    assert_eq!(
        client.try_validate_sponsorship(&user, &vault_contract, &1000),
        Ok(Ok(()))
    );
}

#[test]
fn test_t54_sponsorship_daily_budget() {
    let env = Env::default();

    let admin = Address::generate(&env);
    let user_1 = Address::generate(&env);
    let user_2 = Address::generate(&env);
    let vault_contract = Address::generate(&env);

    // Set daily budget: 50,000 stroops (e.g. 0.005 XLM)
    let contract_id = env.register(FeeSponsorContract, (admin.clone(), 100u32, 200u32, 50_000u64));
    let client = FeeSponsorContractClient::new(&env, &contract_id);

    env.mock_all_auths();
    client.whitelist_contract(&vault_contract);

    let start_ts: u64 = 200_000;
    env.ledger().set_timestamp(start_ts);

    // User 1 executes op consuming 30,000 stroops
    assert_eq!(
        client.try_validate_sponsorship(&user_1, &vault_contract, &30_000),
        Ok(Ok(()))
    );
    let relayer = Address::generate(&env);
    client.add_relayer(&relayer);
    client.record_sponsored_op(&relayer, &user_1, &vault_contract, &30_000);

    let global_stats = client.get_global_stats();
    assert_eq!(global_stats.total_ops_today, 1);
    assert_eq!(global_stats.total_fees_today_stroops, 30_000);

    // User 2 attempts an op bidding 25,000 stroops (total 55,000 > 50,000 limit)
    let budget_breach = client.try_validate_sponsorship(&user_2, &vault_contract, &25_000);
    assert_eq!(budget_breach, Err(Ok(SponsorError::DailyBudgetExceeded)));

    // User 2 attempts an op bidding 20,000 stroops (total 50,000 <= 50,000 limit): Succeeded!
    assert_eq!(
        client.try_validate_sponsorship(&user_2, &vault_contract, &20_000),
        Ok(Ok(()))
    );
    client.record_sponsored_op(&relayer, &user_2, &vault_contract, &20_000);

    // Budget now completely saturated (50,000 / 50,000). Even 1 stroop rejected!
    let saturated_breach = client.try_validate_sponsorship(&user_1, &vault_contract, &1);
    assert_eq!(
        saturated_breach,
        Err(Ok(SponsorError::DailyBudgetExceeded))
    );

    // Emergency circuit breaker: Pause
    client.pause();
    assert!(client.get_config().is_paused);
    let paused_res = client.try_validate_sponsorship(&user_1, &vault_contract, &100);
    assert_eq!(paused_res, Err(Ok(SponsorError::SponsorPaused)));

    // Recording sponsored op when paused rejected
    let paused_rec = client.try_record_sponsored_op(&relayer, &user_1, &vault_contract, &100);
    assert_eq!(paused_rec, Err(Ok(SponsorError::SponsorPaused)));

    // Unpause
    client.unpause();
    assert!(!client.get_config().is_paused);

    // Advance to next day: Budget resets!
    let next_day_ts = start_ts + 86400;
    env.ledger().set_timestamp(next_day_ts);

    // Dynamic stats check: global stats reset for the new day
    let reset_global = client.get_global_stats();
    assert_eq!(reset_global.total_ops_today, 0);
    assert_eq!(reset_global.total_fees_today_stroops, 0);
    assert_eq!(reset_global.current_day_epoch, next_day_ts / 86400);

    assert_eq!(
        client.try_validate_sponsorship(&user_1, &vault_contract, &40_000),
        Ok(Ok(()))
    );
}

#[test]
fn test_fee_sponsor_admin_transfer_and_limits_reconfiguration() {
    let env = Env::default();

    let admin = Address::generate(&env);
    let new_admin = Address::generate(&env);
    let vault = Address::generate(&env);

    let contract_id = env.register(FeeSponsorContract, (admin.clone(), 5u32, 20u32, 100_000u64));
    let client = FeeSponsorContractClient::new(&env, &contract_id);

    // Non-admin cannot whitelist contract
    let unauth_res = client.try_whitelist_contract(&vault);
    assert!(unauth_res.is_err());

    // Admin updates limits
    env.mock_all_auths();
    client.set_limits(&10, &40, &200_000);
    let config = client.get_config();
    assert_eq!(config.max_ops_per_hour, 10);
    assert_eq!(config.max_ops_per_day, 40);
    assert_eq!(config.daily_budget_stroops, 200_000);

    // Invalid limit configurations rejected
    assert_eq!(
        client.try_set_limits(&0, &40, &200_000),
        Err(Ok(SponsorError::InvalidConfig))
    );
    assert_eq!(
        client.try_set_limits(&50, &40, &200_000),
        Err(Ok(SponsorError::InvalidConfig))
    );
    assert_eq!(
        client.try_set_limits(&10, &40, &0),
        Err(Ok(SponsorError::InvalidConfig))
    );

    // Admin rotation: set_admin
    client.set_admin(&new_admin);
    assert_eq!(client.get_config().admin, new_admin);

    // Whitelist query view function
    assert_eq!(client.get_whitelisted_contracts().len(), 0);
    client.whitelist_contract(&vault);
    let whitelisted = client.get_whitelisted_contracts();
    assert_eq!(whitelisted.len(), 1);
    assert_eq!(whitelisted.get(0).unwrap(), vault);

    // (Audit M3: an uninitialized sponsor can no longer exist - configuration
    // happens in the constructor, atomically with deployment.)
}

// -------------------------------------------------------------------------
// Audit M3 (2026-11-15): a freshly deployed sponsor must not be claimable by
// whoever calls `initialize` first (they would control the fee budget).
// -------------------------------------------------------------------------
#[test]
fn m3_fresh_sponsor_cannot_be_claimed_by_a_stranger() {
    use soroban_sdk::{testutils::Address as _, IntoVal};
    let env = Env::default();
    let owner = Address::generate(&env);
    let sponsor = env.register(FeeSponsorContract, (owner.clone(), 10u32, 50u32, 1_000u64));
    let attacker = Address::generate(&env);
    let args = (attacker, 1_000u32, 1_000u32, u64::MAX).into_val(&env);
    let r = env.try_invoke_contract::<(), soroban_sdk::Error>(&sponsor, &Symbol::new(&env, "initialize"), args);
    assert!(!matches!(r, Ok(Ok(()))), "a stranger must not be able to claim the sponsor");
    assert_eq!(FeeSponsorContractClient::new(&env, &sponsor).get_config().admin, owner);
}

// -------------------------------------------------------------------------
// Audit M2 (2026-11-15): custom events use SCREAMING_SNAKE_CASE (Rule 8).
// -------------------------------------------------------------------------
#[test]
fn m2_every_sponsor_event_follows_rule_8() {
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{FromVal, Symbol, SymbolStr, TryFromVal};
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let id = env.register(FeeSponsorContract, (admin.clone(), 10u32, 50u32, 1_000_000u64));
    let client = FeeSponsorContractClient::new(&env, &id);
    let check = |env: &Env| -> usize {
        let mut n = 0;
        for (c, topics, _) in all_events(env).iter() {
            if c != id { continue; }
            let sym = Symbol::try_from_val(env, &topics.get(0).unwrap()).unwrap();
            let s = SymbolStr::from_val(env, &sym.to_symbol_val());
            let bytes: &[u8] = s.as_ref();
            assert!(bytes.iter().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || *b == b'_'),
                "event `{}` violates Rule 8", core::str::from_utf8(bytes).unwrap());
            n += 1;
        }
        n
    };
    let vault = Address::generate(&env);
    let user = Address::generate(&env);
    let mut n = 0;
    client.whitelist_contract(&vault); n += check(&env);
    let relayer = Address::generate(&env);
    client.add_relayer(&relayer);
    client.record_sponsored_op(&relayer, &user, &vault, &100); n += check(&env);
    client.set_limits(&20, &60, &2_000_000); n += check(&env);
    client.pause(); n += check(&env);
    client.unpause(); n += check(&env);
    client.remove_contract(&vault); n += check(&env);
    client.set_admin(&Address::generate(&env)); n += check(&env);
    assert!(n >= 7, "saw {n} events");
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
fn golden_events_fee_sponsor() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let id = env.register(FeeSponsorContract, (admin.clone(), 10u32, 50u32, 1_000_000u64));
    let mut g = GoldenEvents::new(&id);
    g.capture(&env, "constructor");
    let client = FeeSponsorContractClient::new(&env, &id);
    let vault = Address::generate(&env);
    let user = Address::generate(&env);
    client.whitelist_contract(&vault); g.capture(&env, "whitelist_contract");
    let relayer = Address::generate(&env);
    client.add_relayer(&relayer); g.capture(&env, "add_relayer");
    client.record_sponsored_op(&relayer, &user, &vault, &100); g.capture(&env, "record_sponsored_op");
    client.set_limits(&20, &60, &2_000_000); g.capture(&env, "set_limits");
    client.pause(); g.capture(&env, "pause");
    client.unpause(); g.capture(&env, "unpause");
    client.remove_relayer(&relayer); g.capture(&env, "remove_relayer");
    client.remove_contract(&vault); g.capture(&env, "remove_contract");
    client.set_admin(&Address::generate(&env)); g.capture(&env, "set_admin");
    g.finish(&["SPONSOR_INITIALIZED", "CONTRACT_WHITELISTED", "RELAYER_ADDED", "RELAYER_REMOVED", "SPONSORED_OP", "LIMITS_UPDATED",
        "SPONSOR_PAUSED", "SPONSOR_UNPAUSED", "CONTRACT_UNWHITELISTED", "SPONSOR_ADMIN_TRANSFERRED"]);
}

/// Audit F1a: per-user quota state must not live in instance storage. The
/// instance entry is loaded by every call and is size-capped, so growing it
/// per user would eventually brick the sponsor for everyone (same class as
/// vault audit W2). Each user's stats belong in their own persistent entry.
#[test]
fn f1a_instance_storage_does_not_grow_with_the_number_of_users() {
    use soroban_sdk::testutils::storage::{Instance as _, Persistent as _};
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_731_600_000);
    let admin = Address::generate(&env);
    let id = env.register(FeeSponsorContract, (admin.clone(), 10u32, 50u32, 1_000_000_000u64));
    let client = FeeSponsorContractClient::new(&env, &id);
    let vault = Address::generate(&env);
    client.whitelist_contract(&vault);
    let first = Address::generate(&env);
    let relayer = Address::generate(&env);
    client.add_relayer(&relayer);
    client.record_sponsored_op(&relayer, &first, &vault, &1_000);
    let instance_len = || env.as_contract(&id, || env.storage().instance().all().len());
    let baseline = instance_len();
    for _ in 0..20 {
        client.record_sponsored_op(&relayer, &Address::generate(&env), &vault, &1_000);
    }
    assert_eq!(instance_len(), baseline, "per-user stats leaked into instance storage");
    assert!(env.as_contract(&id, || env.storage().instance().get_ttl()) > 0);
    let persisted = env.as_contract(&id, || env.storage().persistent().all().len());
    assert!(persisted >= 21, "each user's stats must be their own persistent entry");
}

/// Audit F1b: only the designated relayer records sponsored operations; the
/// relayer key cannot reconfigure anything; only the admin designates it.
#[test]
fn f1b_only_pool_relayers_record_and_they_cannot_reconfigure() {
    use soroban_sdk::testutils::{MockAuth, MockAuthInvoke};
    use soroban_sdk::IntoVal;
    let env = Env::default();
    env.ledger().set_timestamp(1_731_600_000);
    let admin = Address::generate(&env);
    let r1 = Address::generate(&env);
    let r2 = Address::generate(&env);
    let id = env.register(FeeSponsorContract, (admin.clone(), 10u32, 50u32, 1_000_000u64));
    let client = FeeSponsorContractClient::new(&env, &id);
    let vault = Address::generate(&env);
    let user = Address::generate(&env);
    let as_ = |who: &Address, f: &'static str, args: soroban_sdk::Vec<soroban_sdk::Val>| {
        env.mock_auths(&[MockAuth { address: who, invoke: &MockAuthInvoke { contract: &id, fn_name: f, args, sub_invokes: &[] } }]);
    };
    let rec = |r: &Address| (r.clone(), user.clone(), vault.clone(), 100u64).into_val(&env);
    as_(&admin, "whitelist_contract", (vault.clone(),).into_val(&env));
    client.whitelist_contract(&vault);
    // Nobody can record before a relayer is added.
    as_(&r1, "record_sponsored_op", rec(&r1));
    assert_eq!(client.try_record_sponsored_op(&r1, &user, &vault, &100), Err(Ok(SponsorError::RelayerNotSet)));
    // Only the admin manages the pool.
    as_(&r1, "add_relayer", (r1.clone(),).into_val(&env));
    assert!(client.try_add_relayer(&r1).is_err());
    for r in [&r1, &r2] {
        as_(&admin, "add_relayer", (r.clone(),).into_val(&env));
        client.add_relayer(r);
    }
    as_(&admin, "add_relayer", (r1.clone(),).into_val(&env));
    assert_eq!(client.try_add_relayer(&r1), Err(Ok(SponsorError::RelayerExists)));
    assert_eq!(client.get_relayers(), soroban_sdk::vec![&env, r1.clone(), r2.clone()]);
    // Both pool members record; each must sign for itself.
    for r in [&r1, &r2] {
        as_(r, "record_sponsored_op", rec(r));
        client.record_sponsored_op(r, &user, &vault, &100);
    }
    assert_eq!(client.get_user_stats(&user).daily_ops, 2);
    as_(&r1, "record_sponsored_op", rec(&r2));
    assert!(client.try_record_sponsored_op(&r2, &user, &vault, &100).is_err(), "r1 cannot sign as r2");
    // The admin is not a relayer; a stranger is not either, even signing.
    let stranger = Address::generate(&env);
    as_(&stranger, "record_sponsored_op", rec(&stranger));
    assert_eq!(client.try_record_sponsored_op(&stranger, &user, &vault, &100), Err(Ok(SponsorError::NotARelayer)));
    as_(&admin, "record_sponsored_op", rec(&admin));
    assert_eq!(client.try_record_sponsored_op(&admin, &user, &vault, &100), Err(Ok(SponsorError::NotARelayer)));
    // A removed (rotated / compromised) key stops working at once.
    as_(&admin, "remove_relayer", (r2.clone(),).into_val(&env));
    client.remove_relayer(&r2);
    as_(&r2, "record_sponsored_op", rec(&r2));
    assert_eq!(client.try_record_sponsored_op(&r2, &user, &vault, &100), Err(Ok(SponsorError::NotARelayer)));
    as_(&admin, "remove_relayer", (r2.clone(),).into_val(&env));
    assert_eq!(client.try_remove_relayer(&r2), Err(Ok(SponsorError::NotARelayer)));
    // A relayer key cannot loosen anything.
    as_(&r1, "set_limits", (1_000u32, 5_000u32, 1_000_000_000u64).into_val(&env));
    assert!(client.try_set_limits(&1_000, &5_000, &1_000_000_000).is_err());
    as_(&admin, "pause", ().into_val(&env));
    client.pause();
    as_(&r1, "unpause", ().into_val(&env));
    assert!(client.try_unpause().is_err());
    as_(&r1, "whitelist_contract", (user.clone(),).into_val(&env));
    assert!(client.try_whitelist_contract(&user).is_err());
    as_(&r1, "add_relayer", (stranger.clone(),).into_val(&env));
    assert!(client.try_add_relayer(&stranger).is_err());
}

/// Decision D-I: the pool is bounded (it lives in instance storage).
#[test]
fn f1c_the_relayer_pool_is_bounded() {
    let env = Env::default();
    env.mock_all_auths();
    let id = env.register(FeeSponsorContract, (Address::generate(&env), 10u32, 50u32, 1_000_000u64));
    let client = FeeSponsorContractClient::new(&env, &id);
    for _ in 0..MAX_RELAYERS {
        client.add_relayer(&Address::generate(&env));
    }
    assert_eq!(client.try_add_relayer(&Address::generate(&env)), Err(Ok(SponsorError::TooManyRelayers)));
    assert_eq!(client.get_relayers().len(), MAX_RELAYERS);
}

// -------------------------------------------------------------------------
// Step 3: users' own vaults have new addresses, so a static whitelist cannot
// sponsor them. The sponsor also trusts vaults PROVEN by a registered
// Constella factory (`is_vault`: deployed by that factory from approved
// code). Bounded, admin-managed; explicit whitelisting keeps working.
// Real factory and real vault below, no doubles.
// -------------------------------------------------------------------------
mod trusted_factory {
    use super::*;
    use constella_nav_oracle::NavOracle;
    use constella_policy::PolicyContract;
    use constella_registry::{AssetEntry, AssetRegistry, AssetRegistryClient, SleeveType};
    use constella_vault_factory::{VaultFactory, VaultFactoryClient, VaultParams};
    use soroban_sdk::{testutils::{MockAuth, MockAuthInvoke}, vec, BytesN, IntoVal, String};

    const VAULT_WASM: &[u8] = include_bytes!("../../target/wasm32v1-none/release/constella_vault_strategy.wasm");
    const ONE: i128 = 10_000_000;

    struct S { env: Env, sponsor: FeeSponsorContractClient<'static>, factory: VaultFactoryClient<'static>, vault: Address }

    fn setup() -> S {
        let env = Env::default();
        env.mock_all_auths();
        env.cost_estimate().budget().reset_unlimited();
        let admin = Address::generate(&env);
        let usdc = env.register_stellar_asset_contract_v2(Address::generate(&env)).address();
        let registry = env.register(AssetRegistry, (admin.clone(),));
        AssetRegistryClient::new(&env, &registry).add_asset(&AssetEntry {
            contract_address: usdc.clone(), asset_id: Symbol::new(&env, "USDC"), issuer: Symbol::new(&env, "CIRCLE"),
            sleeve_type: SleeveType::Cash, liquidity_tier: 0, permissionless: true,
            oracle_sources: vec![&env, Symbol::new(&env, "USDC_BASE")], eligible_jurisdictions: vec![&env],
            blocked_jurisdictions: vec![&env], investor_classes_required: vec![&env], is_active: true, added_at: 0 });
        let oracle = env.register(NavOracle, (admin.clone(), usdc.clone(), 7u32, 300u32));
        let policy = env.register(PolicyContract, (admin.clone(), oracle.clone()));
        let hash = env.deployer().upload_contract_wasm(VAULT_WASM);
        let factory_id = env.register(VaultFactory, (admin.clone(), hash, registry, oracle, policy, admin.clone(),
                                                     1_000_000 * ONE, 1000u32, Address::generate(&env), None::<Address>));
        let factory = VaultFactoryClient::new(&env, &factory_id);
        let creator = Address::generate(&env);
        let vault = factory.create_vault(&creator, &VaultParams {
            admin: creator.clone(), pause_authority: creator.clone(), numeraire: usdc, performance_fee_bps: 1000,
            tvl_cap: 1000 * ONE, per_user_cap: 1000 * ONE, name: String::from_str(&env, "Mine"),
            symbol: String::from_str(&env, "MINE"), salt: BytesN::from_array(&env, &[1; 32]) });
        let sponsor_id = env.register(FeeSponsorContract, (admin, 10u32, 50u32, 1_000_000u64));
        S { env: env.clone(), sponsor: FeeSponsorContractClient::new(&env, &sponsor_id), factory, vault }
    }

    #[test]
    fn a_vault_proven_by_a_trusted_factory_is_sponsorable() {
        let s = setup();
        let user = Address::generate(&s.env);
        let stranger_contract = Address::generate(&s.env);
        assert!(!s.sponsor.is_whitelisted(&s.vault), "not before the factory is trusted");
        s.sponsor.trust_factory(&s.factory.address);
        assert_eq!(s.sponsor.get_trusted_factories(), vec![&s.env, s.factory.address.clone()]);
        assert!(s.sponsor.is_whitelisted(&s.vault));
        assert_eq!(s.sponsor.try_validate_sponsorship(&user, &s.vault, &5000), Ok(Ok(())));
        // A contract the factory did not deploy stays refused.
        assert!(!s.sponsor.is_whitelisted(&stranger_contract));
        assert_eq!(s.sponsor.try_validate_sponsorship(&user, &stranger_contract, &5000),
                   Err(Ok(SponsorError::TargetNotWhitelisted)));
        // Untrusting the factory removes the coverage at once.
        s.sponsor.untrust_factory(&s.factory.address);
        assert!(!s.sponsor.is_whitelisted(&s.vault));
        assert!(s.sponsor.get_trusted_factories().is_empty());
    }

    #[test]
    fn only_the_admin_manages_trusted_factories() {
        let s = setup();
        let mallory = Address::generate(&s.env);
        s.env.mock_auths(&[MockAuth { address: &mallory, invoke: &MockAuthInvoke { contract: &s.sponsor.address,
            fn_name: "trust_factory", args: (s.factory.address.clone(),).into_val(&s.env), sub_invokes: &[] } }]);
        assert!(s.sponsor.try_trust_factory(&s.factory.address).is_err());
        s.env.mock_all_auths();
        s.sponsor.trust_factory(&s.factory.address);
        s.env.mock_auths(&[MockAuth { address: &mallory, invoke: &MockAuthInvoke { contract: &s.sponsor.address,
            fn_name: "untrust_factory", args: (s.factory.address.clone(),).into_val(&s.env), sub_invokes: &[] } }]);
        assert!(s.sponsor.try_untrust_factory(&s.factory.address).is_err());
        assert!(s.sponsor.is_whitelisted(&s.vault));
    }

    #[test]
    fn the_trusted_factory_list_is_bounded_and_without_duplicates() {
        let s = setup();
        s.sponsor.trust_factory(&s.factory.address);
        assert_eq!(s.sponsor.try_trust_factory(&s.factory.address), Err(Ok(SponsorError::FactoryAlreadyTrusted)));
        for _ in 1..MAX_TRUSTED_FACTORIES {
            s.sponsor.trust_factory(&Address::generate(&s.env));
        }
        assert_eq!(s.sponsor.try_trust_factory(&Address::generate(&s.env)), Err(Ok(SponsorError::TooManyFactories)));
        assert_eq!(s.sponsor.try_untrust_factory(&Address::generate(&s.env)), Err(Ok(SponsorError::FactoryNotTrusted)));
    }

    #[test]
    fn explicit_whitelisting_still_works_alongside() {
        let s = setup();
        let target = Address::generate(&s.env);
        s.sponsor.whitelist_contract(&target);
        s.sponsor.trust_factory(&s.factory.address);
        assert!(s.sponsor.is_whitelisted(&target) && s.sponsor.is_whitelisted(&s.vault));
    }
}
