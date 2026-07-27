//! Finalisation and the timelock queue.

use anchor_lang::prelude::*;

use crate::constants::{PROPOSAL_SEED, REALM_SEED};
use crate::errors::GovernanceError;
use crate::events::{ProposalFinalized, ProposalQueued};
use crate::state::{Proposal, ProposalState, Realm};

#[derive(Accounts)]
pub struct AdvanceProposal<'info> {
    #[account(
        seeds = [REALM_SEED, realm.staking_pool.as_ref()],
        bump = realm.bump,
    )]
    pub realm: Account<'info, Realm>,

    #[account(
        mut,
        seeds = [
            PROPOSAL_SEED,
            realm.key().as_ref(),
            proposal.id.to_le_bytes().as_ref(),
        ],
        bump = proposal.bump,
        has_one = realm,
    )]
    pub proposal: Account<'info, Proposal>,
}

/// Resolves a closed vote into `Succeeded` or `Defeated`.
///
/// Permissionless. Anyone may finalise, because the outcome is a pure function of
/// state already on chain — there is nothing to decide, only to record. Making
/// this permissioned would let whoever held the permission strand a proposal they
/// disliked in `Voting` forever.
pub fn finalize_proposal(ctx: Context<AdvanceProposal>) -> Result<()> {
    ctx.accounts.proposal.require_state(ProposalState::Voting)?;

    let now = Clock::get()?.unix_timestamp;
    require!(
        now >= ctx.accounts.proposal.voting_ends_at,
        GovernanceError::VotingStillOpen
    );

    let outcome = ctx.accounts.proposal.outcome(
        ctx.accounts.realm.quorum_bps,
        ctx.accounts.realm.approval_bps,
    )?;

    let proposal = &mut ctx.accounts.proposal;
    proposal.state = outcome;

    emit!(ProposalFinalized {
        proposal: proposal.key(),
        outcome,
        for_votes: proposal.for_votes,
        against_votes: proposal.against_votes,
        abstain_votes: proposal.abstain_votes,
        total_weight_snapshot: proposal.total_weight_snapshot,
        timestamp: now,
    });

    Ok(())
}

/// Moves a passed proposal into the timelock.
///
/// `eta` is computed here, from the clock at this moment, and never recomputed.
/// A later change to `realm.timelock_delay` therefore cannot shorten the delay on
/// something already queued — which is what stops the timelock from being
/// bypassed by governing the timelock itself.
pub fn queue_proposal(ctx: Context<AdvanceProposal>) -> Result<()> {
    ctx.accounts
        .proposal
        .require_state(ProposalState::Succeeded)?;

    let now = Clock::get()?.unix_timestamp;
    let eta = now
        .checked_add(ctx.accounts.realm.timelock_delay)
        .ok_or(GovernanceError::MathOverflow)?;

    let proposal = &mut ctx.accounts.proposal;
    proposal.state = ProposalState::Queued;
    proposal.eta = eta;

    let expires_at = proposal.expires_at()?;

    emit!(ProposalQueued {
        proposal: proposal.key(),
        eta,
        expires_at,
        timestamp: now,
    });

    Ok(())
}
