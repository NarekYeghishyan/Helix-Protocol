//! Treasury and vesting-stream state.

use anchor_lang::prelude::*;

use crate::errors::TreasuryError;

/// A DAO-owned vault.
///
/// The vault authority is a PDA, so no key can move funds. Spending requires a
/// signature from [`Self::governance_executor`] — a PDA that only the governance
/// program can produce, and only inside the execution of a proposal that passed
/// quorum and cleared its timelock. That chain is the entire security model; see
/// `docs/THREAT-MODEL.md`.
#[account]
#[derive(InitSpace, Debug)]
pub struct Treasury {
    /// The only signer permitted to spend. Set at initialisation and changeable
    /// only by the current executor — so migrating governance is itself an act
    /// of governance.
    pub governance_executor: Pubkey,

    pub mint: Pubkey,
    pub vault: Pubkey,

    pub total_deposited: u64,
    pub total_spent: u64,

    /// Sum of the unclaimed remainder of every live vesting stream.
    ///
    /// Tracked so that `spend` cannot pay out tokens already promised to a
    /// beneficiary. Without this, a passed proposal could drain the vault and
    /// leave existing streams unfunded — the stream holder would discover it
    /// only when their claim failed (`INVARIANTS.md` §1.6).
    pub committed_to_streams: u64,

    /// Defence in depth against a malicious-but-passed proposal: even with a
    /// genuine majority, the treasury cannot be emptied in a single
    /// transaction. It buys time for the guardian veto and for holders to exit.
    pub epoch_duration: i64,
    pub epoch_spend_cap: u64,
    pub spent_this_epoch: u64,
    pub current_epoch: u64,

    /// Monotonic counter seeding stream PDAs.
    pub stream_count: u64,

    pub bump: u8,
    pub vault_authority_bump: u8,
}

impl Treasury {
    /// The epoch index containing `now`.
    pub fn epoch_at(&self, now: i64) -> Result<u64> {
        require!(self.epoch_duration > 0, TreasuryError::InvalidEpochDuration);
        Ok((now.max(0) as u64) / (self.epoch_duration as u64))
    }

    /// Charges `amount` against the current epoch's spend budget, rolling the
    /// window if `now` has moved past it.
    ///
    /// Returns an error without mutating if the spend would breach the cap.
    pub fn charge_epoch_budget(&mut self, amount: u64, now: i64) -> Result<()> {
        let epoch = self.epoch_at(now)?;
        // Compute against a local copy first so a rejected spend leaves no trace.
        let base = if epoch == self.current_epoch {
            self.spent_this_epoch
        } else {
            0
        };

        let used = base
            .checked_add(amount)
            .ok_or(TreasuryError::MathOverflow)?;
        require!(
            used <= self.epoch_spend_cap,
            TreasuryError::EpochSpendCapExceeded
        );

        self.current_epoch = epoch;
        self.spent_this_epoch = used;
        Ok(())
    }

    /// Remaining spend budget in the epoch containing `now`.
    pub fn remaining_budget(&self, now: i64) -> Result<u64> {
        if self.epoch_at(now)? != self.current_epoch {
            return Ok(self.epoch_spend_cap);
        }
        Ok(self.epoch_spend_cap.saturating_sub(self.spent_this_epoch))
    }

    /// Vault balance not already promised to vesting streams.
    pub fn uncommitted(&self, vault_balance: u64) -> u64 {
        vault_balance.saturating_sub(self.committed_to_streams)
    }
}

/// A linear vesting stream with an optional cliff.
///
/// Created only by governance, claimable only by the beneficiary, revocable only
/// by governance — and a revoke never claws back what has already vested.
#[account]
#[derive(InitSpace, Debug)]
pub struct VestingStream {
    pub treasury: Pubkey,
    pub beneficiary: Pubkey,
    pub stream_id: u64,

    pub total_amount: u64,
    pub claimed: u64,

    pub start_ts: i64,
    /// Nothing is claimable before this. Vesting still *accrues* from
    /// `start_ts`, so the cliff releases everything accrued up to that point at
    /// once — the standard "1 year cliff on a 4 year schedule" shape.
    pub cliff_ts: i64,
    pub end_ts: i64,

    pub revoked: bool,
    /// Timestamp of revocation. Vesting is evaluated as of this moment
    /// afterwards, which is what makes a revoke forward-only.
    pub revoked_at: i64,

    pub bump: u8,
}

