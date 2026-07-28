//! Executing token-manager actions.
//!
//! Split from `execute.rs` because it is a distinct authority chain: these do not
//! touch treasury funds, they govern issuance policy.
//!
//! The token-manager admin is the one authority that cannot be wired to
//! governance at bootstrap — `register_minter` must run before the staking
//! program can pay rewards, and only an admin can register a minter. So it starts
//! as a multisig and is handed over afterwards, which is why `accept_admin` needs
//! to be reachable by proposal at all
//! ([F-9](../../../../docs/SECURITY-ASSESSMENT.md#f-9--token-manager-admin-cannot-be-handed-to-governance)).
//!
//! The handover alone is not enough. A realm that became admin without also being
//! able to register minters, retune caps or pause issuance would hold the role and
//! none of its powers — the same defect as F-8 in a new place. The whole admin
//! surface is therefore covered here.

use anchor_lang::prelude::*;

use crate::constants::{EXECUTOR_SEED, PROPOSAL_SEED, REALM_SEED};
use crate::errors::GovernanceError;
use crate::events::ProposalExecuted;
use crate::state::{Proposal, ProposalAction, Realm};

use super::execute::authorize_execution;

/// Accounts common to every token-manager action that touches only the config.
#[derive(Accounts)]
pub struct ExecuteTokenAdmin<'info> {
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

    /// CHECK: signs as the token-manager admin; identity fixed by seeds.
    #[account(
        seeds = [EXECUTOR_SEED, realm.key().as_ref()],
        bump = realm.executor_bump,
    )]
    pub executor: UncheckedAccount<'info>,

    #[account(mut)]
    pub token_config: Box<Account<'info, helix_token_manager::state::TokenConfig>>,

    pub token_manager_program: Program<'info, helix_token_manager::program::HelixTokenManager>,
}

impl<'info> ExecuteTokenAdmin<'info> {
    fn admin_only(&self) -> helix_token_manager::cpi::accounts::AdminOnly<'info> {
        helix_token_manager::cpi::accounts::AdminOnly {
            config: self.token_config.to_account_info(),
            admin: self.executor.to_account_info(),
        }
    }
}

/// Completes the admin handover, making this realm the mint's admin.
///
/// The token-manager's `accept_admin` requires the incoming admin to sign, and
/// this PDA can only sign inside an execution — which is exactly why the handover
/// was impossible before this instruction existed.
pub fn execute_accept_token_manager_admin(ctx: Context<ExecuteTokenAdmin>) -> Result<()> {
    require!(
        matches!(
            ctx.accounts.proposal.action,
            ProposalAction::AcceptTokenManagerAdmin
        ),
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

    helix_token_manager::cpi::accept_admin(CpiContext::new_with_signer(
        ctx.accounts.token_manager_program.key(),
        helix_token_manager::cpi::accounts::AcceptAdmin {
            config: ctx.accounts.token_config.to_account_info(),
            new_admin: ctx.accounts.executor.to_account_info(),
        },
        signer_seeds,
    ))?;

    emit!(ProposalExecuted {
        proposal: ctx.accounts.proposal.key(),
        action: ctx.accounts.proposal.action,
        timestamp: now,
    });

    Ok(())
}

/// Halts or resumes issuance.
pub fn execute_set_token_paused(ctx: Context<ExecuteTokenAdmin>) -> Result<()> {
    let paused = match ctx.accounts.proposal.action {
        ProposalAction::SetTokenPaused { paused } => paused,
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

    helix_token_manager::cpi::set_paused(
        CpiContext::new_with_signer(
            ctx.accounts.token_manager_program.key(),
            ctx.accounts.admin_only(),
            signer_seeds,
        ),
        paused,
    )?;

    emit!(ProposalExecuted {
        proposal: ctx.accounts.proposal.key(),
        action: ctx.accounts.proposal.action,
        timestamp: now,
    });

    Ok(())
}

/// Begins handing the admin role onward. The successor must still accept.
pub fn execute_propose_token_admin(ctx: Context<ExecuteTokenAdmin>) -> Result<()> {
    let new_admin = match ctx.accounts.proposal.action {
        ProposalAction::ProposeTokenAdmin { new_admin } => new_admin,
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

    helix_token_manager::cpi::propose_admin(
        CpiContext::new_with_signer(
            ctx.accounts.token_manager_program.key(),
            ctx.accounts.admin_only(),
            signer_seeds,
        ),
        new_admin,
    )?;

    emit!(ProposalExecuted {
        proposal: ctx.accounts.proposal.key(),
        action: ctx.accounts.proposal.action,
        timestamp: now,
    });

    Ok(())
}

// ------------------------------------------------------------ minter registry

#[derive(Accounts)]
pub struct ExecuteRegisterMinter<'info> {
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

    /// CHECK: signs as admin; identity fixed by seeds.
    #[account(
        seeds = [EXECUTOR_SEED, realm.key().as_ref()],
        bump = realm.executor_bump,
    )]
    pub executor: UncheckedAccount<'info>,

    /// Pays rent for the new minter account.
    #[account(mut)]
    pub payer: Signer<'info>,

    #[account(mut)]
    pub token_config: Box<Account<'info, helix_token_manager::state::TokenConfig>>,

    /// CHECK: created by the token-manager, which derives and validates this PDA
    /// under its own seeds constraint.
    #[account(mut)]
    pub minter: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
    pub token_manager_program: Program<'info, helix_token_manager::program::HelixTokenManager>,
}

