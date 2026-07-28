//! Governance retuning and re-owning itself.
//!
//! Kept apart from `execute.rs` and `execute_token.rs` because it is a third
//! authority chain, and the only one that is reflexive: the realm is both the
//! account being changed and the source of the authority to change it.
//!
//! # Why these exist
//!
//! `update_realm_params` is gated on `realm.authority`, which is set once at
//! `initialize_realm`. Before these handlers there was no `ProposalAction` that
//! could produce that signature and no instruction that could move the authority
//! anywhere — so the parameters defining what "passing" *means* (quorum,
//! approval, voting period, timelock, the proposal threshold) belonged
//! permanently to a key outside governance.
//!
//! That is not a governance system with an admin key attached, it is an admin key
//! with a governance system attached: whoever held `realm.authority` could lower
//! quorum to a hundredth of a percent and then pass anything, treasury transfers
//! included, with a dust position. See F-11 and
//! `lowering_quorum_lets_a_dust_position_move_the_treasury`.
//!
//! Both handlers mutate the realm directly rather than through a CPI. The realm
//! belongs to this program, so signing as the executor to call ourselves would be
//! ceremony without a check attached — `authorize_execution` is the gate, and it
//! is the same gate every other action passes through.

use anchor_lang::prelude::*;

use crate::constants::{EXECUTOR_SEED, PROPOSAL_SEED, REALM_SEED};
use crate::errors::GovernanceError;
use crate::events::{ProposalExecuted, RealmAuthorityChanged, RealmParamsUpdated};
use crate::instructions::execute::authorize_execution;
use crate::state::{Proposal, ProposalAction, Realm};

#[derive(Accounts)]
pub struct ExecuteRealmConfig<'info> {
    #[account(
        mut,
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

    /// CHECK: identity fixed by seeds. Present so the executed action is visibly
    /// attributed to this realm's executor even though no CPI is made.
    #[account(
        seeds = [EXECUTOR_SEED, realm.key().as_ref()],
        bump,
    )]
    pub executor: UncheckedAccount<'info>,
}

/// Applies a `ProposalAction::UpdateRealmParams`.
pub fn execute_update_realm_params(ctx: Context<ExecuteRealmConfig>) -> Result<()> {
    let ProposalAction::UpdateRealmParams { params } = ctx.accounts.proposal.action else {
        return err!(GovernanceError::ActionAccountMismatch);
    };

    // Validated before the proposal is marked executed, so a proposal carrying
    // parameters the program would refuse from a direct caller fails here and
    // stays executable rather than burning itself on an invalid change. Routing
    // through governance must not be a way to bypass the floors — least of all
    // MIN_APPROVAL_BPS, which is the only thing keeping `for > against` true.
    params.validate()?;

    let now = Clock::get()?.unix_timestamp;
    authorize_execution(&mut ctx.accounts.proposal, now)?;

    let realm = &mut ctx.accounts.realm;
    realm.quorum_bps = params.quorum_bps;
    realm.approval_bps = params.approval_bps;
    realm.voting_period = params.voting_period;
    realm.timelock_delay = params.timelock_delay;
    realm.min_weight_to_propose = params.min_weight_to_propose;

    emit!(RealmParamsUpdated {
        realm: realm.key(),
        by_proposal: true,
        quorum_bps: params.quorum_bps,
        approval_bps: params.approval_bps,
        voting_period: params.voting_period,
        timelock_delay: params.timelock_delay,
        min_weight_to_propose: params.min_weight_to_propose,
        timestamp: now,
    });
    emit!(ProposalExecuted {
        proposal: ctx.accounts.proposal.key(),
        action: ctx.accounts.proposal.action,
        timestamp: now,
    });

    Ok(())
}

/// Applies a `ProposalAction::SetRealmAuthority`.
///
/// The migration in ROADMAP Phase 7.1. Pointing the authority at the realm's own
/// executor PDA means the parameters answer only to a passed, timelocked
/// proposal — and `execute_update_realm_params` above is what keeps them
/// reachable afterwards. Handing the authority to an address that cannot be made
/// to sign would freeze the realm's configuration permanently, which is the same
/// defect as F-8 and F-9 one step further along.
pub fn execute_set_realm_authority(ctx: Context<ExecuteRealmConfig>) -> Result<()> {
    let ProposalAction::SetRealmAuthority { new_authority } = ctx.accounts.proposal.action else {
        return err!(GovernanceError::ActionAccountMismatch);
    };

    let now = Clock::get()?.unix_timestamp;
    authorize_execution(&mut ctx.accounts.proposal, now)?;

    let executor = ctx.accounts.executor.key();
    let realm = &mut ctx.accounts.realm;
    let previous_authority = realm.authority;
    realm.authority = new_authority;

    emit!(RealmAuthorityChanged {
        realm: realm.key(),
        previous_authority,
        new_authority,
        self_governing: new_authority == executor,
        timestamp: now,
    });
    emit!(ProposalExecuted {
        proposal: ctx.accounts.proposal.key(),
        action: ctx.accounts.proposal.action,
        timestamp: now,
    });

    Ok(())
}