impl VestingStream {
    /// Total vested as of `now`, ignoring what has been claimed.
    pub fn vested_at(&self, now: i64) -> Result<u64> {
        // A revoked stream is frozen at the moment of revocation. Evaluating it
        // at `now` instead would keep accruing after revocation; evaluating the
        // whole stream as zero would claw back tokens the beneficiary had
        // already earned. Freezing is the only option that does neither.
        let t = if self.revoked {
            now.min(self.revoked_at)
        } else {
            now
        };

        if t < self.cliff_ts {
            return Ok(0);
        }
        if t >= self.end_ts {
            return Ok(self.total_amount);
        }

        let elapsed = t
            .checked_sub(self.start_ts)
            .ok_or(TreasuryError::MathOverflow)?
            .max(0) as u128;
        let duration = self
            .end_ts
            .checked_sub(self.start_ts)
            .ok_or(TreasuryError::MathOverflow)? as u128;
        require!(duration > 0, TreasuryError::InvalidVestingSchedule);

        // Truncates, so vesting runs marginally slow rather than marginally
        // fast. The remainder is released by the `t >= end_ts` branch above, so
        // the beneficiary still receives exactly `total_amount` in the end.
        let vested = (self.total_amount as u128)
            .checked_mul(elapsed)
            .ok_or(TreasuryError::MathOverflow)?
            / duration;

        u64::try_from(vested).map_err(|_| TreasuryError::MathOverflow.into())
    }

    /// Vested but not yet withdrawn.
    pub fn claimable_at(&self, now: i64) -> Result<u64> {
        self.vested_at(now)?
            .checked_sub(self.claimed)
            .ok_or_else(|| TreasuryError::MathOverflow.into())
    }

    /// Amount that will never vest, given a revoke at `revoked_at`. This is what
    /// returns to the treasury's uncommitted balance.
    pub fn unvested_remainder(&self) -> Result<u64> {
        let vested = self.vested_at(self.revoked_at)?;
        self.total_amount
            .checked_sub(vested)
            .ok_or_else(|| TreasuryError::MathOverflow.into())
    }

