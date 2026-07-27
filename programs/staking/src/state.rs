//! Pool and position state, and the reward accounting that connects them.
//!
//! The whole design turns on one idea: instead of tracking what each staker is
//! owed, the pool tracks a single running total of *rewards per unit of weighted
//! stake* since inception. A position records the value of that accumulator when
//! it last settled. The difference between the two, times the position's weight,
//! is what it has earned — so paying out is O(1) and the program never iterates
//! over stakers.
//!
//! This is the Synthetix `StakingRewards` / MasterChef shape. The alternative —
//! looping over stakers to credit each one — passes a ten-staker test and then
//! permanently bricks the pool once distribution exceeds the compute budget.

use anchor_lang::prelude::*;

use crate::constants::{BPS_DENOMINATOR, PRECISION, SECONDS_PER_DAY};
use crate::errors::StakingError;

/// Lock commitment, which sets both reward share and governance weight.
///
/// Weight is applied to the staked amount to give a *weighted* amount; all
/// reward maths and all vote tallies operate on the weighted figure. Tying
/// influence to lock duration is what makes the governance design in
/// `helix-governance` work: vote weight cannot be rented for a single block.
#[derive(
    AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, PartialEq, Eq, Debug, Default,
)]
pub enum LockTier {
    #[default]
    Flexible,
    Bronze,
    Silver,
    Gold,
}

impl LockTier {
    /// Lock duration in seconds.
    pub const fn duration(self) -> i64 {
        match self {
            LockTier::Flexible => 0,
            LockTier::Bronze => 30 * SECONDS_PER_DAY,
            LockTier::Silver => 90 * SECONDS_PER_DAY,
            LockTier::Gold => 180 * SECONDS_PER_DAY,
        }
    }

    /// Reward and vote multiplier, in basis points.
    pub const fn weight_bps(self) -> u64 {
        match self {
            LockTier::Flexible => 10_000, // 1.00x
            LockTier::Bronze => 12_500,   // 1.25x
            LockTier::Silver => 15_000,   // 1.50x
            LockTier::Gold => 20_000,     // 2.00x
        }
    }

    /// Applies this tier's multiplier to `amount`.
    ///
    /// Truncates, so weight is never rounded up. Rounding weight up would let a
    /// position claim a marginally larger share of emissions than it paid for,
    /// and the error would compound across every position in the pool.
    pub fn apply_weight(self, amount: u64) -> Result<u64> {
        let weighted = (amount as u128)
            .checked_mul(self.weight_bps() as u128)
            .ok_or(StakingError::MathOverflow)?
            / (BPS_DENOMINATOR as u128);
        u64::try_from(weighted).map_err(|_| StakingError::MathOverflow.into())
    }
}

/// A staking pool. One per (stake mint, reward mint) pair.
#[account]
#[derive(InitSpace, Debug)]
pub struct Pool {
    /// Sets the reward rate and funds the reward vault. Cannot touch principal.
    pub authority: Pubkey,
    /// Pending half of the two-step authority handover.
    pub pending_authority: Option<Pubkey>,

    pub stake_mint: Pubkey,
    pub reward_mint: Pubkey,
    pub stake_vault: Pubkey,
    pub reward_vault: Pubkey,

    /// Principal actually held, in stake-token base units. Must always equal the
    /// stake vault balance (`INVARIANTS.md` §1.1).
    pub total_staked: u64,
    /// Sum of every position's weighted amount. The denominator of reward share
    /// and of governance quorum.
    pub total_weighted: u64,

    /// Emission rate in reward-token base units per second.
    pub reward_rate: u64,
    /// Emissions stop at this timestamp. Beyond it the accumulator stops
    /// advancing, so an unfunded pool silently stops paying instead of
    /// promising rewards it cannot cover.
    pub reward_period_end: i64,

    /// Rewards per unit of weighted stake since inception, scaled by
    /// [`PRECISION`]. Monotonically non-decreasing (`INVARIANTS.md` §3.1).
    pub reward_per_token: u128,
    /// Timestamp the accumulator was last advanced to.
    pub last_update_ts: i64,

