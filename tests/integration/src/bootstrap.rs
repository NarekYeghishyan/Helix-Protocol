//! A fully wired system: mint, pool, realm, treasury, authorities connected.
//!
//! The wiring is the point. Unit tests reach every pure function in the
//! workspace, so a fixture that stops short of connecting the programs would only
//! re-test what already passes. This builds the whole chain — stake confers vote
//! weight, a passed proposal produces an executor signature, the treasury accepts
//! that signature and nothing else.

use anchor_lang::prelude::Pubkey;
use anchor_spl::token_2022;
use helix_governance::instructions::realm::RealmParams;
use helix_governance::state::{ProposalAction, VoteChoice};
use helix_staking::state::LockTier;
use solana_keypair::Keypair;
use solana_native_token::LAMPORTS_PER_SOL;
use solana_signer::Signer as _;

use crate::{pda, TestEnv, TransferFee};

pub const DECIMALS: u8 = 9;
pub const HOUR: i64 = 3_600;

/// Realm settings used by the tests. Voting period and timelock are at their
/// permitted minimums so tests warp seconds rather than days.
pub fn default_realm_params() -> RealmParams {
    RealmParams {
        quorum_bps: 2_000,   // 20%
        approval_bps: 5_001, // simple majority
        voting_period: HOUR,
        timelock_delay: HOUR,
        min_weight_to_propose: 1,
    }
}

pub struct System {
    pub env: TestEnv,

    pub mint: Pubkey,
    pub mint_authority: Keypair,

    pub pool: Pubkey,
    pub stake_vault: Pubkey,
    pub reward_vault: Pubkey,

    pub realm: Pubkey,
    pub executor: Pubkey,
    pub guardian: Keypair,

    pub treasury: Pubkey,
    pub treasury_vault: Pubkey,
    pub treasury_vault_authority: Pubkey,

    /// Holds stake and votes.
    pub voter: Keypair,
    pub voter_tokens: Pubkey,
}

/// A staker independent of [`System::voter`] — own key, own token account, own
/// position.
///
/// Most tests only need one actor, but anything asserting a property *across*
/// stakers (reward distribution, vote tallies, compute cost as the pool fills)
/// needs the pool populated by distinct owners rather than one owner holding
/// several positions.
pub struct Staker {
    pub owner: Keypair,
    pub tokens: Pubkey,
    pub position: Pubkey,
    pub position_id: u64,
}

impl System {
    /// Builds everything, with the treasury's sole spender set to the realm's
    /// executor PDA.
    pub fn bootstrap(fee: Option<TransferFee>, treasury_funding: u64) -> Self {
        Self::bootstrap_with_mint(Keypair::new(), fee, treasury_funding)
    }

