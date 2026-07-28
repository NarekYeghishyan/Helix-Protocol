//! Phase 3.5 / F-9 — handing the token-manager admin to governance.
//!
//! This is the one authority that cannot be wired at bootstrap. `register_minter`
//! must run before the staking program can pay rewards, and only an admin can
//! register a minter — so the admin starts as a human key and is handed over
//! afterwards.
//!
//! `accept_admin` requires the incoming admin to sign, and the executor PDA signs
//! only inside a governance `execute_*`. Before
//! [F-9](../../../docs/SECURITY-ASSESSMENT.md#f-9--token-manager-admin-cannot-be-handed-to-governance)
//! was fixed, no such instruction existed and the handover was impossible.
//!
//! The tests deliberately follow the real deployment order, including the
//! chicken-and-egg, rather than shortcutting it.

use anchor_lang::prelude::Pubkey;
use anchor_spl::token_2022;
use helix_governance::state::{ProposalAction, VoteChoice};
use helix_integration_tests::bootstrap::{default_realm_params, HOUR};
use helix_integration_tests::{pda, TestEnv};
use helix_staking::state::LockTier;
use helix_token_manager::instructions::initialize_token::InitializeTokenArgs;
use helix_token_manager::state::{Minter, TokenConfig};
use solana_keypair::Keypair;
use solana_signer::Signer as _;

const DECIMALS: u8 = 9;
const STAKE: u64 = 1_000_000;
const EPOCH_CAP: u64 = 100_000_000;
const DAY: i64 = 86_400;

struct Fixture {
    env: TestEnv,
    /// The human key that starts as admin.
    admin: Keypair,
    mint: Pubkey,
    config: Pubkey,
    realm: Pubkey,
    executor: Pubkey,
    pool: Pubkey,
    voter: Keypair,
    voter_tokens: Pubkey,
    position: Pubkey,
}

