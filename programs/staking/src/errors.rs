use anchor_lang::prelude::*;

#[error_code]
pub enum StakingError {
    #[msg("Caller is not the pool authority")]
    NotAuthority,

    #[msg("No authority transfer is pending")]
    NoPendingAuthority,

    #[msg("Caller is not the pending authority")]
    NotPendingAuthority,

    #[msg("New deposits are paused")]
    DepositsPaused,

    #[msg("Stake amount is below the minimum")]
    BelowMinimumStake,

    #[msg("Amount must be greater than zero")]
    ZeroAmount,

    #[msg("Position is still locked")]
    PositionLocked,

    #[msg("Requested amount exceeds the position balance")]
    InsufficientStake,

    #[msg("Reward rate exceeds the permitted maximum")]
    RewardRateTooHigh,

    #[msg("Reward period end must be in the future")]
    InvalidRewardPeriod,

    #[msg("Reward vault cannot fund this rate for the full period")]
    InsufficientRewardFunding,

    #[msg("Nothing to claim")]
    NothingToClaim,

    #[msg("Deposit credited zero after transfer fees")]
    ZeroAfterFees,

    #[msg("Vault balance moved unexpectedly during the transfer")]
    VaultBalanceMismatch,

    #[msg("Position does not belong to this pool")]
    PositionPoolMismatch,

    #[msg("Arithmetic overflow")]
    MathOverflow,

    // New variants are appended rather than inserted. Anchor numbers these in
    // declaration order from 6000, so inserting one silently renumbers every
    // error after it — and any client, log or runbook that recorded the old
    // number starts naming a different failure.
    #[msg("Position still holds principal or vote weight")]
    PositionNotEmpty,

    #[msg("Position has unclaimed rewards")]
    UnclaimedRewards,

    #[msg("position_id must equal the pool's current position_count")]
    UnexpectedPositionId,
}
