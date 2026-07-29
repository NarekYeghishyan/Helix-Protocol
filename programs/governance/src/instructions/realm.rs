//! Realm creation and parameter changes.

use anchor_lang::prelude::*;
use helix_staking::state::Pool;

use crate::constants::{
    EXECUTOR_SEED, MAX_BPS, MAX_TIMELOCK_DELAY, MAX_VOTING_PERIOD, MIN_APPROVAL_BPS,
    MIN_TIMELOCK_DELAY, MIN_VOTING_PERIOD, REALM_SEED,
};
use crate::errors::GovernanceError;
use crate::events::{RealmInitialized, RealmParamsUpdated};
use crate::state::Realm;

/// Parameters shared by realm creation and parameter updates.
/// Derives `InitSpace` because `ProposalAction::UpdateRealmParams` carries one,
/// and a proposal's action is stored in the account.
#[derive(AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, PartialEq, Eq, Debug)]
pub struct RealmParams {
    pub quorum_bps: u16,
    pub approval_bps: u16,
    pub voting_period: i64,
    pub timelock_delay: i64,
    pub min_weight_to_propose: u64,
}

impl RealmParams {
    pub fn validate(&self) -> Result<()> {
        require!(
            self.quorum_bps > 0 && self.quorum_bps <= MAX_BPS,
            GovernanceError::InvalidQuorum
        );
        // A sub-majority approval threshold would let a proposal pass with more
        // weight against it than for it.
        require!(
            self.approval_bps >= MIN_APPROVAL_BPS && self.approval_bps <= MAX_BPS,
            GovernanceError::InvalidApprovalThreshold
        );
        require!(
            (MIN_VOTING_PERIOD..=MAX_VOTING_PERIOD).contains(&self.voting_period),
            GovernanceError::InvalidVotingPeriod
        );
        // The timelock floor is not zero: the delay is the window in which
        // holders who dislike a passed proposal can exit before it takes effect.
        require!(
            (MIN_TIMELOCK_DELAY..=MAX_TIMELOCK_DELAY).contains(&self.timelock_delay),
            GovernanceError::InvalidTimelockDelay
        );
        Ok(())
    }
}

#[derive(Accounts)]
pub struct InitializeRealm<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,

    /// CHECK: stored as configuration. Intended end state is the realm's own
    /// executor PDA, so parameter changes go through a vote like anything else.
    pub authority: UncheckedAccount<'info>,

    /// CHECK: stored as configuration. May only veto — see `cancel_proposal`.
    pub guardian: UncheckedAccount<'info>,

    /// The pool whose positions confer vote weight. Typed, so an account that is
    /// not actually a staking pool cannot be passed.
    pub staking_pool: Account<'info, Pool>,

    #[account(
        init,
        payer = payer,
        space = 8 + Realm::INIT_SPACE,
        seeds = [REALM_SEED, staking_pool.key().as_ref()],
        bump,
    )]
    pub realm: Account<'info, Realm>,

    /// CHECK: the signer for executed proposals. Never deserialised; identity
    /// fixed by seeds. Possession of this PDA is the right to spend the treasury,
    /// and only `execute_*` can produce it.
    #[account(
        seeds = [EXECUTOR_SEED, realm.key().as_ref()],
        bump,
    )]
    pub executor: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

pub fn initialize_realm(ctx: Context<InitializeRealm>, params: RealmParams) -> Result<()> {
    params.validate()?;

    let now = Clock::get()?.unix_timestamp;
    let realm = &mut ctx.accounts.realm;

    realm.authority = ctx.accounts.authority.key();
    realm.guardian = ctx.accounts.guardian.key();
    realm.staking_pool = ctx.accounts.staking_pool.key();

    realm.quorum_bps = params.quorum_bps;
    realm.approval_bps = params.approval_bps;
    realm.voting_period = params.voting_period;
    realm.timelock_delay = params.timelock_delay;
    realm.min_weight_to_propose = params.min_weight_to_propose;

    realm.proposal_count = 0;
    realm.bump = ctx.bumps.realm;
    realm.executor_bump = ctx.bumps.executor;

    emit!(RealmInitialized {
        realm: realm.key(),
        authority: realm.authority,
        guardian: realm.guardian,
        staking_pool: realm.staking_pool,
        quorum_bps: realm.quorum_bps,
        approval_bps: realm.approval_bps,
        voting_period: realm.voting_period,
        timelock_delay: realm.timelock_delay,
        min_weight_to_propose: realm.min_weight_to_propose,
        timestamp: now,
    });

    Ok(())
}

#[derive(Accounts)]
pub struct UpdateRealmParams<'info> {
    #[account(
        mut,
        seeds = [REALM_SEED, realm.staking_pool.as_ref()],
        bump = realm.bump,
        has_one = authority @ GovernanceError::NotAuthority,
    )]
    pub realm: Account<'info, Realm>,

    pub authority: Signer<'info>,
}

/// Changes governance parameters.
///
/// A changed `timelock_delay` applies only to proposals queued afterwards —
/// `eta` is computed at queue time and never recomputed, so this cannot be used
/// to shorten the delay on something already waiting.
pub fn update_realm_params(ctx: Context<UpdateRealmParams>, params: RealmParams) -> Result<()> {
    params.validate()?;

    let realm = &mut ctx.accounts.realm;
    realm.quorum_bps = params.quorum_bps;
    realm.approval_bps = params.approval_bps;
    realm.voting_period = params.voting_period;
    realm.timelock_delay = params.timelock_delay;
    realm.min_weight_to_propose = params.min_weight_to_propose;

    // Emitted with `by_proposal: false`, which is the point of the flag. Until
    // `realm.authority` is migrated to the executor PDA, this instruction is a
    // way to change what "passing" means without anything passing — and an
    // observer watching the log should be able to tell the two apart. See F-11.
    emit!(RealmParamsUpdated {
        realm: realm.key(),
        by_proposal: false,
        quorum_bps: params.quorum_bps,
        approval_bps: params.approval_bps,
        voting_period: params.voting_period,
        timelock_delay: params.timelock_delay,
        min_weight_to_propose: params.min_weight_to_propose,
        timestamp: Clock::get()?.unix_timestamp,
    });

    Ok(())
}