    /// Still-owed amount, used to maintain [`Treasury::committed_to_streams`].
    pub fn outstanding(&self) -> Result<u64> {
        self.total_amount
            .checked_sub(self.claimed)
            .ok_or_else(|| TreasuryError::MathOverflow.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 86_400;
    const YEAR: i64 = 365 * DAY;

    fn treasury(cap: u64, epoch: i64) -> Treasury {
        Treasury {
            governance_executor: Pubkey::default(),
            mint: Pubkey::default(),
            vault: Pubkey::default(),
            total_deposited: 0,
            total_spent: 0,
            committed_to_streams: 0,
            epoch_duration: epoch,
            epoch_spend_cap: cap,
            spent_this_epoch: 0,
            current_epoch: 0,
            stream_count: 0,
            bump: 255,
            vault_authority_bump: 255,
        }
    }

    /// A 4-year stream of 4000 with a 1-year cliff, starting at t=0.
    fn stream() -> VestingStream {
        VestingStream {
            treasury: Pubkey::default(),
            beneficiary: Pubkey::default(),
            stream_id: 0,
            total_amount: 4_000,
            claimed: 0,
            start_ts: 0,
            cliff_ts: YEAR,
            end_ts: 4 * YEAR,
            revoked: false,
            revoked_at: 0,
            bump: 255,
        }
    }

    // -------------------------------------------------------- spend budget

    #[test]
    fn spend_budget_accrues_within_cap() {
        let mut t = treasury(1_000, DAY);
        t.charge_epoch_budget(600, 0).unwrap();
        t.charge_epoch_budget(400, 10).unwrap();
        assert_eq!(t.spent_this_epoch, 1_000);
        assert_eq!(t.remaining_budget(10).unwrap(), 0);
    }

    #[test]
    fn spend_over_cap_is_rejected_without_mutating() {
        let mut t = treasury(1_000, DAY);
        t.charge_epoch_budget(900, 0).unwrap();
        assert!(t.charge_epoch_budget(200, 0).is_err());
        assert_eq!(t.spent_this_epoch, 900);
    }

    #[test]
    fn spend_budget_rolls_over() {
        let mut t = treasury(1_000, DAY);
        t.charge_epoch_budget(1_000, 0).unwrap();
        assert!(t.charge_epoch_budget(1, 100).is_err());

        t.charge_epoch_budget(1_000, DAY).unwrap();
        assert_eq!(t.current_epoch, 1);
        assert_eq!(t.spent_this_epoch, 1_000);
    }

    #[test]
    fn idle_epochs_do_not_accumulate_budget() {
        let mut t = treasury(1_000, DAY);
        t.charge_epoch_budget(1_000, 0).unwrap();
        // Ten idle days must not grant ten days of allowance at once.
        assert!(t.charge_epoch_budget(1_001, 10 * DAY).is_err());
        t.charge_epoch_budget(1_000, 10 * DAY).unwrap();
    }

    // ------------------------------------------------------------ commitment

    #[test]
    fn uncommitted_excludes_stream_obligations() {
        let mut t = treasury(u64::MAX, DAY);
        t.committed_to_streams = 400;
        assert_eq!(t.uncommitted(1_000), 600);
        // A vault below its commitments has nothing free, and must not underflow.
        assert_eq!(t.uncommitted(300), 0);
    }

    // --------------------------------------------------------------- vesting

    #[test]
    fn nothing_vests_before_the_cliff() {
        let s = stream();
        assert_eq!(s.vested_at(0).unwrap(), 0);
        assert_eq!(s.vested_at(YEAR - 1).unwrap(), 0);
    }

    #[test]
    fn the_cliff_releases_everything_accrued_since_start() {
        let s = stream();
        // One year into a four-year linear schedule: a quarter, all at once.
        assert_eq!(s.vested_at(YEAR).unwrap(), 1_000);
    }

    #[test]
    fn vesting_is_linear_after_the_cliff() {
        let s = stream();
        assert_eq!(s.vested_at(2 * YEAR).unwrap(), 2_000);
        assert_eq!(s.vested_at(3 * YEAR).unwrap(), 3_000);
    }

    #[test]
    fn the_full_amount_vests_at_the_end_and_never_more() {
        let s = stream();
        assert_eq!(s.vested_at(4 * YEAR).unwrap(), 4_000);
        // Long past the end it stays at the total — no accrual beyond the term.
        assert_eq!(s.vested_at(40 * YEAR).unwrap(), 4_000);
    }

    #[test]
    fn vesting_truncates_in_the_treasurys_favour() {
        let mut s = stream();
        s.total_amount = 10;
        s.cliff_ts = 0;
        s.end_ts = 3;
        // 10 * 1 / 3 = 3.33 -> 3, never 4.
        assert_eq!(s.vested_at(1).unwrap(), 3);
        // The remainder is not lost: the endpoint releases the full amount.
        assert_eq!(s.vested_at(3).unwrap(), 10);
    }

    #[test]
    fn claimable_subtracts_what_was_already_taken() {
        let mut s = stream();
        s.claimed = 600;
        assert_eq!(s.claimable_at(YEAR).unwrap(), 400);
    }

    // -------------------------------------------------------------- revoking

    #[test]
    fn revoke_freezes_vesting_without_clawing_back() {
        let mut s = stream();
        s.revoked = true;
        s.revoked_at = 2 * YEAR;

        // Everything vested by the revoke stays claimable...
        assert_eq!(s.vested_at(2 * YEAR).unwrap(), 2_000);
        // ...and nothing accrues afterwards, however long we wait.
        assert_eq!(s.vested_at(4 * YEAR).unwrap(), 2_000);
        assert_eq!(s.claimable_at(10 * YEAR).unwrap(), 2_000);
    }

    #[test]
    fn revoke_before_the_cliff_vests_nothing() {
        let mut s = stream();
        s.revoked = true;
        s.revoked_at = YEAR / 2;
        assert_eq!(s.claimable_at(10 * YEAR).unwrap(), 0);
        assert_eq!(s.unvested_remainder().unwrap(), 4_000);
    }

    #[test]
    fn unvested_remainder_returns_to_the_treasury() {
        let mut s = stream();
        s.revoked = true;
        s.revoked_at = 3 * YEAR;
        assert_eq!(s.unvested_remainder().unwrap(), 1_000);
    }

    #[test]
    fn already_claimed_tokens_survive_a_revoke() {
        let mut s = stream();
        s.claimed = 1_000; // claimed at the cliff
        s.revoked = true;
        s.revoked_at = 2 * YEAR;

        // Vested 2000, of which 1000 is already in hand.
        assert_eq!(s.claimable_at(4 * YEAR).unwrap(), 1_000);
        assert_eq!(s.outstanding().unwrap(), 3_000);
    }

    #[test]
    fn outstanding_tracks_the_unclaimed_remainder() {
        let mut s = stream();
        assert_eq!(s.outstanding().unwrap(), 4_000);
        s.claimed = 4_000;
        assert_eq!(s.outstanding().unwrap(), 0);
    }
}