    pub total_rewards_funded: u64,
    pub total_rewards_paid: u64,

    /// Monotonic counter used to seed position PDAs.
    pub position_count: u64,

    /// Blocks new deposits only. Unstaking and claiming stay live — see
    /// `INVARIANTS.md` §6.4.
    pub paused: bool,

    pub bump: u8,
    pub vault_authority_bump: u8,
}

impl Pool {
    /// Advances [`Self::reward_per_token`] to `now`.
    ///
    /// Must be called before any change to `total_weighted` or to a position's
    /// balance. Calling it afterwards would apply the new weight retroactively
    /// to a period the position was not staked for.
    pub fn update_rewards(&mut self, now: i64) -> Result<()> {
        // Emissions never run past the funded period.
        let effective_now = now.min(self.reward_period_end);

        if effective_now > self.last_update_ts && self.total_weighted > 0 {
            let elapsed = effective_now
                .checked_sub(self.last_update_ts)
                .ok_or(StakingError::MathOverflow)? as u128;

            let emitted = elapsed
                .checked_mul(self.reward_rate as u128)
                .ok_or(StakingError::MathOverflow)?;

            // Truncating here means the remainder stays in the vault rather than
            // being distributed. Rounding the other way would pay out marginally
            // more than was emitted, every single update.
            let delta = emitted
                .checked_mul(PRECISION)
                .ok_or(StakingError::MathOverflow)?
                / (self.total_weighted as u128);

            self.reward_per_token = self
                .reward_per_token
                .checked_add(delta)
                .ok_or(StakingError::MathOverflow)?;
        }

        // Advance the clock even when nothing accrued — when total_weighted is
        // zero, emissions for that interval are deliberately forfeited to the
        // vault rather than handed to whoever stakes next.
        if now > self.last_update_ts {
            self.last_update_ts = now;
        }

        Ok(())
    }

    /// Rewards still outstanding to stakers, given the current accumulator.
    ///
    /// Used by `set_reward_rate` to refuse a rate the vault cannot sustain.
    pub fn unpaid_liability(&self) -> Result<u64> {
        self.total_rewards_funded
            .checked_sub(self.total_rewards_paid)
            .ok_or_else(|| StakingError::MathOverflow.into())
    }

    /// Total emission implied by running `rate` from `from` until
    /// [`Self::reward_period_end`].
    pub fn emission_for(&self, rate: u64, from: i64) -> Result<u64> {
        if self.reward_period_end <= from {
            return Ok(0);
        }
        let seconds = (self.reward_period_end - from) as u128;
        let total = seconds
            .checked_mul(rate as u128)
            .ok_or(StakingError::MathOverflow)?;
        u64::try_from(total).map_err(|_| StakingError::MathOverflow.into())
    }
}

/// One staked position. A user may hold several, in different tiers.
#[account]
#[derive(InitSpace, Debug)]
pub struct Position {
    pub pool: Pubkey,
    pub owner: Pubkey,
    /// Index within the pool, used in the PDA seeds. A `u64` in little-endian
    /// bytes rather than a caller-supplied byte string, so seeds are
    /// fixed-length and cannot be crafted to collide.
    pub position_id: u64,

    /// Principal, in stake-token base units. For a fee-bearing Token-2022 mint
    /// this is the amount the vault *received*, not the amount sent.
    pub amount: u64,
    /// `amount` scaled by the tier multiplier.
    pub weighted_amount: u64,

    pub tier: LockTier,
    /// Principal is withdrawable from this timestamp. Also the gate on voting:
    /// `helix-governance` accepts this position's weight only for proposals
    /// that close at or before `lock_end`.
    pub lock_end: i64,

    /// The pool accumulator as of this position's last settlement.
    pub reward_per_token_paid: u128,
    /// Rewards accrued but not yet withdrawn.
    pub pending_rewards: u64,

    pub created_at: i64,
    pub bump: u8,
}

