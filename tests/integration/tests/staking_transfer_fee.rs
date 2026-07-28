//! Invariant §2.1 / §2.2 — Token-2022 transfer fees.
//!
//! The highest-value tests in the project, because they are the ones the unit
//! suite structurally cannot reach.
//!
//! Staking credits the *observed vault balance delta*, never the `amount`
//! argument. On a plain SPL mint those two numbers are identical, so every unit
//! test passes whether the code does the right thing or the wrong thing — the
//! correct behaviour could be deleted and nothing would go red. Only a mint with
//! the transfer-fee extension separates them.
//!
//! Each test therefore runs against both mints and asserts the *difference*.

use anchor_lang::prelude::Pubkey;
use helix_integration_tests::{pda, TestEnv, TransferFee};
use helix_staking::state::{LockTier, Pool, Position};
use solana_keypair::Keypair;
use solana_signer::Signer as _;

const DECIMALS: u8 = 9;
const STAKE_AMOUNT: u64 = 1_000_000;

/// 1% fee, uncapped in the range these tests use.
const ONE_PERCENT: TransferFee = TransferFee {
    basis_points: 100,
    maximum_fee: u64::MAX,
};

struct Fixture {
    env: TestEnv,
    mint: Pubkey,
    pool: Pubkey,
    stake_vault: Pubkey,
    staker: Keypair,
    staker_tokens: Pubkey,
}

/// Builds a pool on a mint that either does or does not charge transfer fees.
fn setup(fee: Option<TransferFee>) -> Fixture {
    let mut env = TestEnv::new();

    let mint_authority = Keypair::new();
    let mint_kp = env.create_mint(DECIMALS, &mint_authority.pubkey(), fee);
    let mint = mint_kp.pubkey();

    // Same mint for stake and reward, as in the deployed design.
    let (pool, _) = pda::pool(&mint, &mint);
    let (vault_authority, _) = pda::vault_authority(&pool);
    let (stake_vault, _) = pda::stake_vault(&pool);
    let (reward_vault, _) = pda::reward_vault(&pool);

    let payer = env.payer_pubkey();
    let ix = TestEnv::ix(
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
            token_program: anchor_spl::token_2022::ID,
            system_program: anchor_lang::system_program::ID,
        },
        helix_staking::instruction::InitializePool {},
    );
    env.send(&[ix], &[]);

    // Fund a staker generously enough that fees never starve the test.
    let staker = Keypair::new();
    env.svm
        .airdrop(
            &staker.pubkey(),
            100 * solana_native_token::LAMPORTS_PER_SOL,
        )
        .unwrap();
    let staker_tokens = env.create_token_account(&mint, &staker.pubkey()).pubkey();
    env.mint_tokens_raw(&mint, &staker_tokens, &mint_authority, STAKE_AMOUNT * 10);

    Fixture {
        env,
        mint,
        pool,
        stake_vault,
        staker,
        staker_tokens,
    }
}

impl Fixture {
    fn stake(&mut self, position_id: u64, amount: u64, tier: LockTier) -> Pubkey {
        let (position, _) = pda::position(&self.pool, &self.staker.pubkey(), position_id);

        let ix = TestEnv::ix(
            helix_staking::ID,
            helix_staking::accounts::Stake {
                pool: self.pool,
                owner: self.staker.pubkey(),
                position,
                stake_mint: self.mint,
                owner_token_account: self.staker_tokens,
                stake_vault: self.stake_vault,
                token_program: anchor_spl::token_2022::ID,
                system_program: anchor_lang::system_program::ID,
            },
            helix_staking::instruction::Stake {
                position_id,
                amount,
                tier,
            },
        );

        let staker = self.staker.insecure_clone();
        self.env.send(&[ix], &[&staker]);
        position
    }
}

// ---------------------------------------------------------------------------

#[test]
fn plain_mint_credits_the_full_amount() {
    let mut f = setup(None);

    let before = f.env.token_balance(&f.stake_vault);
    let position = f.stake(0, STAKE_AMOUNT, LockTier::Flexible);
    let after = f.env.token_balance(&f.stake_vault);

    let pos: Position = f.env.anchor_account(&position);

    // With no fee, sent == received == credited. This is the baseline the
    // fee-bearing case is compared against.
    assert_eq!(after - before, STAKE_AMOUNT);
    assert_eq!(pos.amount, STAKE_AMOUNT);
}

#[test]
fn fee_bearing_mint_credits_the_delta_not_the_argument() {
    // INVARIANTS.md §2.1 — the whole reason this file exists.
    let mut f = setup(Some(ONE_PERCENT));

    let expected_fee = ONE_PERCENT.expected_on(STAKE_AMOUNT);
    assert!(expected_fee > 0, "test is meaningless without a real fee");

    let before = f.env.token_balance(&f.stake_vault);
    let position = f.stake(0, STAKE_AMOUNT, LockTier::Flexible);
    let after = f.env.token_balance(&f.stake_vault);

    let credited = after - before;
    let pos: Position = f.env.anchor_account(&position);

    // The vault received less than was sent...
    assert_eq!(credited, STAKE_AMOUNT - expected_fee);
    // ...and the position was credited what arrived, not what was sent.
    assert_eq!(
        pos.amount, credited,
        "position credited {} but vault received {credited} — this is the bug \
         §2.1 exists to catch",
        pos.amount
    );
    assert_ne!(
        pos.amount, STAKE_AMOUNT,
        "position was credited the argument rather than the delta"
    );
}

#[test]
fn fee_bearing_mint_preserves_vault_solvency() {
    // INVARIANTS.md §2.2 and §1.1 — the consequence of getting §2.1 wrong.
    //
    // If the position were credited `amount` while the vault received less, the
    // pool would be insolvent from the first deposit, and repeated stake/unstake
    // cycles would drain it at other stakers' expense.
    let mut f = setup(Some(ONE_PERCENT));

    let mut positions = Vec::new();
    for id in 0..3u64 {
        positions.push(f.stake(id, STAKE_AMOUNT, LockTier::Flexible));
    }

    let vault_balance = f.env.token_balance(&f.stake_vault);
    let summed: u64 = positions
        .iter()
        .map(|p| f.env.anchor_account::<Position>(p).amount)
        .sum();

    // §1.1 — Σ position.amount == stake_vault.amount
    assert_eq!(
        summed, vault_balance,
        "sum of positions ({summed}) does not match vault ({vault_balance})"
    );

    // §1.3 — the pool's own counter agrees
    let pool: Pool = f.env.anchor_account(&f.pool);
    assert_eq!(pool.total_staked, vault_balance);

    // And the pool is genuinely holding less than was sent to it.
    assert!(vault_balance < STAKE_AMOUNT * 3);
}

#[test]
fn weighted_amount_is_derived_from_the_credited_amount() {
    // A subtler form of the same bug: crediting the delta but computing weight
    // from the argument would inflate reward share and vote power by the fee.
    let mut f = setup(Some(ONE_PERCENT));

    let position = f.stake(0, STAKE_AMOUNT, LockTier::Gold);
    let pos: Position = f.env.anchor_account(&position);

    let expected_weight = LockTier::Gold.apply_weight(pos.amount).unwrap();
    assert_eq!(pos.weighted_amount, expected_weight);

    // Explicitly: not the weight of the pre-fee amount.
    let inflated = LockTier::Gold.apply_weight(STAKE_AMOUNT).unwrap();
    assert_ne!(pos.weighted_amount, inflated);

    let pool: Pool = f.env.anchor_account(&f.pool);
    assert_eq!(pool.total_weighted, pos.weighted_amount);
}
