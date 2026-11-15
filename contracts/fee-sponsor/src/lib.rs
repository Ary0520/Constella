#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, Address, Env, Symbol, Vec,
};

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum SponsorError {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    SponsorPaused = 4,
    TargetNotWhitelisted = 5,
    HourlyLimitExceeded = 6,
    DailyLimitExceeded = 7,
    DailyBudgetExceeded = 8,
    ContractAlreadyWhitelisted = 9,
    ContractNotWhitelisted = 10,
    InvalidConfig = 11,
}

#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct SponsorConfig {
    pub admin: Address,
    pub max_ops_per_hour: u32,
    pub max_ops_per_day: u32,
    pub daily_budget_stroops: u64,
    pub is_paused: bool,
}

#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct UserSponsorStats {
    pub hourly_ops: u32,
    pub daily_ops: u32,
    pub last_op_timestamp: u64,
}

#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct GlobalSponsorStats {
    pub total_ops_today: u32,
    pub total_fees_today_stroops: u64,
    pub current_day_epoch: u64,
}

#[derive(Clone)]
#[contracttype]
pub enum DataKey {
    Config,
    Whitelisted(Address),
    WhitelistList,
    UserStats(Address),
    GlobalStats,
}

#[contract]
pub struct FeeSponsorContract;

#[contractimpl]
impl FeeSponsorContract {
    /// Configure guardrails at deployment (atomic, so no one can claim an
    /// uninitialized sponsor - audit M3). There is no `initialize` entrypoint.
    pub fn __constructor(
        env: Env,
        admin: Address,
        max_ops_per_hour: u32,
        max_ops_per_day: u32,
        daily_budget_stroops: u64,
    ) {
        if max_ops_per_hour == 0
            || max_ops_per_day < max_ops_per_hour
            || daily_budget_stroops == 0
        {
            soroban_sdk::panic_with_error!(&env, SponsorError::InvalidConfig);
        }

        let config = SponsorConfig {
            admin: admin.clone(),
            max_ops_per_hour,
            max_ops_per_day,
            daily_budget_stroops,
            is_paused: false,
        };

        let now = env.ledger().timestamp();
        let global_stats = GlobalSponsorStats {
            total_ops_today: 0,
            total_fees_today_stroops: 0,
            current_day_epoch: now / 86400,
        };

        env.storage().instance().set(&DataKey::Config, &config);
        env.storage()
            .instance()
            .set(&DataKey::GlobalStats, &global_stats);
        let empty_list: Vec<Address> = Vec::new(&env);
        env.storage()
            .instance()
            .set(&DataKey::WhitelistList, &empty_list);

        env.events().publish(
            (Symbol::new(&env, "sponsor_initialized"), admin),
            daily_budget_stroops,
        );

    }

    /// Whitelist an approved Constella contract (e.g. strategy vault, policy) for sponsored gas
    pub fn whitelist_contract(env: Env, target: Address) -> Result<(), SponsorError> {
        let config = Self::require_admin(&env)?;

        if env
            .storage()
            .instance()
            .has(&DataKey::Whitelisted(target.clone()))
        {
            return Err(SponsorError::ContractAlreadyWhitelisted);
        }

        env.storage()
            .instance()
            .set(&DataKey::Whitelisted(target.clone()), &true);

        let mut list: Vec<Address> = env
            .storage()
            .instance()
            .get(&DataKey::WhitelistList)
            .unwrap_or_else(|| Vec::new(&env));
        list.push_back(target.clone());
        env.storage().instance().set(&DataKey::WhitelistList, &list);

        env.events().publish(
            (Symbol::new(&env, "contract_whitelisted"), config.admin),
            target,
        );

        Ok(())
    }

    /// Remove a contract from the sponsorship whitelist (storage rent hygiene enforced)
    pub fn remove_contract(env: Env, target: Address) -> Result<(), SponsorError> {
        let config = Self::require_admin(&env)?;

        if !env
            .storage()
            .instance()
            .has(&DataKey::Whitelisted(target.clone()))
        {
            return Err(SponsorError::ContractNotWhitelisted);
        }

        // Storage rent hygiene: completely remove boolean flag
        env.storage()
            .instance()
            .remove(&DataKey::Whitelisted(target.clone()));

        let list: Vec<Address> = env
            .storage()
            .instance()
            .get(&DataKey::WhitelistList)
            .unwrap_or_else(|| Vec::new(&env));

        let mut new_list = Vec::new(&env);
        for item in list.iter() {
            if item != target {
                new_list.push_back(item);
            }
        }
        env.storage()
            .instance()
            .set(&DataKey::WhitelistList, &new_list);

        env.events().publish(
            (Symbol::new(&env, "contract_unwhitelisted"), config.admin),
            target,
        );

        Ok(())
    }

