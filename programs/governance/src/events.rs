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
    /// Carried for the same reason every other parameter is: a consumer that
    /// reconstructs the realm from events must arrive at the account, and this
    /// field is otherwise unlearnable until the first `RealmParamsUpdated`. An
    /// event that omits one field of the state it announces makes the whole
    /// reconstruction conditional on an update that may never happen.
    pub min_weight_to_propose: u64,
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
    /// How many positions that denominator covers. Carried so a consumer can
    /// tell whether a later vote belonged to the electorate without reading the
    /// proposal account back.
    pub position_count_snapshot: u64,
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

#[event]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RealmParamsUpdated {
    pub realm: Pubkey,
    /// Whether the change came through a proposal or from the realm authority
    /// signing directly. Worth recording: before the authority is migrated these
    /// are two very different events with the same effect.
    pub by_proposal: bool,
    pub quorum_bps: u16,
    pub approval_bps: u16,
    pub voting_period: i64,
    pub timelock_delay: i64,
    pub min_weight_to_propose: u64,
    pub timestamp: i64,
}

#[event]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RealmAuthorityChanged {
    pub realm: Pubkey,
    pub previous_authority: Pubkey,
    pub new_authority: Pubkey,
    /// True once the realm's parameters answer only to the realm itself.
    pub self_governing: bool,
    pub timestamp: i64,
}
