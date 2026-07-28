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
}
