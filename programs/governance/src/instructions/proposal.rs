//! Proposal creation, activation, and the guardian veto.

use anchor_lang::prelude::*;
use helix_staking::state::{Pool, Position};

use crate::constants::{MAX_TITLE_LEN, MAX_URI_LEN, PROPOSAL_SEED, REALM_SEED};
use crate::errors::GovernanceError;
use crate::events::{ProposalActivated, ProposalCancelled, ProposalCreated};
use crate::state::{Proposal, ProposalAction, ProposalState, Realm};

#[derive(Accounts)]
#[instruction(proposal_id: u64)]
pub struct CreateProposal<'info> {
    #[account(
        mut,
        seeds = [REALM_SEED, realm.staking_pool.as_ref()],
        bump = realm.bump,
    )]
    pub realm: Account<'info, Realm>,

    #[account(mut)]
    pub proposer: Signer<'info>,

    /// A position held by the proposer, used only to prove they meet
    /// `min_weight_to_propose`. It is not consumed and does not vote.
    #[account(
        has_one = owner @ GovernanceError::NotPositionOwner,
        constraint = proposer_position.pool == realm.staking_pool @ GovernanceError::PoolMismatch,
    )]
    pub proposer_position: Box<Account<'info, Position>>,

    /// CHECK: `has_one = owner` above ties this to the position; it must also be
    /// the signer, which is enforced in the handler.
    pub owner: UncheckedAccount<'info>,

    #[account(
        init,
        payer = proposer,
        space = 8 + Proposal::INIT_SPACE,
        seeds = [
            PROPOSAL_SEED,
            realm.key().as_ref(),
            proposal_id.to_le_bytes().as_ref(),
        ],
        bump,
    )]
    pub proposal: Account<'info, Proposal>,

    pub system_program: Program<'info, System>,
}

/// Creates a proposal in `Draft`.
///
/// Draft exists so that the quorum snapshot is taken at *activation* rather than
/// creation, which gives the proposal text a window to be read before the clock
/// starts running.
pub fn create_proposal(
    ctx: Context<CreateProposal>,
    proposal_id: u64,
    action: ProposalAction,
    title: String,
    descriptor_uri: String,
) -> Result<()> {
    require!(
        title.len() <= MAX_TITLE_LEN && descriptor_uri.len() <= MAX_URI_LEN,
        GovernanceError::TextTooLong
    );
    require_keys_eq!(
        ctx.accounts.owner.key(),
        ctx.accounts.proposer.key(),
        GovernanceError::NotPositionOwner
    );
    require_eq!(
        proposal_id,
        ctx.accounts.realm.proposal_count,
        GovernanceError::MathOverflow
    );

    // Spam pricing: proposing costs weight, not just rent.
    require!(
        ctx.accounts.proposer_position.weighted_amount >= ctx.accounts.realm.min_weight_to_propose,
        GovernanceError::BelowProposalThreshold
    );

    let now = Clock::get()?.unix_timestamp;

    let proposal = &mut ctx.accounts.proposal;
    proposal.realm = ctx.accounts.realm.key();
    proposal.proposer = ctx.accounts.proposer.key();
    proposal.id = proposal_id;
    proposal.state = ProposalState::Draft;
    proposal.action = action;
    proposal.title = title.clone();
    proposal.descriptor_uri = descriptor_uri;
    proposal.created_at = now;
    proposal.voting_starts_at = 0;
    proposal.voting_ends_at = 0;
    proposal.eta = 0;
    proposal.for_votes = 0;
    proposal.against_votes = 0;
    proposal.abstain_votes = 0;
    proposal.total_weight_snapshot = 0;
    proposal.bump = ctx.bumps.proposal;

    let realm = &mut ctx.accounts.realm;
    realm.proposal_count = realm
        .proposal_count
        .checked_add(1)
        .ok_or(GovernanceError::MathOverflow)?;

    emit!(ProposalCreated {
        realm: realm.key(),
        proposal: proposal.key(),
        proposer: proposal.proposer,
        id: proposal_id,
        action,
        title,
        timestamp: now,
    });

    Ok(())
}

#[derive(Accounts)]
pub struct ActivateProposal<'info> {
    #[account(
        seeds = [REALM_SEED, realm.staking_pool.as_ref()],
        bump = realm.bump,
        has_one = staking_pool,
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

    /// Read to snapshot `total_weighted` as the quorum denominator.
    pub staking_pool: Account<'info, Pool>,
}

/// Opens voting and fixes the quorum denominator.
///
/// Permissionless — anyone may start the clock on a draft. Restricting it to the
/// proposer would let a proposer sit on an unactivated draft indefinitely, and
/// there is nothing to gain by activating someone else's proposal early.
///
/// The snapshot is taken here and never re-read. Reading `total_weighted` live at
/// finalisation instead would let a whale defeat a proposal by staking more after
/// seeing how the vote was going, inflating the denominator until quorum failed.
pub fn activate_proposal(ctx: Context<ActivateProposal>) -> Result<()> {
    ctx.accounts.proposal.require_state(ProposalState::Draft)?;

    let now = Clock::get()?.unix_timestamp;
    let voting_ends_at = now
        .checked_add(ctx.accounts.realm.voting_period)
        .ok_or(GovernanceError::MathOverflow)?;

    let snapshot = ctx.accounts.staking_pool.total_weighted;
    // With nothing staked there is no denominator, so no meaningful quorum.
    require!(snapshot > 0, GovernanceError::MissingSnapshot);

    let proposal = &mut ctx.accounts.proposal;
    proposal.state = ProposalState::Voting;
    proposal.voting_starts_at = now;
    proposal.voting_ends_at = voting_ends_at;
    proposal.total_weight_snapshot = snapshot;

    emit!(ProposalActivated {
        proposal: proposal.key(),
        voting_starts_at: now,
        voting_ends_at,
        total_weight_snapshot: snapshot,
        timestamp: now,
    });

    Ok(())
}

#[derive(Accounts)]
pub struct CancelProposal<'info> {
    #[account(
        seeds = [REALM_SEED, realm.staking_pool.as_ref()],
        bump = realm.bump,
        has_one = guardian @ GovernanceError::NotGuardian,
    )]
    pub realm: Account<'info, Realm>,

    pub guardian: Signer<'info>,

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

/// Vetoes a proposal before it executes.
///
/// This is the guardian's only power. It cannot create, activate, pass, queue or
/// execute anything — a guardian that could also *pass* proposals would not be a
/// safety mechanism, it would be an admin key with a reassuring name.
///
/// Vetoing an already-executed proposal is refused: the effect has happened, and
/// recording it as cancelled would misreport what the chain did.
pub fn cancel_proposal(ctx: Context<CancelProposal>) -> Result<()> {
    let proposal = &mut ctx.accounts.proposal;
    require!(
        proposal.state.is_cancellable(),
        GovernanceError::InvalidProposalState
    );

    let previous_state = proposal.state;
    proposal.state = ProposalState::Cancelled;

    emit!(ProposalCancelled {
        proposal: proposal.key(),
        guardian: ctx.accounts.guardian.key(),
        previous_state,
        timestamp: Clock::get()?.unix_timestamp,
    });

    Ok(())
}
