//! Events are this program's public data interface: consumers reconstruct state
//! by decoding these from transaction logs rather than by reading accounts. They
//! derive `Clone`/`Debug`/`PartialEq` so a consumer can hold, compare and test
//! against them — `#[event]` alone provides only the Borsh pair. See
//! [`indexer/`](../../../../indexer).

use anchor_lang::prelude::*;

use crate::state::{ProposalAction, ProposalState, VoteChoice};

#[event]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RealmInitialized {
    pub realm: Pubkey,
    pub authority: Pubkey,
    pub guardian: Pubkey,
    pub staking_pool: Pubkey,
    pub quorum_bps: u16,
    pub approval_bps: u16,
    pub voting_period: i64,
    pub timelock_delay: i64,
    pub timestamp: i64,
}

#[event]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProposalCreated {
    pub realm: Pubkey,
    pub proposal: Pubkey,
    pub proposer: Pubkey,
    pub id: u64,
    pub action: ProposalAction,
    pub title: String,
    pub timestamp: i64,
}

#[event]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProposalActivated {
    pub proposal: Pubkey,
    pub voting_starts_at: i64,
    pub voting_ends_at: i64,
    /// The quorum denominator, fixed at this moment.
    pub total_weight_snapshot: u64,
    pub timestamp: i64,
}

#[event]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VoteCast {
    pub proposal: Pubkey,
    pub position: Pubkey,
    pub voter: Pubkey,
    pub choice: VoteChoice,
    pub weight: u64,
    pub for_votes: u64,
    pub against_votes: u64,
    pub abstain_votes: u64,
    pub timestamp: i64,
}

#[event]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProposalFinalized {
    pub proposal: Pubkey,
    pub outcome: ProposalState,
    pub for_votes: u64,
    pub against_votes: u64,
    pub abstain_votes: u64,
    pub total_weight_snapshot: u64,
    pub timestamp: i64,
}

#[event]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProposalQueued {
    pub proposal: Pubkey,
    /// Earliest execution time.
    pub eta: i64,
    /// After this, the proposal expires unexecuted.
    pub expires_at: i64,
    pub timestamp: i64,
}

#[event]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProposalExecuted {
    pub proposal: Pubkey,
    pub action: ProposalAction,
    pub timestamp: i64,
}

#[event]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProposalCancelled {
    pub proposal: Pubkey,
    pub guardian: Pubkey,
    /// State the proposal was vetoed from, so the record shows how far it got.
    pub previous_state: ProposalState,
    pub timestamp: i64,
}