    /// The same, with the mint key supplied rather than generated.
    ///
    /// Every PDA in the system descends from the mint, and so does every PDA
    /// *bump*. Anchor derives some of those bumps on chain, at a syscall per
    /// attempt, which makes a random mint a source of run-to-run compute variance
    /// in 1500-unit steps. Anything comparing measurements across two `System`s
    /// has to hold the mint fixed or it is comparing deployments, not code — see
    /// `compute_budget.rs`.
    pub fn bootstrap_with_mint(
        mint_keypair: Keypair,
        fee: Option<TransferFee>,
        treasury_funding: u64,
    ) -> Self {
        let mut env = TestEnv::new();
        let payer = env.payer_pubkey();

        let mint_authority = Keypair::new();
        let mint = env
            .create_mint_with_keypair(mint_keypair, DECIMALS, &mint_authority.pubkey(), fee)
            .pubkey();

        // ---------------------------------------------------------- staking
        let (pool, _) = pda::pool(&mint, &mint);
        let (vault_authority, _) = pda::vault_authority(&pool);
        let (stake_vault, _) = pda::stake_vault(&pool);
        let (reward_vault, _) = pda::reward_vault(&pool);

        env.send(
            &[TestEnv::ix(
                helix_staking::ID,
                helix_staking::accounts::InitializePool {
                    payer,
                    authority: payer,
                    pool,
                    vault_authority,
                    stake_mint: mint,
                    reward_mint: mint,
                    stake_vault,
                    reward_vault,
                    token_program: token_2022::ID,
                    system_program: anchor_lang::system_program::ID,
                },
                helix_staking::instruction::InitializePool {},
            )],
            &[],
        );

        // --------------------------------------------------------- governance
        let (realm, _) = pda::realm(&pool);
        let (executor, _) = pda::executor(&realm);
        let guardian = Keypair::new();

        env.send(
            &[TestEnv::ix(
                helix_governance::ID,
                helix_governance::accounts::InitializeRealm {
                    payer,
                    authority: payer,
                    guardian: guardian.pubkey(),
                    staking_pool: pool,
                    realm,
                    executor,
                    system_program: anchor_lang::system_program::ID,
                },
                helix_governance::instruction::InitializeRealm {
                    params: default_realm_params(),
                },
            )],
            &[],
        );

        // ----------------------------------------------------------- treasury
        // The executor PDA is named as the only spender. This is the wiring the
        // whole security model depends on.
        let (treasury, _) = pda::treasury(&mint);
        let (treasury_vault, _) = pda::treasury_vault(&treasury);
        let (treasury_vault_authority, _) = pda::treasury_vault_authority(&treasury);

        env.send(
            &[TestEnv::ix(
                helix_treasury::ID,
                helix_treasury::accounts::InitializeTreasury {
                    payer,
                    governance_executor: executor,
                    treasury,
                    vault_authority: treasury_vault_authority,
                    mint,
                    vault: treasury_vault,
                    token_program: token_2022::ID,
                    system_program: anchor_lang::system_program::ID,
                },
                helix_treasury::instruction::InitializeTreasury {
                    epoch_spend_cap: u64::MAX / 2,
                    epoch_duration: 24 * HOUR,
                },
            )],
            &[],
        );

        // ------------------------------------------------------------- actors
        let voter = Keypair::new();
        env.svm
            .airdrop(&voter.pubkey(), 100 * LAMPORTS_PER_SOL)
            .unwrap();
        let voter_tokens = env.create_token_account(&mint, &voter.pubkey()).pubkey();
        env.mint_tokens_raw(&mint, &voter_tokens, &mint_authority, 10_000_000);

        let mut system = Self {
            env,
            mint,
            mint_authority,
            pool,
            stake_vault,
            reward_vault,
            realm,
            executor,
            guardian,
            treasury,
            treasury_vault,
            treasury_vault_authority,
            voter,
            voter_tokens,
        };

        if treasury_funding > 0 {
            system.fund_treasury(treasury_funding);
        }

        system
    }

    /// Mints to the payer and deposits into the treasury vault.
    pub fn fund_treasury(&mut self, amount: u64) {
        let ix = self.fund_treasury_ix(amount);
        self.env.send(&[ix], &[]);
    }

    /// Prepares the funder account and returns the deposit unsent, so a caller can
    /// measure it in isolation from the mint and token-account setup around it.
    pub fn fund_treasury_ix(&mut self, amount: u64) -> solana_instruction::Instruction {
        let payer = self.env.payer_pubkey();
        let funder_tokens = self.env.create_token_account(&self.mint, &payer).pubkey();
        let authority = self.mint_authority.insecure_clone();
        self.env
            .mint_tokens_raw(&self.mint, &funder_tokens, &authority, amount);

        TestEnv::ix(
            helix_treasury::ID,
            helix_treasury::accounts::Deposit {
                treasury: self.treasury,
                depositor: payer,
                mint: self.mint,
                depositor_token_account: funder_tokens,
                vault: self.treasury_vault,
                token_program: token_2022::ID,
            },
            helix_treasury::instruction::Deposit { amount },
        )
    }

    /// Opens a staking position for the voter.
    pub fn stake(&mut self, position_id: u64, amount: u64, tier: LockTier) -> Pubkey {
        let (position, _) = pda::position(&self.pool, &self.voter.pubkey(), position_id);
        let ix = self.stake_ix(position_id, amount, tier);
        let voter = self.voter.insecure_clone();
        self.env.send(&[ix], &[&voter]);
        position
    }

    /// The `position_id` the next `stake` must carry.
    ///
    /// `stake` requires `position_id == pool.position_count`, so the id is not the
    /// caller's to choose — it is a pool-wide monotonic counter, shared across all
    /// owners.
    pub fn next_position_id(&self) -> u64 {
        self.env
            .anchor_account::<helix_staking::state::Pool>(&self.pool)
            .position_count
    }

