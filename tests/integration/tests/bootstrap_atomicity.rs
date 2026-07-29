//! Phase 3 — is the atomic bootstrap actually possible?
//!
//! [F-1](../../../docs/SECURITY-ASSESSMENT.md#f-1--initialisers-are-front-runnable):
//! `initialize_pool`, `initialize_realm` and `initialize_treasury` each take the
//! privileged party as an unchecked argument, first-caller-wins, with PDAs seeded
//! by mints. An observer can front-run them between deploy and bootstrap and
//! install themselves as authority — permanently, because the seeds are fixed.
//!
//! The recommended mitigation is to run all three in a single transaction so no
//! window exists. That recommendation is worthless if they do not fit: Solana
//! caps a transaction at 1232 bytes and roughly three dozen accounts.
//!
//! This file measures it instead of assuming it.
//!
//! The instructions come from [`helix_ops::plan`], which is what
//! `helix-bootstrap` prints for an operator to read before sending. That is the
//! point of the arrangement: a one-shot transaction against a front-running
//! window cannot be rehearsed on the day, so **the plan an operator reads is the
//! plan this suite has executed** rather than a second implementation of it that
//! happens to look similar.

use anchor_lang::prelude::Pubkey;
use anchor_spl::token_2022;
use helix_integration_tests::bootstrap::default_realm_params;
use helix_integration_tests::{pda, TestEnv};
use helix_ops::BootstrapConfig;
use helix_staking::state::Pool;
use helix_treasury::state::Treasury;
use solana_keypair::Keypair;
use solana_signer::Signer as _;

const DECIMALS: u8 = 9;

/// The plan `helix-bootstrap` would emit for this mint.
fn bootstrap_plan(env: &TestEnv, mint: &Pubkey, guardian: &Pubkey) -> helix_ops::Plan {
    helix_ops::plan(&BootstrapConfig {
        payer: env.payer_pubkey(),
        mint: *mint,
        guardian: *guardian,
        realm: default_realm_params(),
        epoch_spend_cap: 1_000_000_000,
        epoch_duration: 24 * 3_600,
    })
}

#[test]
fn the_whole_bootstrap_fits_in_one_transaction() {
    // The measurement F-1's mitigation depends on.
    let mut env = TestEnv::new();
    let mint_authority = Keypair::new();
    let mint = env
        .create_mint(DECIMALS, &mint_authority.pubkey(), None)
        .pubkey();
    let guardian = Keypair::new().pubkey();

    let plan = bootstrap_plan(&env, &mint, &guardian);

    // One transaction, three programs, no window for anyone to interleave.
    env.send(&plan.instructions, &[]);

    let (pool, _) = pda::pool(&mint, &mint);
    let (realm, _) = pda::realm(&pool);
    let (executor, _) = pda::executor(&realm);
    let (treasury, _) = pda::treasury(&mint);

    // Everything is wired to governance from the first block it exists.
    let p: Pool = env.anchor_account(&pool);
    assert_eq!(
        p.authority, executor,
        "the pool must be governance-controlled from the start"
    );

    let t: Treasury = env.anchor_account(&treasury);
    assert_eq!(
        t.governance_executor, executor,
        "the treasury must accept only the realm executor"
    );
}

#[test]
fn the_bootstrap_transaction_is_within_solana_size_limits() {
    // A transaction that works under LiteSVM could still be rejected by a real
    // cluster for exceeding the 1232-byte packet limit, so measure the serialised
    // size rather than trusting that it executed.
    const SOLANA_TX_SIZE_LIMIT: usize = 1232;

    let mut env = TestEnv::new();
    let mint_authority = Keypair::new();
    let mint = env
        .create_mint(DECIMALS, &mint_authority.pubkey(), None)
        .pubkey();
    let guardian = Keypair::new().pubkey();

    let plan = bootstrap_plan(&env, &mint, &guardian);

    let tx = solana_transaction::Transaction::new_signed_with_payer(
        &plan.instructions,
        Some(&env.payer_pubkey()),
        &[&env.payer],
        env.svm.latest_blockhash(),
    );

    let serialised = bincode::serialize(&tx).expect("transaction should serialise");
    let size = serialised.len();
    let accounts = tx.message.account_keys.len();

    println!("bootstrap transaction: {size} bytes, {accounts} accounts");

    assert!(
        size <= SOLANA_TX_SIZE_LIMIT,
        "bootstrap is {size} bytes, over the {SOLANA_TX_SIZE_LIMIT}-byte limit — \
         F-1's atomic mitigation would fail on a real cluster and the deployer-gate \
         fix is required instead"
    );
}

