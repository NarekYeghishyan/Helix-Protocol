//! Casting a vote.
//!
//! This file contains the whole of the flash-loan defence, and it is one
//! comparison. See [`cast_vote`].

use anchor_lang::prelude::*;
use helix_staking::state::Position;

use crate::constants::{PROPOSAL_SEED, REALM_SEED, VOTE_SEED};
use crate::errors::GovernanceError;
use crate::events::VoteCast;
use crate::state::{Proposal, ProposalState, Realm, VoteChoice, VoteRecord};

#[derive(Accounts)]
pub struct CastVote<'info> {
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

    #[account(mut)]
    pub voter: Signer<'info>,

    /// The staking position whose weight is being cast.
    ///
    /// Typed as `Account<Position>`, so Anchor verifies the account is owned by
    /// the staking program before this handler sees it — a look-alike account
    /// with a forged `weighted_amount` cannot be substituted.
    #[account(
        constraint = position.owner == voter.key() @ GovernanceError::NotPositionOwner,
        constraint = position.pool == realm.staking_pool @ GovernanceError::PoolMismatch,
    )]
    pub position: Box<Account<'info, Position>>,

    /// One record per (proposal, position). `init` makes a second vote from the
    /// same position fail at account creation, before any handler logic runs —
    /// double voting is structurally impossible rather than checked for.
    #[account(
        init,
        payer = voter,
        space = 8 + VoteRecord::INIT_SPACE,
        seeds = [VOTE_SEED, proposal.key().as_ref(), position.key().as_ref()],
        bump,
    )]
    pub vote_record: Account<'info, VoteRecord>,

    pub system_program: Program<'info, System>,
}

/// Casts `position`'s weight for `choice`.
///
/// # Why flash loans do not work here
///
/// The standard attack on token governance is to borrow a large balance, vote,
/// and repay, all in one transaction. Two common defences are historical
/// snapshots (expensive on Solana, and gameable by borrowing *before* the
/// snapshot block) and vote-escrow.
///
/// Helix uses neither. Instead:
///
/// > a position may only vote if `lock_end >= proposal.voting_ends_at`.
///
/// You can only vote with stake you are contractually unable to withdraw before
/// the vote closes. A position created inside the attacking transaction has
/// `lock_end` at or near `now`, which is strictly less than any live proposal's
/// end, so borrowed capital carries exactly zero weight.
///
/// This is stronger than a snapshot — the capital must stay locked across the
/// entire voting window, not merely exist at one block — and it costs one
/// comparison, because the staking design was chosen to make it cheap.
pub fn cast_vote(ctx: Context<CastVote>, choice: VoteChoice) -> Result<()> {
    ctx.accounts.proposal.require_state(ProposalState::Voting)?;

    let now = Clock::get()?.unix_timestamp;
    require!(
        now >= ctx.accounts.proposal.voting_starts_at,
        GovernanceError::VotingNotStarted
    );
    require!(
        now < ctx.accounts.proposal.voting_ends_at,
        GovernanceError::VotingEnded
    );

    // ---- the flash-loan gate --------------------------------------------
    require!(
        ctx.accounts
            .position
            .can_vote_until(ctx.accounts.proposal.voting_ends_at),
        GovernanceError::InsufficientLockDuration
    );
    // --------------------------------------------------------------------

    let weight = ctx.accounts.position.weighted_amount;
    // A zero-weight position would create a vote record that consumes rent and
    // says nothing. Refusing keeps the record set meaningful.
    require!(weight > 0, GovernanceError::ZeroWeight);

    ctx.accounts.proposal.tally(choice, weight)?;

    let vote_record = &mut ctx.accounts.vote_record;
    vote_record.proposal = ctx.accounts.proposal.key();
    vote_record.position = ctx.accounts.position.key();
    vote_record.voter = ctx.accounts.voter.key();
    vote_record.choice = choice;
    // Recorded so the tally can be audited after the fact, and so a later change
    // to the position's weight cannot retroactively change what it voted with.
    vote_record.weight = weight;
    vote_record.voted_at = now;
    vote_record.bump = ctx.bumps.vote_record;

    let proposal = &ctx.accounts.proposal;
    emit!(VoteCast {
        proposal: proposal.key(),
        position: vote_record.position,
        voter: vote_record.voter,
        choice,
        weight,
        for_votes: proposal.for_votes,
        against_votes: proposal.against_votes,
        abstain_votes: proposal.abstain_votes,
        timestamp: now,
    });

    Ok(())
}