    /// Check if target contract is approved for fee sponsorship
    pub fn is_whitelisted(env: Env, target: Address) -> bool {
        env.storage()
            .instance()
            .get(&DataKey::Whitelisted(target))
            .unwrap_or(false)
    }

    /// Validate whether an operation is eligible for fee sponsorship (stateless check against limits)
    pub fn validate_sponsorship(
        env: Env,
        user: Address,
        target_contract: Address,
        fee_bid_stroops: u64,
    ) -> Result<(), SponsorError> {
        let config: SponsorConfig = match env.storage().instance().get(&DataKey::Config) {
            Some(c) => c,
            None => return Err(SponsorError::NotInitialized),
        };

        if config.is_paused {
            return Err(SponsorError::SponsorPaused);
        }

        if !Self::is_whitelisted(env.clone(), target_contract) {
            return Err(SponsorError::TargetNotWhitelisted);
        }

        // Global daily budget validation
        let global_stats = Self::get_global_stats(env.clone());
        let projected_global_fees = global_stats
            .total_fees_today_stroops
            .saturating_add(fee_bid_stroops);

        if projected_global_fees > config.daily_budget_stroops {
            return Err(SponsorError::DailyBudgetExceeded);
        }

        // Per-user quota validation (hourly & daily)
        let user_stats = Self::get_user_stats(env.clone(), user);

        if user_stats.hourly_ops.saturating_add(1) > config.max_ops_per_hour {
            return Err(SponsorError::HourlyLimitExceeded);
        }

        if user_stats.daily_ops.saturating_add(1) > config.max_ops_per_day {
            return Err(SponsorError::DailyLimitExceeded);
        }

        Ok(())
    }

    /// Record an executed sponsored operation (updates quotas and consumes daily budget)
    pub fn record_sponsored_op(
        env: Env,
        user: Address,
        target_contract: Address,
        fee_spent_stroops: u64,
    ) -> Result<(), SponsorError> {
        let _config = Self::require_admin(&env)?;

        // Ensure operation adheres to whitelist, circuit breaker, and budget before recording
        Self::validate_sponsorship(
            env.clone(),
            user.clone(),
            target_contract.clone(),
            fee_spent_stroops,
        )?;

        let now = env.ledger().timestamp();
        let today_epoch = now / 86400;

        // Update Global Stats
        let global_stats = Self::get_global_stats(env.clone());
        let updated_global_stats = GlobalSponsorStats {
            total_ops_today: global_stats.total_ops_today.saturating_add(1),
            total_fees_today_stroops: global_stats
                .total_fees_today_stroops
                .saturating_add(fee_spent_stroops),
            current_day_epoch: today_epoch,
        };
        env.storage()
            .instance()
            .set(&DataKey::GlobalStats, &updated_global_stats);

        // Update User Stats
        let current_user_stats = Self::get_user_stats(env.clone(), user.clone());
        let updated_user_stats = UserSponsorStats {
            hourly_ops: current_user_stats.hourly_ops.saturating_add(1),
            daily_ops: current_user_stats.daily_ops.saturating_add(1),
            last_op_timestamp: now,
        };

        env.storage()
            .instance()
            .set(&DataKey::UserStats(user.clone()), &updated_user_stats);

        env.events().publish(
            (
                Symbol::new(&env, "sponsored_op"),
                user,
                target_contract,
            ),
            fee_spent_stroops,
        );

        Ok(())
    }

