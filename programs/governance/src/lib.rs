#![allow(unexpected_cfgs)]
#![doc = include_str!("../README.md")]

use anchor_lang::prelude::*;

pub mod constants;
pub mod errors;
pub mod events;
pub mod instructions;
pub mod state;

use instructions::*;
use state::{ProposalAction, VoteChoice};

declare_id!("nSZnzJR8uUuZu8t1SqmLU2ExCvXNYABuVHwrDQJqSf5");

#[program]
pub mod helix_governance {
    use super::*;

    /// Creates a realm governing one staking pool.
    pub fn initialize_realm(ctx: Context<InitializeRealm>, params: RealmParams) -> Result<()> {
        instructions::realm::initialize_realm(ctx, params)
    }

    /// Changes governance parameters. Does not affect already-queued proposals.
    pub fn update_realm_params(ctx: Context<UpdateRealmParams>, params: RealmParams) -> Result<()> {
        instructions::realm::update_realm_params(ctx, params)
    }

    // --------------------------------------------------------------- proposal

    /// Creates a proposal in `Draft`. Requires `min_weight_to_propose`.
    pub fn create_proposal(
        ctx: Context<CreateProposal>,
        proposal_id: u64,
        action: ProposalAction,
        title: String,
        descriptor_uri: String,
    ) -> Result<()> {
        instructions::proposal::create_proposal(ctx, proposal_id, action, title, descriptor_uri)
    }

    /// Opens voting and fixes the quorum denominator. Permissionless.
    pub fn activate_proposal(ctx: Context<ActivateProposal>) -> Result<()> {
        instructions::proposal::activate_proposal(ctx)
    }

    /// Casts a position's weight. Requires `lock_end >= voting_ends_at`.
    pub fn cast_vote(ctx: Context<CastVote>, choice: VoteChoice) -> Result<()> {
        instructions::vote::cast_vote(ctx, choice)
    }

    /// Resolves a closed vote. Permissionless.
    pub fn finalize_proposal(ctx: Context<AdvanceProposal>) -> Result<()> {
        instructions::lifecycle::finalize_proposal(ctx)
    }

    /// Moves a passed proposal into the timelock.
    pub fn queue_proposal(ctx: Context<AdvanceProposal>) -> Result<()> {
        instructions::lifecycle::queue_proposal(ctx)
    }

    /// Guardian veto. The guardian's only power.
    pub fn cancel_proposal(ctx: Context<CancelProposal>) -> Result<()> {
        instructions::proposal::cancel_proposal(ctx)
    }

    // ---------------------------------------------------------------- execute

    /// Executes a signalling proposal.
    pub fn execute_signal(ctx: Context<ExecuteSignal>) -> Result<()> {
        instructions::execute::execute_signal(ctx)
    }

    /// Executes a treasury transfer, CPI-signing as the realm executor.
    pub fn execute_treasury_transfer(ctx: Context<ExecuteTreasuryTransfer>) -> Result<()> {
        instructions::execute::execute_treasury_transfer(ctx)
    }

    /// Executes a staking emission change.
    pub fn execute_set_staking_reward_rate(
        ctx: Context<ExecuteSetStakingRewardRate>,
    ) -> Result<()> {
        instructions::execute::execute_set_staking_reward_rate(ctx)
    }

    /// Creates the vesting stream a passed proposal called for. `stream_id` must
    /// equal the treasury's current `stream_count`, which the treasury verifies.
    pub fn execute_create_vesting_stream(
        ctx: Context<ExecuteCreateVestingStream>,
        stream_id: u64,
    ) -> Result<()> {
        instructions::execute::execute_create_vesting_stream(ctx, stream_id)
    }

    /// Revokes a vesting stream. Already-vested tokens stay claimable.
    pub fn execute_revoke_vesting_stream(ctx: Context<ExecuteRevokeVestingStream>) -> Result<()> {
        instructions::execute::execute_revoke_vesting_stream(ctx)
    }

    /// Adjusts the treasury's per-epoch spend cap.
    pub fn execute_set_treasury_spend_cap(ctx: Context<ExecuteTreasuryConfig>) -> Result<()> {
        instructions::execute::execute_set_treasury_spend_cap(ctx)
    }

    /// Hands treasury spending rights to a different governance executor.
    pub fn execute_set_governance_executor(ctx: Context<ExecuteTreasuryConfig>) -> Result<()> {
        instructions::execute::execute_set_governance_executor(ctx)
    }

    // ---------------------------------------------------------- token-manager

    /// Completes the admin handover, making this realm the mint's admin.
    pub fn execute_accept_token_manager_admin(ctx: Context<ExecuteTokenAdmin>) -> Result<()> {
        instructions::execute_token::execute_accept_token_manager_admin(ctx)
    }

    /// Halts or resumes HLX issuance. Never blocks burning.
    pub fn execute_set_token_paused(ctx: Context<ExecuteTokenAdmin>) -> Result<()> {
        instructions::execute_token::execute_set_token_paused(ctx)
    }

    /// Begins handing the admin role onward; the successor must still accept.
    pub fn execute_propose_token_admin(ctx: Context<ExecuteTokenAdmin>) -> Result<()> {
        instructions::execute_token::execute_propose_token_admin(ctx)
    }

    /// Registers a minter with a per-epoch issuance cap.
    pub fn execute_register_minter(ctx: Context<ExecuteRegisterMinter>) -> Result<()> {
        instructions::execute_token::execute_register_minter(ctx)
    }

    /// Adjusts a minter's cap, or enables/disables it.
    pub fn execute_update_minter(ctx: Context<ExecuteModifyMinter>) -> Result<()> {
        instructions::execute_token::execute_update_minter(ctx)
    }

    /// Permanently disables a minter, retaining its issuance history.
    pub fn execute_revoke_minter(ctx: Context<ExecuteModifyMinter>) -> Result<()> {
        instructions::execute_token::execute_revoke_minter(ctx)
    }

    /// Retunes the realm's own parameters — quorum, approval, periods,
    /// proposal threshold — through a passed, timelocked proposal.
    pub fn execute_update_realm_params(ctx: Context<ExecuteRealmConfig>) -> Result<()> {
        instructions::execute_realm::execute_update_realm_params(ctx)
    }

    /// Moves `realm.authority`, in practice to the realm's own executor PDA.
    /// This is the migration that makes the realm answer only to itself.
    pub fn execute_set_realm_authority(ctx: Context<ExecuteRealmConfig>) -> Result<()> {
        instructions::execute_realm::execute_set_realm_authority(ctx)
    }
}