impl Position {
    /// Total rewards owed at the given accumulator value, without mutating.
    pub fn earned(&self, reward_per_token: u128) -> Result<u64> {
        let delta = reward_per_token
            .checked_sub(self.reward_per_token_paid)
            .ok_or(StakingError::MathOverflow)?;

        // Truncating divide: any fraction of a base unit stays with the pool.
        let accrued = (self.weighted_amount as u128)
            .checked_mul(delta)
            .ok_or(StakingError::MathOverflow)?
            / PRECISION;

        let total = accrued
            .checked_add(self.pending_rewards as u128)
            .ok_or(StakingError::MathOverflow)?;

        u64::try_from(total).map_err(|_| StakingError::MathOverflow.into())
    }

    /// Books everything earned so far into `pending_rewards` and snaps the
    /// watermark to `reward_per_token`.
    ///
    /// Every instruction that changes `weighted_amount` must settle first,
    /// otherwise the new weight would be applied to rewards accrued under the
    /// old one.
    pub fn settle(&mut self, reward_per_token: u128) -> Result<()> {
        self.pending_rewards = self.earned(reward_per_token)?;
        self.reward_per_token_paid = reward_per_token;
        Ok(())
    }

    /// Whether principal may be withdrawn at `now`.
    pub fn is_unlocked(&self, now: i64) -> bool {
        now >= self.lock_end
    }

