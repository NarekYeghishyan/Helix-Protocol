//! Executing a passed, timelocked proposal.
//!
//! There is one instruction per [`ProposalAction`] variant rather than a single
//! `execute` taking `remaining_accounts`. It costs some repetition, and buys two
//! things worth more than the repetition: the accounts each action touches are
//! visible in the IDL, and every account is a *typed* Anchor account rather than
//! a bare `AccountInfo` that the handler has to validate by hand.

use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

use crate::constants::{EXECUTOR_SEED, PROPOSAL_SEED, REALM_SEED};
use crate::errors::GovernanceError;
use crate::events::ProposalExecuted;
use crate::state::{Proposal, ProposalAction, ProposalState, Realm};

/// Common gate for every execution path.
///
/// Sets `Executed` **before** the caller performs any CPI. Marking afterwards
/// would leave a window in which a re-entrant call could observe the proposal
/// still `Queued` and execute it a second time (`INVARIANTS.md` §4.5).
fn authorize_execution(proposal: &mut Proposal, now: i64) -> Result<()> {
    proposal.require_state(ProposalState::Queued)?;

    require!(now >= proposal.eta, GovernanceError::TimelockNotElapsed);
    // A queued proposal that nobody executed eventually expires. Without this, a
    // proposal passed under one set of conditions could lie dormant and then be
    // executed into a completely different world a year later.
    require!(
        now <= proposal.expires_at()?,
        GovernanceError::ProposalExpired
    );

    proposal.state = ProposalState::Executed;
    Ok(())
}

// ---------------------------------------------------------------- Signal

#[derive(Accounts)]
pub struct ExecuteSignal<'info> {
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

/// Records the outcome of a signalling proposal. Moves nothing.
pub fn execute_signal(ctx: Context<ExecuteSignal>) -> Result<()> {
    require!(
        matches!(ctx.accounts.proposal.action, ProposalAction::Signal),
        GovernanceError::ActionAccountMismatch
    );

    let now = Clock::get()?.unix_timestamp;
    authorize_execution(&mut ctx.accounts.proposal, now)?;

    emit!(ProposalExecuted {
        proposal: ctx.accounts.proposal.key(),
        action: ctx.accounts.proposal.action,
        timestamp: now,
    });

    Ok(())
}

// ------------------------------------------------------- TreasuryTransfer

#[derive(Accounts)]
pub struct ExecuteTreasuryTransfer<'info> {
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

    /// CHECK: the PDA that signs for the treasury. Identity is fixed by the seeds
    /// constraint; it is never deserialised, only used as a signer. The treasury
    /// program accepts nothing else, so producing this signature here is exactly
    /// what "governance approved the spend" means.
    #[account(
        seeds = [EXECUTOR_SEED, realm.key().as_ref()],
        bump = realm.executor_bump,
    )]
    pub executor: UncheckedAccount<'info>,

    #[account(mut)]
    pub treasury: Box<Account<'info, helix_treasury::state::Treasury>>,

    pub treasury_mint: Box<InterfaceAccount<'info, Mint>>,

    #[account(mut)]
    pub treasury_vault: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(mut)]
    pub destination: Box<InterfaceAccount<'info, TokenAccount>>,

    /// CHECK: the treasury's own vault authority PDA, validated by the treasury
    /// program's own seeds constraint when the CPI lands.
    pub treasury_vault_authority: UncheckedAccount<'info>,

    pub token_program: Interface<'info, TokenInterface>,
    pub treasury_program: Program<'info, helix_treasury::program::HelixTreasury>,
}