    /// Configure rate limits and daily budget
    pub fn set_limits(
        env: Env,
        max_ops_per_hour: u32,
        max_ops_per_day: u32,
        daily_budget_stroops: u64,
    ) -> Result<(), SponsorError> {
        let mut config = Self::require_admin(&env)?;

        if max_ops_per_hour == 0
            || max_ops_per_day < max_ops_per_hour
            || daily_budget_stroops == 0
        {
            return Err(SponsorError::InvalidConfig);
        }

        config.max_ops_per_hour = max_ops_per_hour;
        config.max_ops_per_day = max_ops_per_day;
        config.daily_budget_stroops = daily_budget_stroops;

        env.storage().instance().set(&DataKey::Config, &config);

        env.events().publish(
            (Symbol::new(&env, "limits_updated"), config.admin),
            daily_budget_stroops,
        );

        Ok(())
    }

    /// Transfer sponsorship administration to a new admin address
    pub fn set_admin(env: Env, new_admin: Address) -> Result<(), SponsorError> {
        let mut config = Self::require_admin(&env)?;
        let old_admin = config.admin.clone();
        config.admin = new_admin.clone();
        env.storage().instance().set(&DataKey::Config, &config);

        env.events().publish(
            (Symbol::new(&env, "sponsor_admin_transferred"), old_admin),
            new_admin,
        );

        Ok(())
    }

    /// Emergency circuit breaker: pause all sponsorship immediately
    pub fn pause(env: Env) -> Result<(), SponsorError> {
        let mut config = Self::require_admin(&env)?;
        config.is_paused = true;
        env.storage().instance().set(&DataKey::Config, &config);

        env.events()
            .publish((Symbol::new(&env, "sponsor_paused"), config.admin), ());

        Ok(())
    }

    /// Emergency circuit breaker: resume sponsorship
    pub fn unpause(env: Env) -> Result<(), SponsorError> {
        let mut config = Self::require_admin(&env)?;
        config.is_paused = false;
        env.storage().instance().set(&DataKey::Config, &config);

        env.events()
            .publish((Symbol::new(&env, "sponsor_unpaused"), config.admin), ());

        Ok(())
    }

    /// Get current sponsor configuration
    pub fn get_config(env: Env) -> Result<SponsorConfig, SponsorError> {
        match env.storage().instance().get(&DataKey::Config) {
            Some(c) => Ok(c),
            None => Err(SponsorError::NotInitialized),
        }
    }

    /// Query usage statistics for a specific user (evaluates effective quota for current timestamp)
    pub fn get_user_stats(env: Env, user: Address) -> UserSponsorStats {
        let now = env.ledger().timestamp();
        let stored: Option<UserSponsorStats> =
            env.storage().instance().get(&DataKey::UserStats(user));
        match stored {
            Some(stats) => {
                let hourly = if stats.last_op_timestamp == 0
                    || (now / 3600 != stats.last_op_timestamp / 3600)
                {
                    0
                } else {
                    stats.hourly_ops
                };

                let daily = if stats.last_op_timestamp == 0
                    || (now / 86400 != stats.last_op_timestamp / 86400)
                {
                    0
                } else {
                    stats.daily_ops
                };

                UserSponsorStats {
                    hourly_ops: hourly,
                    daily_ops: daily,
                    last_op_timestamp: stats.last_op_timestamp,
                }
            }
            None => UserSponsorStats {
                hourly_ops: 0,
                daily_ops: 0,
                last_op_timestamp: 0,
            },
        }
    }

    /// Query global sponsorship stats for today (resets if day rolled over)
    pub fn get_global_stats(env: Env) -> GlobalSponsorStats {
        let now = env.ledger().timestamp();
        let today_epoch = now / 86400;

        let stored: Option<GlobalSponsorStats> =
            env.storage().instance().get(&DataKey::GlobalStats);
        match stored {
            Some(stats) if stats.current_day_epoch == today_epoch => stats,
            _ => GlobalSponsorStats {
                total_ops_today: 0,
                total_fees_today_stroops: 0,
                current_day_epoch: today_epoch,
            },
        }
    }

    /// Query all currently whitelisted target contracts
    pub fn get_whitelisted_contracts(env: Env) -> Vec<Address> {
        env.storage()
            .instance()
            .get(&DataKey::WhitelistList)
            .unwrap_or_else(|| Vec::new(&env))
    }

    fn require_admin(env: &Env) -> Result<SponsorConfig, SponsorError> {
        let config: SponsorConfig = match env.storage().instance().get(&DataKey::Config) {
            Some(c) => c,
            None => return Err(SponsorError::NotInitialized),
        };
        config.admin.require_auth();
        Ok(config)
    }
}

#[cfg(test)]
mod test;

