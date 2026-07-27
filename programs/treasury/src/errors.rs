use anchor_lang::prelude::*;

#[error_code]
pub enum TreasuryError {
    #[msg("Caller is not the governance executor for this treasury")]
    NotGovernanceExecutor,

    #[msg("Caller is not the stream beneficiary")]
    NotBeneficiary,

    #[msg("Amount must be greater than zero")]
    ZeroAmount,

    #[msg("Spend would exceed the treasury's cap for the current epoch")]
    EpochSpendCapExceeded,

    #[msg("Epoch duration is outside the permitted range")]
    InvalidEpochDuration,

    #[msg("Vesting schedule is invalid: require start <= cliff <= end")]
    InvalidVestingSchedule,

    #[msg("Vesting stream is shorter than the minimum duration")]
    StreamTooShort,

    #[msg("Stream has already been revoked")]
    StreamRevoked,

    #[msg("Nothing is claimable yet")]
    NothingClaimable,

    #[msg("Vault holds less than this treasury's committed obligations")]
    InsufficientUncommittedBalance,

    #[msg("Deposit credited zero after transfer fees")]
    ZeroAfterFees,

    #[msg("Vault balance moved unexpectedly during the transfer")]
    VaultBalanceMismatch,

    #[msg("Arithmetic overflow")]
    MathOverflow,
}
