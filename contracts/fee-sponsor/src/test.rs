#![cfg(test)]

use super::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    Address, Env,
};

#[test]
fn test_t52_sponsorship_whitelist() {
    let env = Env::default();
    let contract_id = env.register(FeeSponsorContract, ());
    let client = FeeSponsorContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let vault_contract = Address::generate(&env);
    let foreign_contract = Address::generate(&env);

    // Initialize sponsor contract with 10 ops/hour, 50 ops/day, 1,000,000 stroops daily budget
    client.initialize(&admin, &10, &50, &1_000_000);

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
    let contract_id = env.register(FeeSponsorContract, ());
    let client = FeeSponsorContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let vault_contract = Address::generate(&env);

    // Set tight rate limits: 3 ops/hour, 5 ops/day, 1,000,000 stroops budget
    client.initialize(&admin, &3, &5, &1_000_000);

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
    client.record_sponsored_op(&user, &vault_contract, &1000);

    // Operation 2 (same hour)
    assert_eq!(
        client.try_validate_sponsorship(&user, &vault_contract, &1000),
        Ok(Ok(()))
    );
    client.record_sponsored_op(&user, &vault_contract, &1000);

    // Operation 3 (same hour)
    assert_eq!(
        client.try_validate_sponsorship(&user, &vault_contract, &1000),
        Ok(Ok(()))
    );
    client.record_sponsored_op(&user, &vault_contract, &1000);

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
    client.record_sponsored_op(&user, &vault_contract, &1000);

    // Operation 5 succeeds in the new hour! (Daily ops now reaches 5)
    assert_eq!(
        client.try_validate_sponsorship(&user, &vault_contract, &1000),
        Ok(Ok(()))
    );
    client.record_sponsored_op(&user, &vault_contract, &1000);

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
    let contract_id = env.register(FeeSponsorContract, ());
    let client = FeeSponsorContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let user_1 = Address::generate(&env);
    let user_2 = Address::generate(&env);
    let vault_contract = Address::generate(&env);

    // Set daily budget: 50,000 stroops (e.g. 0.005 XLM)
    client.initialize(&admin, &100, &200, &50_000);

    env.mock_all_auths();
    client.whitelist_contract(&vault_contract);

    let start_ts: u64 = 200_000;
    env.ledger().set_timestamp(start_ts);

    // User 1 executes op consuming 30,000 stroops
    assert_eq!(
        client.try_validate_sponsorship(&user_1, &vault_contract, &30_000),
        Ok(Ok(()))
    );
    client.record_sponsored_op(&user_1, &vault_contract, &30_000);

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
    client.record_sponsored_op(&user_2, &vault_contract, &20_000);

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
    let paused_rec = client.try_record_sponsored_op(&user_1, &vault_contract, &100);
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
    let contract_id = env.register(FeeSponsorContract, ());
    let client = FeeSponsorContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let new_admin = Address::generate(&env);
    let user = Address::generate(&env);
    let vault = Address::generate(&env);

    client.initialize(&admin, &5, &20, &100_000);

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

    // Uninitialized checks on a fresh contract
    let uninit_id = env.register(FeeSponsorContract, ());
    let uninit_client = FeeSponsorContractClient::new(&env, &uninit_id);
    assert_eq!(
        uninit_client.try_validate_sponsorship(&user, &vault, &1000),
        Err(Ok(SponsorError::NotInitialized))
    );
    assert_eq!(
        uninit_client.try_get_config(),
        Err(Ok(SponsorError::NotInitialized))
    );
}

// -------------------------------------------------------------------------
// Audit M3 (2026-11-15): a freshly deployed sponsor must not be claimable by
// whoever calls `initialize` first (they would control the fee budget).
// -------------------------------------------------------------------------
#[test]
fn m3_fresh_sponsor_cannot_be_claimed_by_a_stranger() {
    use soroban_sdk::{testutils::Address as _, IntoVal};
    let env = Env::default();
    let sponsor = env.register(FeeSponsorContract, ());
    let attacker = Address::generate(&env);
    let args = (attacker, 1_000u32, 1_000u32, u64::MAX).into_val(&env);
    let r = env.try_invoke_contract::<(), soroban_sdk::Error>(&sponsor, &Symbol::new(&env, "initialize"), args);
    assert!(!matches!(r, Ok(Ok(()))), "a stranger must not be able to claim the sponsor");
}
