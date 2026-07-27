//! Program state.
//!
//! Both accounts derive [`InitSpace`] so that account sizing is computed by the
//! macro from the field types rather than by a hand-maintained constant. A
//! hand-counted `space =` is a silent corruption waiting to happen the first
//! time someone adds a field.

use anchor_lang::prelude::*;

use crate::errors::TokenManagerError;

/// Singleton configuration for the HLX mint. PDA: `["config", mint]`.
#[account]
#[derive(InitSpace, Debug)]
pub struct TokenConfig {
    /// May register minters and pause deposits. Cannot mint, and cannot move
    /// funds anywhere.
    pub admin: Pubkey,

    /// Set by `propose_admin`, cleared by `accept_admin`. Admin transfer is
    /// deliberately two-step: a one-step transfer to a mistyped address is
    /// unrecoverable.
    pub pending_admin: Option<Pubkey>,

    /// The HLX mint this config governs.
    pub mint: Pubkey,

    /// Bump for `["mint_authority", config]`, stored so every later derivation
    /// uses the canonical bump rather than searching for one.
    pub mint_authority_bump: u8,

    /// Bump for this account's own PDA.
    pub bump: u8,

    /// When true, `mint_to` is rejected. Burning stays available — a pause that
    /// blocks the exit path is indistinguishable from a freeze.
    pub paused: bool,

    /// Lifetime issuance and redemption, for analytics and for reconciling the
    /// indexer against chain state.
    pub total_minted: u64,
    pub total_burned: u64,

    /// Number of registered minters, bounded by [`crate::constants::MAX_MINTERS`].
    pub minter_count: u16,
}

impl TokenConfig {
    /// Circulating supply as this program understands it.
    pub fn circulating(&self) -> Result<u64> {
        self.total_minted
            .checked_sub(self.total_burned)
            .ok_or_else(|| TokenManagerError::MathOverflow.into())
    }
}

/// A registered issuer of HLX. PDA: `["minter", config, authority]`.
///
/// The registry is what makes the mint authority safe to hold in a PDA: the PDA
/// will sign an issuance, but only on behalf of an authority recorded here and
/// only within that authority's cap for the current epoch. In the deployed
/// system the staking program's reward PDA is the only entry.
#[account]
#[derive(InitSpace, Debug)]
pub struct Minter {
    /// The config this entry belongs to. Checked with `has_one` so a minter
    /// from one deployment cannot be replayed against another.
    pub config: Pubkey,

    /// The signer permitted to request issuance through this entry.
    pub authority: Pubkey,

    /// Maximum that may be minted through this entry per epoch.
    pub epoch_cap: u64,

    /// Amount minted so far in [`Self::current_epoch`].
    pub minted_this_epoch: u64,

    /// Index of the epoch [`Self::minted_this_epoch`] refers to, derived from
    /// the chain clock as `unix_timestamp / epoch_duration`.
    pub current_epoch: u64,

    /// Length of an issuance epoch in seconds.
    pub epoch_duration: i64,

    /// Lifetime issuance through this entry.
    pub total_minted: u64,

    /// Cleared by `revoke_minter`. Revocation disables rather than closes, so
    /// the historical `total_minted` stays auditable on chain.
    pub enabled: bool,

    pub bump: u8,
}

impl Minter {
    /// The epoch index containing `now`.
    pub fn epoch_at(&self, now: i64) -> Result<u64> {
        // `epoch_duration` is validated positive at registration, so this
        // division cannot trap; the cast is safe because `now` is non-negative
        // for any realistic chain clock.
        let duration = self.epoch_duration;
        require!(duration > 0, TokenManagerError::InvalidEpochDuration);
        Ok((now.max(0) as u64) / (duration as u64))
    }

    /// Records an issuance of `amount`, rolling the epoch window if `now` has
    /// moved past it.
    ///
    /// Returns [`TokenManagerError::EpochCapExceeded`] without mutating if the
    /// issuance would breach the cap.
    pub fn accrue(&mut self, amount: u64, now: i64) -> Result<()> {
        require!(self.enabled, TokenManagerError::MinterDisabled);

        let epoch = self.epoch_at(now)?;
        if epoch != self.current_epoch {
            self.current_epoch = epoch;
            self.minted_this_epoch = 0;
        }

        let used = self
            .minted_this_epoch
            .checked_add(amount)
            .ok_or(TokenManagerError::MathOverflow)?;
        require!(used <= self.epoch_cap, TokenManagerError::EpochCapExceeded);

        self.minted_this_epoch = used;
        self.total_minted = self
            .total_minted
            .checked_add(amount)
            .ok_or(TokenManagerError::MathOverflow)?;

        Ok(())
    }

    /// Remaining issuance available in the epoch containing `now`.
    pub fn remaining_this_epoch(&self, now: i64) -> Result<u64> {
        if !self.enabled {
            return Ok(0);
        }
        // A future epoch resets the window, so the full cap is available.
        if self.epoch_at(now)? != self.current_epoch {
            return Ok(self.epoch_cap);
        }
        Ok(self.epoch_cap.saturating_sub(self.minted_this_epoch))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minter(cap: u64, duration: i64) -> Minter {
        Minter {
            config: Pubkey::default(),
            authority: Pubkey::default(),
            epoch_cap: cap,
            minted_this_epoch: 0,
            current_epoch: 0,
            epoch_duration: duration,
            total_minted: 0,
            enabled: true,
            bump: 255,
        }
    }

    #[test]
    fn accrues_within_cap() {
        let mut m = minter(1_000, 3_600);
        m.accrue(600, 0).unwrap();
        m.accrue(400, 10).unwrap();
        assert_eq!(m.minted_this_epoch, 1_000);
        assert_eq!(m.total_minted, 1_000);
    }

    #[test]
    fn rejects_over_cap_without_mutating() {
        let mut m = minter(1_000, 3_600);
        m.accrue(900, 0).unwrap();
        assert!(m.accrue(200, 0).is_err());
        // The rejected issuance must leave no trace.
        assert_eq!(m.minted_this_epoch, 900);
        assert_eq!(m.total_minted, 900);
    }

    #[test]
    fn rolls_over_to_a_new_epoch() {
        let mut m = minter(1_000, 3_600);
        m.accrue(1_000, 0).unwrap();
        assert!(m.accrue(1, 100).is_err());

        // One hour later the window resets, but lifetime issuance does not.
        m.accrue(1_000, 3_600).unwrap();
        assert_eq!(m.current_epoch, 1);
        assert_eq!(m.minted_this_epoch, 1_000);
        assert_eq!(m.total_minted, 2_000);
    }

    #[test]
    fn skipping_epochs_does_not_accumulate_allowance() {
        let mut m = minter(1_000, 3_600);
        m.accrue(1_000, 0).unwrap();

        // Ten idle epochs must not grant ten epochs' worth of headroom.
        assert!(m.accrue(1_001, 36_000).is_err());
        m.accrue(1_000, 36_000).unwrap();
        assert_eq!(m.minted_this_epoch, 1_000);
    }

    #[test]
    fn disabled_minter_cannot_accrue() {
        let mut m = minter(1_000, 3_600);
        m.enabled = false;
        assert!(m.accrue(1, 0).is_err());
        assert_eq!(m.remaining_this_epoch(0).unwrap(), 0);
    }

    #[test]
    fn remaining_reports_full_cap_in_a_future_epoch() {
        let mut m = minter(1_000, 3_600);
        m.accrue(750, 0).unwrap();
        assert_eq!(m.remaining_this_epoch(0).unwrap(), 250);
        assert_eq!(m.remaining_this_epoch(7_200).unwrap(), 1_000);
    }
}
