use anchor_lang::prelude::*;

#[error_code]
pub enum GovernanceError {
    #[msg("Caller is not the realm authority")]
    NotAuthority,

    #[msg("Caller is not the realm guardian")]
    NotGuardian,

    #[msg("Voting period is outside the permitted range")]
    InvalidVotingPeriod,

    #[msg("Timelock delay is outside the permitted range")]
    InvalidTimelockDelay,

    #[msg("Quorum must be between 1 and 10000 basis points")]
    InvalidQuorum,

    #[msg("Approval threshold must be a simple majority or greater")]
    InvalidApprovalThreshold,

    #[msg("Title or URI exceeds its maximum length")]
    TextTooLong,

    #[msg("Proposal is not in the required state for this action")]
    InvalidProposalState,

    #[msg("Voting has not started")]
    VotingNotStarted,

    #[msg("Voting has ended")]
    VotingEnded,

    #[msg("Voting is still open")]
    VotingStillOpen,

    #[msg("Position does not belong to the voter")]
    NotPositionOwner,

    #[msg("Position belongs to a different staking pool than this realm")]
    PoolMismatch,

    #[msg("Position lock expires before voting closes, so it carries no weight")]
    InsufficientLockDuration,

    #[msg("Position carries zero weight")]
    ZeroWeight,

    #[msg("Position was opened after the proposal's weight snapshot was taken")]
    PositionNotInSnapshot,

    #[msg("Proposer does not meet the minimum weight to create a proposal")]
    BelowProposalThreshold,

    #[msg("Timelock has not elapsed")]
    TimelockNotElapsed,

    #[msg("Proposal has expired and can no longer be executed")]
    ProposalExpired,

    #[msg("Proposal has no snapshot of total voting weight")]
    MissingSnapshot,

    #[msg("Accounts supplied do not match the proposal's action")]
    ActionAccountMismatch,

    #[msg("Arithmetic overflow")]
    MathOverflow,
}