    /// Whether this position may vote on a proposal closing at
    /// `voting_ends_at`.
    ///
    /// The rule that makes flash-loaned governance weight impossible: the stake
    /// must remain locked until at least the moment the vote closes. A position
    /// opened inside the attacking transaction has `lock_end` at or near `now`,
    /// so it cannot qualify for any live proposal.
    pub fn can_vote_until(&self, voting_ends_at: i64) -> bool {
        self.lock_end >= voting_ends_at
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = SECONDS_PER_DAY;

    fn pool(rate: u64, period_end: i64) -> Pool {
        Pool {
            authority: Pubkey::default(),
            pending_authority: None,
            stake_mint: Pubkey::default(),
            reward_mint: Pubkey::default(),
            stake_vault: Pubkey::default(),
            reward_vault: Pubkey::default(),
            total_staked: 0,
            total_weighted: 0,
            reward_rate: rate,
            reward_period_end: period_end,
            reward_per_token: 0,
            last_update_ts: 0,
            total_rewards_funded: 0,
            total_rewards_paid: 0,
            position_count: 0,
            paused: false,
            bump: 255,
            vault_authority_bump: 255,
        }
    }

    fn position(weighted: u64, tier: LockTier) -> Position {
        Position {
            pool: Pubkey::default(),
            owner: Pubkey::default(),
            position_id: 0,
            amount: weighted,
            weighted_amount: weighted,
            tier,
            lock_end: tier.duration(),
            reward_per_token_paid: 0,
            pending_rewards: 0,
            created_at: 0,
            bump: 255,
        }
    }

    // ------------------------------------------------------------- lock tiers

    #[test]
    fn tier_weights_are_applied() {
        assert_eq!(LockTier::Flexible.apply_weight(1_000).unwrap(), 1_000);
        assert_eq!(LockTier::Bronze.apply_weight(1_000).unwrap(), 1_250);
        assert_eq!(LockTier::Silver.apply_weight(1_000).unwrap(), 1_500);
        assert_eq!(LockTier::Gold.apply_weight(1_000).unwrap(), 2_000);
    }

    #[test]
    fn tier_weight_truncates_rather_than_rounds_up() {
        // 3 * 1.25 = 3.75 -> 3, never 4.
        assert_eq!(LockTier::Bronze.apply_weight(3).unwrap(), 3);
    }

    #[test]
    fn tier_weight_rejects_overflow() {
        assert!(LockTier::Gold.apply_weight(u64::MAX).is_err());
    }

    // ---------------------------------------------------------- accumulator

    #[test]
    fn accumulator_does_not_advance_without_stake() {
        let mut p = pool(100, 10_000);
        p.update_rewards(500).unwrap();
        assert_eq!(p.reward_per_token, 0);
        // The clock still moves, so the idle interval is forfeited rather than
        // paid to whoever stakes next.
        assert_eq!(p.last_update_ts, 500);

        p.total_weighted = 1_000;
        p.update_rewards(600).unwrap();
        // Only the 100s since staking began are distributed.
        assert_eq!(p.reward_per_token, 100 * 100 * PRECISION / 1_000);
    }

    #[test]
    fn accumulator_is_monotonic() {
        let mut p = pool(100, 100_000);
        p.total_weighted = 1_000;
        let mut last = 0u128;
        for t in [10, 10, 50, 51, 900, 900, 1_000] {
            p.update_rewards(t).unwrap();
            assert!(p.reward_per_token >= last, "regressed at t={t}");
            last = p.reward_per_token;
        }
    }

    #[test]
    fn update_is_idempotent_within_a_timestamp() {
        let mut p = pool(100, 100_000);
        p.total_weighted = 1_000;
        p.update_rewards(500).unwrap();
        let once = p.reward_per_token;
        p.update_rewards(500).unwrap();
        p.update_rewards(500).unwrap();
        assert_eq!(p.reward_per_token, once);
    }

    #[test]
    fn emissions_stop_at_period_end() {
        let mut p = pool(100, 1_000);
        p.total_weighted = 1_000;

        p.update_rewards(1_000).unwrap();
        let at_end = p.reward_per_token;

        // Far past the end: nothing further accrues.
        p.update_rewards(1_000_000).unwrap();
        assert_eq!(p.reward_per_token, at_end);
    }

    #[test]
    fn clock_never_runs_backwards() {
        let mut p = pool(100, 100_000);
        p.total_weighted = 1_000;
        p.update_rewards(5_000).unwrap();
        let snapshot = p.reward_per_token;

        // A stale timestamp must not rewind the pool or double-credit.
        p.update_rewards(1_000).unwrap();
        assert_eq!(p.last_update_ts, 5_000);
        assert_eq!(p.reward_per_token, snapshot);
    }

    // -------------------------------------------------------------- earning

    #[test]
    fn single_staker_earns_the_full_emission() {
        let mut p = pool(100, 100_000);
        p.total_weighted = 1_000;
        let mut pos = position(1_000, LockTier::Flexible);

        p.update_rewards(10).unwrap();
        // 10 seconds * 100 per second, all to the only staker.
        assert_eq!(pos.earned(p.reward_per_token).unwrap(), 1_000);

        pos.settle(p.reward_per_token).unwrap();
        assert_eq!(pos.pending_rewards, 1_000);
        // Settling twice must not double-credit.
        pos.settle(p.reward_per_token).unwrap();
        assert_eq!(pos.pending_rewards, 1_000);
    }

    #[test]
    fn rewards_split_by_weight_not_by_principal() {
        let mut p = pool(300, 100_000);

        // Equal principal, different tiers: 1000 flexible (1.0x) and 1000 gold
        // (2.0x) give weights of 1000 and 2000.
        let flexible = position(
            LockTier::Flexible.apply_weight(1_000).unwrap(),
            LockTier::Flexible,
        );
        let gold = position(LockTier::Gold.apply_weight(1_000).unwrap(), LockTier::Gold);
        p.total_weighted = flexible.weighted_amount + gold.weighted_amount;
        assert_eq!(p.total_weighted, 3_000);

        p.update_rewards(10).unwrap(); // 3000 emitted

        assert_eq!(flexible.earned(p.reward_per_token).unwrap(), 1_000);
        assert_eq!(gold.earned(p.reward_per_token).unwrap(), 2_000);
    }

    #[test]
    fn a_position_earns_nothing_for_time_before_it_existed() {
        let mut p = pool(100, 100_000);
        p.total_weighted = 1_000;
        p.update_rewards(1_000).unwrap();

        // Joining later starts from the current watermark.
        let mut latecomer = position(1_000, LockTier::Flexible);
        latecomer.reward_per_token_paid = p.reward_per_token;
        assert_eq!(latecomer.earned(p.reward_per_token).unwrap(), 0);

        p.total_weighted = 2_000;
        p.update_rewards(1_010).unwrap(); // 1000 emitted, split evenly
        assert_eq!(latecomer.earned(p.reward_per_token).unwrap(), 500);
    }

    #[test]
    fn open_and_close_within_one_timestamp_earns_nothing() {
        // INVARIANTS.md §3.4 — the property a flash-loan reward attack needs.
        let mut p = pool(1_000_000, 100_000);
        p.total_weighted = 1_000;
        p.update_rewards(5_000).unwrap();

        let mut attacker = position(1_000_000, LockTier::Flexible);
        attacker.reward_per_token_paid = p.reward_per_token;

        p.total_weighted += attacker.weighted_amount;
        p.update_rewards(5_000).unwrap(); // same timestamp

        assert_eq!(attacker.earned(p.reward_per_token).unwrap(), 0);
    }

    // ------------------------------------------------------------- rounding

    #[test]
    fn rounding_always_favours_the_pool() {
        // INVARIANTS.md §3.3. Differential check: the sum actually payable to
        // every staker must never exceed what the pool emitted.
        //
        // Weights chosen to divide badly on purpose.
        let weights: [u64; 3] = [333, 7, 999_991];
        let total: u64 = weights.iter().sum();

        let mut p = pool(1_000, 1_000_000);
        p.total_weighted = total;

        let mut positions: Vec<Position> = weights
            .iter()
            .map(|w| position(*w, LockTier::Flexible))
            .collect();

        let mut emitted_total: u128 = 0;
        let mut t = 0i64;

        for _ in 0..50 {
            t += 7; // an interval that does not divide evenly
            emitted_total += 7 * p.reward_rate as u128;
            p.update_rewards(t).unwrap();
        }

        let paid_total: u128 = positions
            .iter_mut()
            .map(|pos| pos.earned(p.reward_per_token).unwrap() as u128)
            .sum();

        assert!(
            paid_total <= emitted_total,
            "pool would pay {paid_total} against {emitted_total} emitted"
        );
    }

    #[test]
    fn dust_weight_cannot_extract_more_than_it_earns() {
        // A single-unit position against an enormous pool earns zero rather
        // than being rounded up to one.
        let mut p = pool(1, 1_000_000);
        p.total_weighted = u64::MAX / 2;
        let dust = position(1, LockTier::Flexible);

        p.update_rewards(1).unwrap();
        assert_eq!(dust.earned(p.reward_per_token).unwrap(), 0);
    }

    // ---------------------------------------------------------------- gating

    #[test]
    fn lock_gate_rejects_freshly_opened_positions() {
        // INVARIANTS.md §4.2 / THREAT-MODEL A1.
        let now = 1_000i64;
        let voting_ends_at = now + 3 * DAY;

        let mut flash = position(1_000_000, LockTier::Flexible);
        flash.lock_end = now; // opened this instant, no commitment
        assert!(!flash.can_vote_until(voting_ends_at));

        let mut committed = position(1_000, LockTier::Gold);
        committed.lock_end = now + LockTier::Gold.duration();
        assert!(committed.can_vote_until(voting_ends_at));
    }

    #[test]
    fn a_lock_expiring_mid_vote_cannot_vote() {
        let now = 1_000i64;
        let voting_ends_at = now + 30 * DAY;

        let mut expiring = position(1_000, LockTier::Bronze);
        expiring.lock_end = voting_ends_at - 1; // one second short
        assert!(!expiring.can_vote_until(voting_ends_at));

        expiring.lock_end = voting_ends_at; // exactly enough
        assert!(expiring.can_vote_until(voting_ends_at));
    }

    #[test]
    fn unlock_boundary_is_inclusive() {
        let p = position(1_000, LockTier::Bronze);
        assert!(!p.is_unlocked(p.lock_end - 1));
        assert!(p.is_unlocked(p.lock_end));
    }

    // ------------------------------------------------------------- solvency

    #[test]
    fn emission_for_reports_the_full_commitment() {
        let p = pool(100, 1_000);
        assert_eq!(p.emission_for(100, 0).unwrap(), 100_000);
        assert_eq!(p.emission_for(100, 900).unwrap(), 10_000);
        // Past the end there is nothing left to commit.
        assert_eq!(p.emission_for(100, 1_000).unwrap(), 0);
        assert_eq!(p.emission_for(100, 5_000).unwrap(), 0);
    }
}