/// Builds the system in the order a real deployment must use.
fn setup() -> Fixture {
    let mut env = TestEnv::new();
    let payer = env.payer_pubkey();

    // The admin is a distinct key, so "governance took over" is observable rather
    // than coincidentally equal to the payer.
    let admin = Keypair::new();
    env.svm
        .airdrop(&admin.pubkey(), 100 * solana_native_token::LAMPORTS_PER_SOL)
        .unwrap();

    // 1. Create the HLX mint. Its authority is a PDA from this instruction on, so
    //    no key can ever mint directly.
    let mint_kp = Keypair::new();
    let mint = mint_kp.pubkey();
    let (config, _) = pda::token_config(&mint);
    let (mint_authority, _) = pda::mint_authority(&config);

    env.send(
        &[TestEnv::ix(
            helix_token_manager::ID,
            helix_token_manager::accounts::InitializeToken {
                payer,
                admin: admin.pubkey(),
                config,
                mint_authority,
                mint,
                token_program: token_2022::ID,
                system_program: anchor_lang::system_program::ID,
            },
            helix_token_manager::instruction::InitializeToken {
                args: InitializeTokenArgs {
                    decimals: DECIMALS,
                    name: "Helix".to_string(),
                    symbol: "HLX".to_string(),
                    uri: "https://helix.example/hlx.json".to_string(),
                },
            },
        )],
        &[&mint_kp],
    );

    // 2. The admin registers itself as a minter. This is the step that forces the
    //    admin to start as a key: nothing can be minted until it happens, and only
    //    an admin can do it.
    let (minter, _) = pda::minter(&config, &admin.pubkey());
    let admin_clone = admin.insecure_clone();
    env.send(
        &[TestEnv::ix(
            helix_token_manager::ID,
            helix_token_manager::accounts::RegisterMinter {
                config,
                admin: admin.pubkey(),
                payer,
                minter,
                system_program: anchor_lang::system_program::ID,
            },
            helix_token_manager::instruction::RegisterMinter {
                authority: admin.pubkey(),
                epoch_cap: EPOCH_CAP,
                epoch_duration: DAY,
            },
        )],
        &[&admin_clone],
    );

    // 3. Mint the voter a stake through the gated path.
    let voter = Keypair::new();
    env.svm
        .airdrop(&voter.pubkey(), 100 * solana_native_token::LAMPORTS_PER_SOL)
        .unwrap();
    let voter_tokens = env.create_token_account(&mint, &voter.pubkey()).pubkey();

    let admin_clone = admin.insecure_clone();
    env.send(
        &[TestEnv::ix(
            helix_token_manager::ID,
            helix_token_manager::accounts::MintTokens {
                config,
                minter,
                authority: admin.pubkey(),
                mint,
                mint_authority,
                recipient: voter_tokens,
                token_program: token_2022::ID,
            },
            helix_token_manager::instruction::MintTokens { amount: STAKE * 10 },
        )],
        &[&admin_clone],
    );

    // 4. Pool and realm, both naming the executor PDA at initialisation.
    let (pool, _) = pda::pool(&mint, &mint);
    let (pool_vault_authority, _) = pda::vault_authority(&pool);
    let (stake_vault, _) = pda::stake_vault(&pool);
    let (reward_vault, _) = pda::reward_vault(&pool);
    let (realm, _) = pda::realm(&pool);
    let (executor, _) = pda::executor(&realm);

    env.send(
        &[
            TestEnv::ix(
                helix_staking::ID,
                helix_staking::accounts::InitializePool {
                    payer,
                    authority: executor,
                    pool,
                    vault_authority: pool_vault_authority,
                    stake_mint: mint,
                    reward_mint: mint,
                    stake_vault,
                    reward_vault,
                    token_program: token_2022::ID,
                    system_program: anchor_lang::system_program::ID,
                },
                helix_staking::instruction::InitializePool {},
            ),
            TestEnv::ix(
                helix_governance::ID,
                helix_governance::accounts::InitializeRealm {
                    payer,
                    authority: executor,
                    guardian: Keypair::new().pubkey(),
                    staking_pool: pool,
                    realm,
                    executor,
                    system_program: anchor_lang::system_program::ID,
                },
                helix_governance::instruction::InitializeRealm {
                    params: default_realm_params(),
                },
            ),
        ],
        &[],
    );

    // 5. Stake, so there is vote weight.
    let (position, _) = pda::position(&pool, &voter.pubkey(), 0);
    let voter_clone = voter.insecure_clone();
    env.send(
        &[TestEnv::ix(
            helix_staking::ID,
            helix_staking::accounts::Stake {
                pool,
                owner: voter.pubkey(),
                position,
                stake_mint: mint,
                owner_token_account: voter_tokens,
                stake_vault,
                token_program: token_2022::ID,
                system_program: anchor_lang::system_program::ID,
            },
            helix_staking::instruction::Stake {
                position_id: 0,
                amount: STAKE,
                tier: LockTier::Gold,
            },
        )],
        &[&voter_clone],
    );

    Fixture {
        env,
        admin,
        mint,
        config,
        realm,
        executor,
        pool,
        voter,
        voter_tokens,
        position,
    }
}

impl Fixture {
    /// Drives a proposal to the point of execution.
    fn pass(&mut self, id: u64, action: ProposalAction) -> Pubkey {
        let (proposal, _) = pda::proposal(&self.realm, id);
        let voter = self.voter.insecure_clone();

        self.env.send(
            &[TestEnv::ix(
                helix_governance::ID,
                helix_governance::accounts::CreateProposal {
                    realm: self.realm,
                    proposer: self.voter.pubkey(),
                    proposer_position: self.position,
                    owner: self.voter.pubkey(),
                    proposal,
                    system_program: anchor_lang::system_program::ID,
                },
                helix_governance::instruction::CreateProposal {
                    proposal_id: id,
                    action,
                    title: "t".to_string(),
                    descriptor_uri: "u".to_string(),
                },
            )],
            &[&voter],
        );

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

        let (vote_record, _) = pda::vote_record(&proposal, &self.position);
        self.env.send(
            &[TestEnv::ix(
                helix_governance::ID,
                helix_governance::accounts::CastVote {
                    realm: self.realm,
                    proposal,
                    voter: self.voter.pubkey(),
                    position: self.position,
                    vote_record,
                    system_program: anchor_lang::system_program::ID,
                },
                helix_governance::instruction::CastVote {
                    choice: VoteChoice::For,
                },
            )],
            &[&voter],
        );

        self.env.warp_forward(HOUR + 1);
        let advance = helix_governance::accounts::AdvanceProposal {
            realm: self.realm,
            proposal,
        };
        self.env.send(
            &[TestEnv::ix(
                helix_governance::ID,
                advance,
                helix_governance::instruction::FinalizeProposal {},
            )],
            &[],
        );
        self.env.send(
            &[TestEnv::ix(
                helix_governance::ID,
                advance,
                helix_governance::instruction::QueueProposal {},
            )],
            &[],
        );
        self.env.warp_forward(HOUR + 1);

        proposal
    }