    /// The `proposal_id` the next `create_proposal` must carry — the realm's
    /// counter, for the same reason [`Self::next_position_id`] is the pool's.
    pub fn next_proposal_id(&self) -> u64 {
        self.env
            .anchor_account::<helix_governance::state::Realm>(&self.realm)
            .proposal_count
    }

    /// Gives `owner` lamports for rent and `amount` tokens to stake, returning
    /// their token account and the `position_id` their next stake must carry.
    ///
    /// Stops deliberately short of staking, so a caller that wants to measure or
    /// reject the `stake` instruction itself can build it. [`Self::add_staker`] is
    /// the one-call version.
    pub fn prepare_staker(&mut self, owner: &Pubkey, amount: u64) -> (Pubkey, u64) {
        self.env
            .svm
            .airdrop(owner, 10 * LAMPORTS_PER_SOL)
            .expect("airdrop failed");

        let tokens = self.env.create_token_account(&self.mint, owner).pubkey();
        let authority = self.mint_authority.insecure_clone();
        self.env
            .mint_tokens_raw(&self.mint, &tokens, &authority, amount);

        (tokens, self.next_position_id())
    }

    /// Funds `owner` and opens a position for them.
    ///
    /// Takes the keypair rather than generating one so callers that need a
    /// particular derived address (see `compute_budget.rs`) can supply it.
    pub fn add_staker(&mut self, owner: Keypair, amount: u64, tier: LockTier) -> Staker {
        let (tokens, position_id) = self.prepare_staker(&owner.pubkey(), amount);
        let (position, _) = pda::position(&self.pool, &owner.pubkey(), position_id);

        let ix = self.stake_ix_for(&owner.pubkey(), &tokens, position_id, amount, tier);
        self.env.send(&[ix], &[&owner]);

        Staker {
            owner,
            tokens,
            position,
            position_id,
        }
    }

    pub fn create_proposal(
        &mut self,
        proposal_id: u64,
        action: ProposalAction,
        proposer_position: Pubkey,
    ) -> Pubkey {
        let (proposal, _) = pda::proposal(&self.realm, proposal_id);
        let ix = self.create_proposal_ix(proposal_id, action, proposer_position);
        let voter = self.voter.insecure_clone();
        self.env.send(&[ix], &[&voter]);
        proposal
    }

    pub fn create_proposal_ix(
        &self,
        proposal_id: u64,
        action: ProposalAction,
        proposer_position: Pubkey,
    ) -> solana_instruction::Instruction {
        self.create_proposal_ix_for(&self.voter.pubkey(), proposal_id, action, proposer_position)
    }

    /// The same, for a proposer other than [`Self::voter`].
    pub fn create_proposal_ix_for(
        &self,
        proposer: &Pubkey,
        proposal_id: u64,
        action: ProposalAction,
        proposer_position: Pubkey,
    ) -> solana_instruction::Instruction {
        let (proposal, _) = pda::proposal(&self.realm, proposal_id);

        TestEnv::ix(
            helix_governance::ID,
            helix_governance::accounts::CreateProposal {
                realm: self.realm,
                proposer: *proposer,
                proposer_position,
                owner: *proposer,
                proposal,
                system_program: anchor_lang::system_program::ID,
            },
            helix_governance::instruction::CreateProposal {
                proposal_id,
                action,
                title: "Test proposal".to_string(),
                descriptor_uri: "ipfs://test".to_string(),
            },
        )
    }

    /// Re-tunes the realm. The authority is the payer, so this needs no proposal.
    ///
    /// [`default_realm_params`] pins voting period and timelock at their
    /// permitted minimums so the end-to-end tests can warp seconds instead of
    /// days. That is the right default for a test that drives one proposal by
    /// hand and the wrong one for the fuzzer, whose clock also has to expire
    /// 30-to-180-day stake locks — see `fuzz::FUZZ_REALM_PARAMS`.
    pub fn set_realm_params(&mut self, params: RealmParams) {
        self.try_set_realm_params(params)
            .unwrap_or_else(|e| panic!("set_realm_params failed: {e}"));
    }

