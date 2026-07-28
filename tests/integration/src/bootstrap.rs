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

impl System {
    /// Builds everything, with the treasury's sole spender set to the realm's
    /// executor PDA.
    pub fn bootstrap(fee: Option<TransferFee>, treasury_funding: u64) -> Self {
        let mut env = TestEnv::new();
        let payer = env.payer_pubkey();

        let mint_authority = Keypair::new();
        let mint = env
            .create_mint(DECIMALS, &mint_authority.pubkey(), fee)
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
        let payer = self.env.payer_pubkey();
        let funder_tokens = self.env.create_token_account(&self.mint, &payer).pubkey();
        let authority = self.mint_authority.insecure_clone();
        self.env
            .mint_tokens_raw(&self.mint, &funder_tokens, &authority, amount);

        self.env.send(
            &[TestEnv::ix(
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
            )],
            &[],
        );
    }

    /// Opens a staking position for the voter.
    pub fn stake(&mut self, position_id: u64, amount: u64, tier: LockTier) -> Pubkey {
        let (position, _) = pda::position(&self.pool, &self.voter.pubkey(), position_id);

        let ix = TestEnv::ix(
            helix_staking::ID,
            helix_staking::accounts::Stake {
                pool: self.pool,
                owner: self.voter.pubkey(),
                position,
                stake_mint: self.mint,
                owner_token_account: self.voter_tokens,
                stake_vault: self.stake_vault,
                token_program: token_2022::ID,
                system_program: anchor_lang::system_program::ID,
            },
            helix_staking::instruction::Stake {
                position_id,
                amount,
                tier,
            },
        );

        let voter = self.voter.insecure_clone();
        self.env.send(&[ix], &[&voter]);
        position
    }

    pub fn create_proposal(
        &mut self,
        proposal_id: u64,
        action: ProposalAction,
        proposer_position: Pubkey,
    ) -> Pubkey {
        let (proposal, _) = pda::proposal(&self.realm, proposal_id);

        let ix = TestEnv::ix(
            helix_governance::ID,
            helix_governance::accounts::CreateProposal {
                realm: self.realm,
                proposer: self.voter.pubkey(),
                proposer_position,
                owner: self.voter.pubkey(),
                proposal,
                system_program: anchor_lang::system_program::ID,
            },
            helix_governance::instruction::CreateProposal {
                proposal_id,
                action,
                title: "Test proposal".to_string(),
                descriptor_uri: "ipfs://test".to_string(),
            },
        );

        let voter = self.voter.insecure_clone();
        self.env.send(&[ix], &[&voter]);
        proposal
    }

    pub fn activate(&mut self, proposal: Pubkey) {
        self.env.send(
            &[TestEnv::ix(
                helix_governance::ID,
                helix_governance::accounts::ActivateProposal {
                    realm: self.realm,
                    proposal,
                    staking_pool: self.pool,
                },
                helix_governance::instruction::ActivateProposal {},
            )],
            &[],
        );
    }

    pub fn vote_ix(
        &self,
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
                voter: self.voter.pubkey(),
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

    pub fn cancel(&mut self, proposal: Pubkey) {
        let ix = TestEnv::ix(
            helix_governance::ID,
            helix_governance::accounts::CancelProposal {
                realm: self.realm,
                guardian: self.guardian.pubkey(),
                proposal,
            },
            helix_governance::instruction::CancelProposal {},
        );
        let guardian = self.guardian.insecure_clone();
        self.env.send(&[ix], &[&guardian]);
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
}