/// The size `helix-bootstrap` prints is the size that actually gets sent.
///
/// The tool reports a figure an operator uses to decide whether the atomic
/// mitigation is available at all. If it measured a differently-shaped
/// transaction from the one the suite executes, the number would be reassuring
/// and wrong — which is worse than not printing it.
#[test]
fn the_reported_transaction_size_matches_the_real_one() {
    let mut env = TestEnv::new();
    let mint_authority = Keypair::new();
    let mint = env
        .create_mint(DECIMALS, &mint_authority.pubkey(), None)
        .pubkey();
    let guardian = Keypair::new().pubkey();

    let plan = bootstrap_plan(&env, &mint, &guardian);

    let signed = solana_transaction::Transaction::new_signed_with_payer(
        &plan.instructions,
        Some(&env.payer_pubkey()),
        &[&env.payer],
        env.svm.latest_blockhash(),
    );
    let actual = bincode::serialize(&signed).expect("serialise").len();

    assert_eq!(
        plan.transaction_size(),
        actual,
        "the tool would report a size the operator cannot rely on"
    );
    assert_eq!(plan.account_count(), signed.message.account_keys.len());
}

#[test]
fn a_front_runner_cannot_take_the_pool_once_bootstrapped() {
    // The property the atomic bootstrap buys: after it lands, the PDAs are taken
    // and an attacker's initialiser fails.
    let mut env = TestEnv::new();
    let mint_authority = Keypair::new();
    let mint = env
        .create_mint(DECIMALS, &mint_authority.pubkey(), None)
        .pubkey();
    let guardian = Keypair::new().pubkey();

    let plan = bootstrap_plan(&env, &mint, &guardian);
    env.send(&plan.instructions, &[]);

    // An attacker tries to initialise the same pool naming themselves authority.
    let attacker = Keypair::new();
    env.svm
        .airdrop(
            &attacker.pubkey(),
            100 * solana_native_token::LAMPORTS_PER_SOL,
        )
        .unwrap();

    let (pool, _) = pda::pool(&mint, &mint);
    let (pool_vault_authority, _) = pda::vault_authority(&pool);
    let (stake_vault, _) = pda::stake_vault(&pool);
    let (reward_vault, _) = pda::reward_vault(&pool);

    let ix = TestEnv::ix(
        helix_staking::ID,
        helix_staking::accounts::InitializePool {
            payer: attacker.pubkey(),
            authority: attacker.pubkey(),
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
    );

    assert!(
        env.try_send(&[ix], &[&attacker]).is_err(),
        "re-initialising an existing pool must fail"
    );

    let (realm, _) = pda::realm(&pool);
    let (executor, _) = pda::executor(&realm);
    let p: Pool = env.anchor_account(&pool);
    assert_eq!(p.authority, executor, "authority must be unchanged");
}

// ===========================================================================
// INVARIANTS.md §5.8 — the post-deploy authority audit
// ===========================================================================

/// Reads the four authorities out of the accounts the bootstrap wrote.
///
/// This is the part an operator does with an RPC connection. `helix-ops` has no
/// network by design, so the reading is the caller's job and the *judging* is
/// the crate's — which is what lets the same `audit` run here, against accounts
/// written by the real programs, and against devnet later.
fn observe(env: &TestEnv, mint: &Pubkey) -> helix_ops::ObservedAuthorities {
    let (pool, _) = pda::pool(mint, mint);
    let (realm, _) = pda::realm(&pool);
    let (treasury, _) = pda::treasury(mint);

    let p: Pool = env.anchor_account(&pool);
    let r: helix_governance::state::Realm = env.anchor_account(&realm);
    let t: Treasury = env.anchor_account(&treasury);

    helix_ops::ObservedAuthorities {
        pool_authority: p.authority,
        realm_authority: r.authority,
        treasury_spender: t.governance_executor,
        guardian: r.guardian,
    }
}

#[test]
fn the_audit_confirms_the_deployed_system_matches_the_plan() {
    let mut env = TestEnv::new();
    let mint_authority = Keypair::new();
    let mint = env
        .create_mint(DECIMALS, &mint_authority.pubkey(), None)
        .pubkey();
    let guardian = Keypair::new().pubkey();

    let plan = bootstrap_plan(&env, &mint, &guardian);
    env.send(&plan.instructions, &[]);

    let discrepancies = helix_ops::audit(&plan, &observe(&env, &mint));
    assert!(
        discrepancies.is_empty(),
        "clean bootstrap reported as wrong: {:?}",
        discrepancies
    );
}

#[test]
fn the_audit_catches_an_initialiser_that_was_front_run() {
    // §5.8 used to read "initialisers cannot install an unintended authority",
    // which is not true of the programs and cannot be made true — they are
    // first-caller-wins, and that is F-1. What is true is that an unintended
    // authority is *detectable before anything of value is deposited*, and this
    // is the test of that.
    let mut env = TestEnv::new();
    let mint_authority = Keypair::new();
    let mint = env
        .create_mint(DECIMALS, &mint_authority.pubkey(), None)
        .pubkey();
    let guardian = Keypair::new().pubkey();
    let plan = bootstrap_plan(&env, &mint, &guardian);

    // The attacker gets there first, naming themselves pool authority.
    let attacker = Keypair::new();
    env.svm
        .airdrop(
            &attacker.pubkey(),
            100 * solana_native_token::LAMPORTS_PER_SOL,
        )
        .unwrap();

    let (pool, _) = pda::pool(&mint, &mint);
    let (pool_vault_authority, _) = pda::vault_authority(&pool);
    let (stake_vault, _) = pda::stake_vault(&pool);
    let (reward_vault, _) = pda::reward_vault(&pool);
    env.send(
        &[TestEnv::ix(
            helix_staking::ID,
            helix_staking::accounts::InitializePool {
                payer: attacker.pubkey(),
                authority: attacker.pubkey(),
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
        )],
        &[&attacker],
    );

    // The bootstrap now fails as a whole — the first line of defence, and the
    // reason the three initialisers share one transaction. Nothing lands
    // half-built.
    assert!(
        env.try_send(&plan.instructions, &[]).is_err(),
        "the bootstrap succeeded over a pool someone else had already taken"
    );

    // An operator who reacts by re-running without the pool instruction, or who
    // simply proceeds, is the case this catches. The realm and treasury do not
    // exist yet, so audit what does.
    let p: Pool = env.anchor_account(&pool);
    let observed = helix_ops::ObservedAuthorities {
        pool_authority: p.authority,
        realm_authority: plan.addresses.executor,
        treasury_spender: plan.addresses.executor,
        guardian,
    };

    let discrepancies = helix_ops::audit(&plan, &observed);
    assert_eq!(discrepancies.len(), 1);
    assert_eq!(discrepancies[0].what, "pool authority");
    assert_eq!(discrepancies[0].found, attacker.pubkey());
    assert_eq!(discrepancies[0].expected, plan.addresses.executor);

    // And the message is one an operator can act on without reading this test.
    let rendered = discrepancies[0].to_string();
    assert!(
        rendered.contains(&attacker.pubkey().to_string()),
        "{rendered}"
    );
}