    /// The same, returning the failure — for asserting that a superseded
    /// authority has actually lost the power.
    pub fn try_set_realm_params(&mut self, params: RealmParams) -> std::result::Result<(), String> {
        self.env.try_send(
            &[TestEnv::ix(
                helix_governance::ID,
                helix_governance::accounts::UpdateRealmParams {
                    realm: self.realm,
                    authority: self.env.payer_pubkey(),
                },
                helix_governance::instruction::UpdateRealmParams { params },
            )],
            &[],
        )
    }

    fn realm_config_accounts(
        &self,
        proposal: Pubkey,
    ) -> helix_governance::accounts::ExecuteRealmConfig {
        helix_governance::accounts::ExecuteRealmConfig {
            realm: self.realm,
            proposal,
            executor: self.executor,
        }
    }

    pub fn execute_update_realm_params_ix(
        &self,
        proposal: Pubkey,
    ) -> solana_instruction::Instruction {
        TestEnv::ix(
            helix_governance::ID,
            self.realm_config_accounts(proposal),
            helix_governance::instruction::ExecuteUpdateRealmParams {},
        )
    }

    pub fn execute_set_realm_authority_ix(
        &self,
        proposal: Pubkey,
    ) -> solana_instruction::Instruction {
        TestEnv::ix(
            helix_governance::ID,
            self.realm_config_accounts(proposal),
            helix_governance::instruction::ExecuteSetRealmAuthority {},
        )
    }

    pub fn activate(&mut self, proposal: Pubkey) {
        let ix = self.activate_ix(proposal);
        self.env.send(&[ix], &[]);
    }

    pub fn activate_ix(&self, proposal: Pubkey) -> solana_instruction::Instruction {
        TestEnv::ix(
            helix_governance::ID,
            helix_governance::accounts::ActivateProposal {
                realm: self.realm,
                proposal,
                staking_pool: self.pool,
            },
            helix_governance::instruction::ActivateProposal {},
        )
    }

    pub fn vote_ix(
        &self,
        proposal: Pubkey,
        position: Pubkey,
        choice: VoteChoice,
    ) -> solana_instruction::Instruction {
        self.vote_ix_for(&self.voter.pubkey(), proposal, position, choice)
    }

    /// The same, for a voter other than [`Self::voter`].
    pub fn vote_ix_for(
        &self,
        voter: &Pubkey,
        proposal: Pubkey,
        position: Pubkey,
        choice: VoteChoice,
    ) -> solana_instruction::Instruction {
        let (vote_record, _) = pda::vote_record(&proposal, &position);
        TestEnv::ix(
            helix_governance::ID,
            helix_governance::accounts::CastVote {
                realm: self.realm,
                proposal,
                voter: *voter,
                position,
                vote_record,
                system_program: anchor_lang::system_program::ID,
            },
            helix_governance::instruction::CastVote { choice },
        )
    }

    pub fn vote(&mut self, proposal: Pubkey, position: Pubkey, choice: VoteChoice) {
        let ix = self.vote_ix(proposal, position, choice);
        let voter = self.voter.insecure_clone();
        self.env.send(&[ix], &[&voter]);
    }

    pub fn advance_ix(&self, proposal: Pubkey, finalize: bool) -> solana_instruction::Instruction {
        let accounts = helix_governance::accounts::AdvanceProposal {
            realm: self.realm,
            proposal,
        };
        if finalize {
            TestEnv::ix(
                helix_governance::ID,
                accounts,
                helix_governance::instruction::FinalizeProposal {},
            )
        } else {
            TestEnv::ix(
                helix_governance::ID,
                accounts,
                helix_governance::instruction::QueueProposal {},
            )
        }
    }

    pub fn finalize(&mut self, proposal: Pubkey) {
        let ix = self.advance_ix(proposal, true);
        self.env.send(&[ix], &[]);
    }

    pub fn queue(&mut self, proposal: Pubkey) {
        let ix = self.advance_ix(proposal, false);
        self.env.send(&[ix], &[]);
    }

    pub fn cancel_ix(&self, proposal: Pubkey) -> solana_instruction::Instruction {
        TestEnv::ix(
            helix_governance::ID,
            helix_governance::accounts::CancelProposal {
                realm: self.realm,
                guardian: self.guardian.pubkey(),
                proposal,
            },
            helix_governance::instruction::CancelProposal {},
        )
    }

