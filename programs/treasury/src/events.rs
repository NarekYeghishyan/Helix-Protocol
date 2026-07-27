use anchor_lang::prelude::*;

#[event]
pub struct TreasuryInitialized {
    pub treasury: Pubkey,
    pub governance_executor: Pubkey,
    pub mint: Pubkey,
    pub epoch_spend_cap: u64,
    pub epoch_duration: i64,
    pub timestamp: i64,
}

#[event]
pub struct Deposited {
    pub treasury: Pubkey,
    pub depositor: Pubkey,
    pub amount_credited: u64,
    pub total_deposited: u64,
    pub timestamp: i64,
}

#[event]
pub struct Spent {
    pub treasury: Pubkey,
    pub destination: Pubkey,
    pub amount: u64,
    pub remaining_epoch_budget: u64,
    pub total_spent: u64,
    pub timestamp: i64,
}

#[event]
pub struct StreamCreated {
    pub treasury: Pubkey,
    pub stream: Pubkey,
    pub beneficiary: Pubkey,
    pub stream_id: u64,
    pub total_amount: u64,
    pub start_ts: i64,
    pub cliff_ts: i64,
    pub end_ts: i64,
    pub timestamp: i64,
}

#[event]
pub struct StreamClaimed {
    pub treasury: Pubkey,
    pub stream: Pubkey,
    pub beneficiary: Pubkey,
    pub amount: u64,
    pub total_claimed: u64,
    pub timestamp: i64,
}

#[event]
pub struct StreamRevoked {
    pub treasury: Pubkey,
    pub stream: Pubkey,
    pub beneficiary: Pubkey,
    /// Already vested and still claimable by the beneficiary.
    pub vested_retained: u64,
    /// Returned to the treasury's uncommitted balance.
    pub unvested_returned: u64,
    pub timestamp: i64,
}

#[event]
pub struct SpendCapChanged {
    pub treasury: Pubkey,
    pub old_cap: u64,
    pub new_cap: u64,
    pub epoch_duration: i64,
    pub timestamp: i64,
}

#[event]
pub struct GovernanceExecutorChanged {
    pub treasury: Pubkey,
    pub previous_executor: Pubkey,
    pub new_executor: Pubkey,
    pub timestamp: i64,
}