    fn token_admin_accounts(
        &self,
        proposal: Pubkey,
    ) -> helix_governance::accounts::ExecuteTokenAdmin {
        helix_governance::accounts::ExecuteTokenAdmin {
            realm: self.realm,
            proposal,
            executor: self.executor,
            token_config: self.config,
            token_manager_program: helix_token_manager::ID,
        }
    }

    /// Runs propose_admin (as the human admin) then the governance-side accept.
    fn hand_admin_to_governance(&mut self, proposal_id: u64) {
        let admin = self.admin.insecure_clone();
        self.env.send(
            &[TestEnv::ix(
                helix_token_manager::ID,
                helix_token_manager::accounts::AdminOnly {
                    config: self.config,
                    admin: self.admin.pubkey(),
                },
                helix_token_manager::instruction::ProposeAdmin {
                    new_admin: self.executor,
                },
            )],
            &[&admin],
        );

        let proposal = self.pass(proposal_id, ProposalAction::AcceptTokenManagerAdmin);
        let ix = TestEnv::ix(
            helix_governance::ID,
            self.token_admin_accounts(proposal),
            helix_governance::instruction::ExecuteAcceptTokenManagerAdmin {},
        );
        self.env.send(&[ix], &[]);
    }
}

// ---------------------------------------------------------------------------

#[test]
fn governance_can_accept_the_token_manager_admin() {
    // F-9's regression test. This sequence was impossible before the fix.
    let mut f = setup();

    let before: TokenConfig = f.env.anchor_account(&f.config);
    assert_eq!(before.admin, f.admin.pubkey());

    f.hand_admin_to_governance(0);

    let after: TokenConfig = f.env.anchor_account(&f.config);
    assert_eq!(
        after.admin, f.executor,
        "the realm executor must now be the admin"
    );
    assert!(
        after.pending_admin.is_none(),
        "the pending handover must be cleared"
    );
}

#[test]
fn the_old_admin_loses_its_powers_after_handover() {
    let mut f = setup();
    f.hand_admin_to_governance(0);

    // The previous admin can no longer pause issuance.
    let admin = f.admin.insecure_clone();
    let ix = TestEnv::ix(
        helix_token_manager::ID,
        helix_token_manager::accounts::AdminOnly {
            config: f.config,
            admin: f.admin.pubkey(),
        },
        helix_token_manager::instruction::SetPaused { paused: true },
    );
    let err = f
        .env
        .try_send(&[ix], &[&admin])
        .expect_err("the superseded admin must have no powers");
    assert!(err.contains("NotAdmin"), "unexpected failure: {err}");

    let config: TokenConfig = f.env.anchor_account(&f.config);
    assert!(!config.paused);
}

#[test]
fn governance_can_pause_issuance_once_it_is_admin() {
    // The point of F-9's full fix: holding the role is useless without the powers.
    let mut f = setup();
    f.hand_admin_to_governance(0);

    let proposal = f.pass(1, ProposalAction::SetTokenPaused { paused: true });
    let ix = TestEnv::ix(
        helix_governance::ID,
        f.token_admin_accounts(proposal),
        helix_governance::instruction::ExecuteSetTokenPaused {},
    );
    f.env.send(&[ix], &[]);

    let config: TokenConfig = f.env.anchor_account(&f.config);
    assert!(config.paused, "governance must be able to halt issuance");
}