    pub fn cancel(&mut self, proposal: Pubkey) {
        let ix = self.cancel_ix(proposal);
        let guardian = self.guardian.insecure_clone();
        self.env.send(&[ix], &[&guardian]);
    }

    /// Records the outcome of a passed signalling proposal. Moves nothing, which
    /// is what makes it the right action for the fuzzer: it drives the lifecycle
    /// through to `Executed` without dragging the treasury's accounts along.
    pub fn execute_signal_ix(&self, proposal: Pubkey) -> solana_instruction::Instruction {
        TestEnv::ix(
            helix_governance::ID,
            helix_governance::accounts::ExecuteSignal {
                realm: self.realm,
                proposal,
            },
            helix_governance::instruction::ExecuteSignal {},
        )
    }

    /// The instruction that makes a treasury transfer happen. Returned rather
    /// than sent so negative tests can assert on its failure.
    pub fn execute_treasury_transfer_ix(
        &self,
        proposal: Pubkey,
        destination: Pubkey,
    ) -> solana_instruction::Instruction {
        TestEnv::ix(
            helix_governance::ID,
            helix_governance::accounts::ExecuteTreasuryTransfer {
                realm: self.realm,
                proposal,
                executor: self.executor,
                treasury: self.treasury,
                treasury_mint: self.mint,
                treasury_vault: self.treasury_vault,
                destination,
                treasury_vault_authority: self.treasury_vault_authority,
                token_program: token_2022::ID,
                treasury_program: helix_treasury::ID,
            },
            helix_governance::instruction::ExecuteTreasuryTransfer {},
        )
    }

    /// Drives a treasury-transfer proposal all the way through the lifecycle.
    pub fn pass_treasury_transfer(
        &mut self,
        proposal_id: u64,
        position: Pubkey,
        destination: Pubkey,
        amount: u64,
    ) -> Pubkey {
        let proposal = self.create_proposal(
            proposal_id,
            ProposalAction::TreasuryTransfer {
                destination,
                amount,
            },
            position,
        );
        self.activate(proposal);
        self.vote(proposal, position, VoteChoice::For);

        // Voting must close before the outcome can be recorded.
        self.env.warp_forward(HOUR + 1);
        self.finalize(proposal);
        self.queue(proposal);

        // And the timelock must elapse before anything executes.
        self.env.warp_forward(HOUR + 1);
        proposal
    }

    pub fn new_token_account(&mut self, owner: &Pubkey) -> Pubkey {
        self.env.create_token_account(&self.mint, owner).pubkey()
    }

    /// Tops up [`Self::voter`]'s balance.
    ///
    /// `bootstrap` mints them enough for the small stakes most tests use; anything
    /// wanting a realistic position size has to ask for more.
    pub fn fund_voter(&mut self, amount: u64) {
        let authority = self.mint_authority.insecure_clone();
        let tokens = self.voter_tokens;
        let mint = self.mint;
        self.env.mint_tokens_raw(&mint, &tokens, &authority, amount);
    }

    // ------------------------------------------------------- staking rewards

    /// Mints and deposits into the reward vault. Permissionless by design.
    pub fn fund_rewards(&mut self, amount: u64) {
        let ix = self.fund_rewards_ix(amount);
        self.env.send(&[ix], &[]);
    }

    /// Prepares the funder account and returns the deposit unsent. See
    /// [`Self::fund_treasury_ix`].
    pub fn fund_rewards_ix(&mut self, amount: u64) -> solana_instruction::Instruction {
        let payer = self.env.payer_pubkey();
        let funder_tokens = self.env.create_token_account(&self.mint, &payer).pubkey();
        let authority = self.mint_authority.insecure_clone();
        self.env
            .mint_tokens_raw(&self.mint, &funder_tokens, &authority, amount);

        TestEnv::ix(
            helix_staking::ID,
            helix_staking::accounts::FundRewards {
                pool: self.pool,
                funder: payer,
                reward_mint: self.mint,
                funder_token_account: funder_tokens,
                reward_vault: self.reward_vault,
                token_program: token_2022::ID,
            },
            helix_staking::instruction::FundRewards { amount },
        )
    }