/// Performs the treasury transfer a passed proposal called for.
pub fn execute_treasury_transfer(ctx: Context<ExecuteTreasuryTransfer>) -> Result<()> {
    // Destructure the action rather than trusting instruction arguments: the
    // amount and destination are whatever the *voters approved*, not whatever the
    // executing caller passes in.
    let (destination, amount) = match ctx.accounts.proposal.action {
        ProposalAction::TreasuryTransfer {
            destination,
            amount,
        } => (destination, amount),
        _ => return Err(GovernanceError::ActionAccountMismatch.into()),
    };

    // The account supplied must be the one the proposal named.
    require_keys_eq!(
        ctx.accounts.destination.key(),
        destination,
        GovernanceError::ActionAccountMismatch
    );

    let now = Clock::get()?.unix_timestamp;
    authorize_execution(&mut ctx.accounts.proposal, now)?;

    let realm_key = ctx.accounts.realm.key();
    let signer_seeds: &[&[&[u8]]] = &[&[
        EXECUTOR_SEED,
        realm_key.as_ref(),
        &[ctx.accounts.realm.executor_bump],
    ]];

    helix_treasury::cpi::spend(
        CpiContext::new_with_signer(
            ctx.accounts.treasury_program.key(),
            helix_treasury::cpi::accounts::Spend {
                treasury: ctx.accounts.treasury.to_account_info(),
                governance_executor: ctx.accounts.executor.to_account_info(),
                mint: ctx.accounts.treasury_mint.to_account_info(),
                vault: ctx.accounts.treasury_vault.to_account_info(),
                destination: ctx.accounts.destination.to_account_info(),
                vault_authority: ctx.accounts.treasury_vault_authority.to_account_info(),
                token_program: ctx.accounts.token_program.to_account_info(),
            },
            signer_seeds,
        ),
        amount,
    )?;

    emit!(ProposalExecuted {
        proposal: ctx.accounts.proposal.key(),
        action: ctx.accounts.proposal.action,
        timestamp: now,
    });

    Ok(())
}

// --------------------------------------------------- SetStakingRewardRate

#[derive(Accounts)]
pub struct ExecuteSetStakingRewardRate<'info> {
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

    /// CHECK: signs for the pool. Must be the pool's configured authority, which
    /// the staking program checks when the CPI lands.
    #[account(
        seeds = [EXECUTOR_SEED, realm.key().as_ref()],
        bump = realm.executor_bump,
    )]
    pub executor: UncheckedAccount<'info>,

    #[account(mut)]
    pub staking_pool: Box<Account<'info, helix_staking::state::Pool>>,

    pub reward_vault: Box<InterfaceAccount<'info, TokenAccount>>,

    pub staking_program: Program<'info, helix_staking::program::HelixStaking>,
}

/// Retunes staking emissions as a passed proposal directed.
///
/// Requires the pool's authority to already be this realm's executor PDA — the
/// deployment step that makes emissions genuinely DAO-controlled rather than
/// operator-controlled. The staking program refuses the CPI otherwise, and its
/// own solvency check still applies: governance cannot vote for a rate the reward
/// vault cannot fund.
pub fn execute_set_staking_reward_rate(ctx: Context<ExecuteSetStakingRewardRate>) -> Result<()> {
    let (new_rate, reward_period_end) = match ctx.accounts.proposal.action {
        ProposalAction::SetStakingRewardRate {
            new_rate,
            reward_period_end,
        } => (new_rate, reward_period_end),
        _ => return Err(GovernanceError::ActionAccountMismatch.into()),
    };

    let now = Clock::get()?.unix_timestamp;
    authorize_execution(&mut ctx.accounts.proposal, now)?;

    let realm_key = ctx.accounts.realm.key();
    let signer_seeds: &[&[&[u8]]] = &[&[
        EXECUTOR_SEED,
        realm_key.as_ref(),
        &[ctx.accounts.realm.executor_bump],
    ]];

    helix_staking::cpi::set_reward_rate(
        CpiContext::new_with_signer(
            ctx.accounts.staking_program.key(),
            helix_staking::cpi::accounts::SetRewardRate {
                pool: ctx.accounts.staking_pool.to_account_info(),
                authority: ctx.accounts.executor.to_account_info(),
                reward_vault: ctx.accounts.reward_vault.to_account_info(),
            },
            signer_seeds,
        ),
        new_rate,
        reward_period_end,
    )?;

    emit!(ProposalExecuted {
        proposal: ctx.accounts.proposal.key(),
        action: ctx.accounts.proposal.action,
        timestamp: now,
    });

    Ok(())
}