#[test]
fn governance_can_register_a_new_minter() {
    let mut f = setup();
    f.hand_admin_to_governance(0);

    let new_authority = Keypair::new().pubkey();
    let (minter, _) = pda::minter(&f.config, &new_authority);

    let proposal = f.pass(
        1,
        ProposalAction::RegisterMinter {
            authority: new_authority,
            epoch_cap: 42_000,
            epoch_duration: DAY,
        },
    );

    let ix = TestEnv::ix(
        helix_governance::ID,
        helix_governance::accounts::ExecuteRegisterMinter {
            realm: f.realm,
            proposal,
            executor: f.executor,
            payer: f.env.payer_pubkey(),
            token_config: f.config,
            minter,
            system_program: anchor_lang::system_program::ID,
            token_manager_program: helix_token_manager::ID,
        },
        helix_governance::instruction::ExecuteRegisterMinter {},
    );
    f.env.send(&[ix], &[]);

    let m: Minter = f.env.anchor_account(&minter);
    assert_eq!(m.authority, new_authority);
    assert_eq!(m.epoch_cap, 42_000);
    assert!(m.enabled);
}

#[test]
fn governance_can_revoke_a_minter() {
    let mut f = setup();
    let (minter, _) = pda::minter(&f.config, &f.admin.pubkey());
    f.hand_admin_to_governance(0);

    let proposal = f.pass(1, ProposalAction::RevokeMinter);
    let ix = TestEnv::ix(
        helix_governance::ID,
        helix_governance::accounts::ExecuteModifyMinter {
            realm: f.realm,
            proposal,
            executor: f.executor,
            token_config: f.config,
            minter,
            token_manager_program: helix_token_manager::ID,
        },
        helix_governance::instruction::ExecuteRevokeMinter {},
    );
    f.env.send(&[ix], &[]);

    let m: Minter = f.env.anchor_account(&minter);
    assert!(!m.enabled, "the minter must be disabled");
    assert_eq!(m.epoch_cap, 0);

    // Issuance through it now fails, which is the point.
    let admin = f.admin.insecure_clone();
    let (mint_authority, _) = pda::mint_authority(&f.config);
    let ix = TestEnv::ix(
        helix_token_manager::ID,
        helix_token_manager::accounts::MintTokens {
            config: f.config,
            minter,
            authority: f.admin.pubkey(),
            mint: f.mint,
            mint_authority,
            recipient: f.voter_tokens,
            token_program: token_2022::ID,
        },
        helix_token_manager::instruction::MintTokens { amount: 1 },
    );
    assert!(
        f.env.try_send(&[ix], &[&admin]).is_err(),
        "a revoked minter must not be able to mint"
    );
}

#[test]
fn a_token_action_cannot_be_executed_through_the_wrong_handler() {
    let mut f = setup();
    f.hand_admin_to_governance(0);

    let proposal = f.pass(1, ProposalAction::SetTokenPaused { paused: true });

    // Same proposal, wrong execute instruction.
    let ix = TestEnv::ix(
        helix_governance::ID,
        f.token_admin_accounts(proposal),
        helix_governance::instruction::ExecuteAcceptTokenManagerAdmin {},
    );
    let err = f
        .env
        .try_send(&[ix], &[])
        .expect_err("a pause proposal must not run as an admin acceptance");
    assert!(
        err.contains("ActionAccountMismatch"),
        "unexpected failure: {err}"
    );
}

#[test]
fn nobody_but_governance_can_drive_the_token_admin() {
    // The executor PDA is the admin, and only an execute_* produces its signature.
    let mut f = setup();
    f.hand_admin_to_governance(0);

    let attacker = Keypair::new();
    f.env
        .svm
        .airdrop(
            &attacker.pubkey(),
            10 * solana_native_token::LAMPORTS_PER_SOL,
        )
        .unwrap();

    let ix = TestEnv::ix(
        helix_token_manager::ID,
        helix_token_manager::accounts::AdminOnly {
            config: f.config,
            admin: attacker.pubkey(),
        },
        helix_token_manager::instruction::SetPaused { paused: true },
    );
    let err = f
        .env
        .try_send(&[ix], &[&attacker])
        .expect_err("an arbitrary signer must not act as admin");
    assert!(err.contains("NotAdmin"), "unexpected failure: {err}");
}