    pub fn set_reward_rate_ix(
        &self,
        new_rate: u64,
        reward_period_end: i64,
    ) -> solana_instruction::Instruction {
        TestEnv::ix(
            helix_staking::ID,
            helix_staking::accounts::SetRewardRate {
                pool: self.pool,
                authority: self.env.payer_pubkey(),
                reward_vault: self.reward_vault,
            },
            helix_staking::instruction::SetRewardRate {
                new_rate,
                reward_period_end,
            },
        )
    }

    pub fn set_reward_rate(&mut self, new_rate: u64, reward_period_end: i64) {
        let ix = self.set_reward_rate_ix(new_rate, reward_period_end);
        self.env.send(&[ix], &[]);
    }

    pub fn set_paused_ix(&self, paused: bool) -> solana_instruction::Instruction {
        TestEnv::ix(
            helix_staking::ID,
            helix_staking::accounts::PoolAuthorityOnly {
                pool: self.pool,
                authority: self.env.payer_pubkey(),
            },
            helix_staking::instruction::SetPaused { paused },
        )
    }

    pub fn set_paused(&mut self, paused: bool) {
        let ix = self.set_paused_ix(paused);
        self.env.send(&[ix], &[]);
    }

    pub fn claim_ix(&self, position: Pubkey) -> solana_instruction::Instruction {
        self.claim_ix_for(&self.voter.pubkey(), &self.voter_tokens, position)
    }

    /// The same, for an owner other than [`Self::voter`].
    pub fn claim_ix_for(
        &self,
        owner: &Pubkey,
        owner_tokens: &Pubkey,
        position: Pubkey,
    ) -> solana_instruction::Instruction {
        let (vault_authority, _) = pda::vault_authority(&self.pool);
        TestEnv::ix(
            helix_staking::ID,
            helix_staking::accounts::Claim {
                pool: self.pool,
                owner: *owner,
                position,
                reward_mint: self.mint,
                reward_vault: self.reward_vault,
                owner_reward_account: *owner_tokens,
                vault_authority,
                token_program: token_2022::ID,
            },
            helix_staking::instruction::Claim {},
        )
    }

    pub fn unstake_ix(&self, position: Pubkey, amount: u64) -> solana_instruction::Instruction {
        self.unstake_ix_for(&self.voter.pubkey(), &self.voter_tokens, position, amount)
    }

    /// The same, for an owner other than [`Self::voter`].
    pub fn unstake_ix_for(
        &self,
        owner: &Pubkey,
        owner_tokens: &Pubkey,
        position: Pubkey,
        amount: u64,
    ) -> solana_instruction::Instruction {
        let (vault_authority, _) = pda::vault_authority(&self.pool);
        TestEnv::ix(
            helix_staking::ID,
            helix_staking::accounts::Unstake {
                pool: self.pool,
                owner: *owner,
                position,
                stake_mint: self.mint,
                stake_vault: self.stake_vault,
                owner_token_account: *owner_tokens,
                vault_authority,
                token_program: token_2022::ID,
            },
            helix_staking::instruction::Unstake { amount },
        )
    }

    /// Builds a `stake` instruction without sending it, for negative tests.
    pub fn stake_ix(
        &self,
        position_id: u64,
        amount: u64,
        tier: LockTier,
    ) -> solana_instruction::Instruction {
        self.stake_ix_for(
            &self.voter.pubkey(),
            &self.voter_tokens,
            position_id,
            amount,
            tier,
        )
    }

    /// The same, for an owner other than [`Self::voter`].
    pub fn stake_ix_for(
        &self,
        owner: &Pubkey,
        owner_tokens: &Pubkey,
        position_id: u64,
        amount: u64,
        tier: LockTier,
    ) -> solana_instruction::Instruction {
        let (position, _) = pda::position(&self.pool, owner, position_id);
        TestEnv::ix(
            helix_staking::ID,
            helix_staking::accounts::Stake {
                pool: self.pool,
                owner: *owner,
                position,
                stake_mint: self.mint,
                owner_token_account: *owner_tokens,
                stake_vault: self.stake_vault,
                token_program: token_2022::ID,
                system_program: anchor_lang::system_program::ID,
            },
            helix_staking::instruction::Stake {
                position_id,
                amount,
                tier,
            },
        )
    }