/// Registers a minter with a per-epoch issuance cap.
pub fn execute_register_minter(ctx: Context<ExecuteRegisterMinter>) -> Result<()> {
    let (authority, epoch_cap, epoch_duration) = match ctx.accounts.proposal.action {
        ProposalAction::RegisterMinter {
            authority,
            epoch_cap,
            epoch_duration,
        } => (authority, epoch_cap, epoch_duration),
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

    helix_token_manager::cpi::register_minter(
        CpiContext::new_with_signer(
            ctx.accounts.token_manager_program.key(),
            helix_token_manager::cpi::accounts::RegisterMinter {
                config: ctx.accounts.token_config.to_account_info(),
                admin: ctx.accounts.executor.to_account_info(),
                payer: ctx.accounts.payer.to_account_info(),
                minter: ctx.accounts.minter.to_account_info(),
                system_program: ctx.accounts.system_program.to_account_info(),
            },
            signer_seeds,
        ),
        authority,
        epoch_cap,
        epoch_duration,
    )?;

    emit!(ProposalExecuted {
        proposal: ctx.accounts.proposal.key(),
        action: ctx.accounts.proposal.action,
        timestamp: now,
    });

    Ok(())
}

#[derive(Accounts)]
pub struct ExecuteModifyMinter<'info> {
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

    /// CHECK: signs as admin; identity fixed by seeds.
    #[account(
        seeds = [EXECUTOR_SEED, realm.key().as_ref()],
        bump = realm.executor_bump,
    )]
    pub executor: UncheckedAccount<'info>,

    #[account(mut)]
    pub token_config: Box<Account<'info, helix_token_manager::state::TokenConfig>>,

    #[account(mut)]
    pub minter: Box<Account<'info, helix_token_manager::state::Minter>>,

    pub token_manager_program: Program<'info, helix_token_manager::program::HelixTokenManager>,
}

/// Adjusts a minter's cap, or enables/disables it.
pub fn execute_update_minter(ctx: Context<ExecuteModifyMinter>) -> Result<()> {
    let (epoch_cap, enabled) = match ctx.accounts.proposal.action {
        ProposalAction::UpdateMinter { epoch_cap, enabled } => (epoch_cap, enabled),
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

    helix_token_manager::cpi::update_minter(
        CpiContext::new_with_signer(
            ctx.accounts.token_manager_program.key(),
            helix_token_manager::cpi::accounts::ModifyMinter {
                config: ctx.accounts.token_config.to_account_info(),
                admin: ctx.accounts.executor.to_account_info(),
                minter: ctx.accounts.minter.to_account_info(),
            },
            signer_seeds,
        ),
        epoch_cap,
        enabled,
    )?;

    emit!(ProposalExecuted {
        proposal: ctx.accounts.proposal.key(),
        action: ctx.accounts.proposal.action,
        timestamp: now,
    });

    Ok(())
}

/// Permanently disables a minter.
pub fn execute_revoke_minter(ctx: Context<ExecuteModifyMinter>) -> Result<()> {
    require!(
        matches!(ctx.accounts.proposal.action, ProposalAction::RevokeMinter),
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

    helix_token_manager::cpi::revoke_minter(CpiContext::new_with_signer(
        ctx.accounts.token_manager_program.key(),
        helix_token_manager::cpi::accounts::RevokeMinter {
            config: ctx.accounts.token_config.to_account_info(),
            admin: ctx.accounts.executor.to_account_info(),
            minter: ctx.accounts.minter.to_account_info(),
        },
        signer_seeds,
    ))?;

    emit!(ProposalExecuted {
        proposal: ctx.accounts.proposal.key(),
        action: ctx.accounts.proposal.action,
        timestamp: now,
    });

    Ok(())
}