    /// Sends an instruction signed by the voter.
    pub fn send_as_voter(&mut self, ix: solana_instruction::Instruction) {
        let voter = self.voter.insecure_clone();
        self.env.send(&[ix], &[&voter]);
    }

    /// Sends an instruction signed by the voter, returning the failure.
    pub fn try_as_voter(
        &mut self,
        ix: solana_instruction::Instruction,
    ) -> std::result::Result<(), String> {
        let voter = self.voter.insecure_clone();
        self.env.try_send(&[ix], &[&voter])
    }

    /// Sends an instruction signed by the voter, returning its compute cost.
    pub fn compute_as_voter(&mut self, ix: solana_instruction::Instruction) -> u64 {
        let voter = self.voter.insecure_clone();
        self.env.compute_units(&[ix], &[&voter])
    }

    // ------------------------------------------------------- vesting streams

    pub fn execute_create_stream_ix(
        &self,
        proposal: Pubkey,
        stream_id: u64,
        beneficiary: Pubkey,
    ) -> solana_instruction::Instruction {
        let (stream, _) = pda::stream(&self.treasury, stream_id);
        TestEnv::ix(
            helix_governance::ID,
            helix_governance::accounts::ExecuteCreateVestingStream {
                realm: self.realm,
                proposal,
                executor: self.executor,
                payer: self.env.payer_pubkey(),
                treasury: self.treasury,
                beneficiary,
                stream,
                treasury_vault: self.treasury_vault,
                system_program: anchor_lang::system_program::ID,
                treasury_program: helix_treasury::ID,
            },
            helix_governance::instruction::ExecuteCreateVestingStream { stream_id },
        )
    }

    pub fn execute_revoke_stream_ix(
        &self,
        proposal: Pubkey,
        stream_id: u64,
    ) -> solana_instruction::Instruction {
        let (stream, _) = pda::stream(&self.treasury, stream_id);
        TestEnv::ix(
            helix_governance::ID,
            helix_governance::accounts::ExecuteRevokeVestingStream {
                realm: self.realm,
                proposal,
                executor: self.executor,
                treasury: self.treasury,
                stream,
                treasury_program: helix_treasury::ID,
            },
            helix_governance::instruction::ExecuteRevokeVestingStream {},
        )
    }

    pub fn execute_treasury_config_ix(
        &self,
        proposal: Pubkey,
        spend_cap: bool,
    ) -> solana_instruction::Instruction {
        let accounts = helix_governance::accounts::ExecuteTreasuryConfig {
            realm: self.realm,
            proposal,
            executor: self.executor,
            treasury: self.treasury,
            treasury_program: helix_treasury::ID,
        };
        if spend_cap {
            TestEnv::ix(
                helix_governance::ID,
                accounts,
                helix_governance::instruction::ExecuteSetTreasurySpendCap {},
            )
        } else {
            TestEnv::ix(
                helix_governance::ID,
                accounts,
                helix_governance::instruction::ExecuteSetGovernanceExecutor {},
            )
        }
    }

    pub fn claim_stream_ix(
        &self,
        stream_id: u64,
        beneficiary: Pubkey,
        beneficiary_tokens: Pubkey,
    ) -> solana_instruction::Instruction {
        let (stream, _) = pda::stream(&self.treasury, stream_id);
        TestEnv::ix(
            helix_treasury::ID,
            helix_treasury::accounts::ClaimStream {
                treasury: self.treasury,
                stream,
                beneficiary,
                mint: self.mint,
                vault: self.treasury_vault,
                beneficiary_token_account: beneficiary_tokens,
                vault_authority: self.treasury_vault_authority,
                token_program: token_2022::ID,
            },
            helix_treasury::instruction::ClaimStream {},
        )
    }

    /// Drives any proposal through to the point of execution.
    pub fn pass_proposal(
        &mut self,
        proposal_id: u64,
        action: ProposalAction,
        position: Pubkey,
    ) -> Pubkey {
        let proposal = self.create_proposal(proposal_id, action, position);
        self.activate(proposal);
        self.vote(proposal, position, VoteChoice::For);
        self.env.warp_forward(HOUR + 1);
        self.finalize(proposal);
        self.queue(proposal);
        self.env.warp_forward(HOUR + 1);
        proposal
    }
}
