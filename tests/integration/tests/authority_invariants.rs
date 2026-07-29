//! INVARIANTS.md §5.3 and §5.5 — the two rows that named a test and had none.
//!
//! Both had sat at ⬜ with a plausible-looking test name beside them, which is
//! worse than an empty cell: a reader skimming the table sees a name and reads
//! it as verified. These are those tests.
//!
//! Neither property is provable by unit test. §5.3 is about the state of a
//! Token-2022 mint account after an instruction ran, and §5.5 is about what the
//! *deployed* program does with an address it is handed — which is precisely
//! what a unit test replaces with a function call.

use anchor_lang::prelude::Pubkey;
use anchor_spl::token_2022::{self, spl_token_2022};
use helix_integration_tests::bootstrap::System;
use helix_integration_tests::{pda, TestEnv};
use helix_staking::state::{LockTier, Pool, Position};
use helix_token_manager::instructions::initialize_token::InitializeTokenArgs;
use helix_token_manager::state::TokenConfig;
use solana_keypair::Keypair;
use solana_signer::Signer as _;

// ===========================================================================
// §5.3 — no non-PDA address holds mint authority after `initialize_token`
// ===========================================================================

struct Minted {
    env: TestEnv,
    admin: Keypair,
    mint: Pubkey,
    config: Pubkey,
    mint_authority: Pubkey,
}

/// Runs `initialize_token` and nothing else.
fn initialize_token() -> Minted {
    let mut env = TestEnv::new();
    let payer = env.payer_pubkey();

    // A distinct key, so "the admin does not hold the mint authority" is an
    // observation rather than a coincidence of two names for the payer.
    let admin = Keypair::new();
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
                    decimals: 9,
                    name: "Helix".to_string(),
                    symbol: "HLX".to_string(),
                    uri: "https://helix.example/hlx.json".to_string(),
                },
            },
        )],
        &[&mint_kp],
    );

    Minted {
        env,
        admin,
        mint,
        config,
        mint_authority,
    }
}

/// Reads the two authorities straight off the Token-2022 mint account.
fn authorities(env: &TestEnv, mint: &Pubkey) -> (Option<Pubkey>, Option<Pubkey>) {
    use spl_token_2022::extension::StateWithExtensions;
    let raw = env.svm.get_account(mint).expect("mint exists");
    let state =
        StateWithExtensions::<spl_token_2022::state::Mint>::unpack(&raw.data).expect("not a mint");
    (
        state.base.mint_authority.into(),
        state.base.freeze_authority.into(),
    )
}

#[test]
fn mint_authority_is_pda() {
    let m = initialize_token();
    let (mint_authority, freeze_authority) = authorities(&m.env, &m.mint);

    assert_eq!(mint_authority, Some(m.mint_authority));
    assert_eq!(freeze_authority, Some(m.mint_authority));

    // The three keys that were in the room when the mint was created hold
    // neither authority. This is the whole content of §5.3: after this one
    // instruction, no signature anywhere can produce HLX except through the
    // minter registry.
    for key in [m.admin.pubkey(), m.env.payer_pubkey(), m.mint] {
        assert_ne!(mint_authority, Some(key));
        assert_ne!(freeze_authority, Some(key));
    }

    // And the PDA is the one this program derives, not merely *some* off-curve
    // address — otherwise the authority could belong to a program that no longer
    // exists, which is indistinguishable from burned right up until it isn't.
    let config: TokenConfig = m.env.anchor_account(&m.config);
    let (expected, bump) = pda::mint_authority(&m.config);
    assert_eq!(m.mint_authority, expected);
    assert_eq!(config.mint_authority_bump, bump);
}

#[test]
fn no_key_present_at_creation_can_mint() {
    // The property §5.3 exists to guarantee, asserted the only way that counts:
    // by trying. A `mint_to` straight to Token-2022, bypassing the registry.
    let mut m = initialize_token();
    let destination = m
        .env
        .create_token_account(&m.mint, &m.admin.pubkey())
        .pubkey();

    for signer in [m.admin.insecure_clone(), m.env.payer.insecure_clone()] {
        let ix = spl_token_2022::instruction::mint_to(
            &token_2022::ID,
            &m.mint,
            &destination,
            &signer.pubkey(),
            &[],
            1_000,
        )
        .expect("build mint_to");

        let err = m
            .env
            .try_send(&[ix], &[&signer])
            .expect_err("a key minted HLX directly");
        assert!(
            err.contains("OwnerMismatch") || err.contains("owner does not match"),
            "{err}"
        );
    }

    assert_eq!(m.env.token_balance(&destination), 0);
}

// ===========================================================================
// §5.5 — every PDA is derived with a stored, verified bump
// ===========================================================================

#[test]
fn canonical_bumps() {
    // Half one: every bump a program persisted is the canonical one.
    //
    // This is what makes `bump = <stored>` safe. Anchor re-derives the address
    // from the seeds and the *stored* bump, so a non-canonical value recorded at
    // init would give a second address that satisfies every constraint — the
    // same logical account twice, with independent balances.
    let mut sys = System::bootstrap(None, 1_000_000);
    let position = sys.stake(0, 1_000_000, LockTier::Gold);

    let pool: Pool = sys.env.anchor_account(&sys.pool);
    assert_eq!(pool.bump, pda::pool(&sys.mint, &sys.mint).1);
    assert_eq!(pool.vault_authority_bump, pda::vault_authority(&sys.pool).1);

    let p: Position = sys.env.anchor_account(&position);
    assert_eq!(p.bump, pda::position(&sys.pool, &sys.voter.pubkey(), 0).1);

    let realm: helix_governance::state::Realm = sys.env.anchor_account(&sys.realm);
    assert_eq!(realm.bump, pda::realm(&sys.pool).1);
    assert_eq!(realm.executor_bump, pda::executor(&sys.realm).1);

    let treasury: helix_treasury::state::Treasury = sys.env.anchor_account(&sys.treasury);
    assert_eq!(treasury.bump, pda::treasury(&sys.mint).1);
    assert_eq!(
        treasury.vault_authority_bump,
        pda::treasury_vault_authority(&sys.treasury).1
    );
}

/// The largest bump below `canonical` that is still a valid off-curve address
/// for these seeds — a second, non-canonical PDA for the same logical account.
fn non_canonical(seeds: &[&[u8]], program: &Pubkey, canonical: u8) -> (Pubkey, u8) {
    for bump in (0..canonical).rev() {
        let mut with_bump: Vec<&[u8]> = seeds.to_vec();
        let byte = [bump];
        with_bump.push(&byte);
        if let Ok(address) = Pubkey::create_program_address(&with_bump, program) {
            return (address, bump);
        }
    }
    panic!("no non-canonical bump exists for these seeds");
}

#[test]
fn a_non_canonical_derivation_is_refused() {
    // Half two, and the half that is actually about the deployed program:
    // storing a canonical bump is worth nothing unless the program *checks* the
    // address it is handed against it.
    //
    // The account under test is `unstake`'s `vault_authority`. It is an
    // `UncheckedAccount` — never deserialised, never owner-checked — whose only
    // job is to sign the transfer out of the stake vault. The seeds constraint
    // is the entire thing standing between an attacker and a forged signer for
    // the vault, so it gets a test rather than an argument.
    let mut sys = System::bootstrap(None, 0);
    let position = sys.stake(0, 1_000_000, LockTier::Flexible);

    let (canonical_authority, canonical_bump) = pda::vault_authority(&sys.pool);
    let pool_key = sys.pool;
    let (forged, forged_bump) = non_canonical(
        &[b"vault_authority", pool_key.as_ref()],
        &helix_staking::ID,
        canonical_bump,
    );
    assert_ne!(forged, canonical_authority);
    assert!(forged_bump < canonical_bump);

    // A real PDA of the same program, from the same seeds, at a different bump.
    // Only the canonical one is the vault authority.
    let mut ix = sys.unstake_ix(position, 1_000_000);
    let slot = ix
        .accounts
        .iter()
        .position(|a| a.pubkey == canonical_authority)
        .expect("vault_authority is in the account list");
    ix.accounts[slot].pubkey = forged;

    let voter = sys.voter.insecure_clone();
    let err = sys
        .env
        .try_send(&[ix], &[&voter])
        .expect_err("a non-canonical vault authority was accepted");
    assert!(err.contains("ConstraintSeeds"), "{err}");

    // The vault still holds everything, and the canonical path still works.
    assert_eq!(sys.env.token_balance(&sys.stake_vault), 1_000_000);
    sys.env
        .send(&[sys.unstake_ix(position, 1_000_000)], &[&voter]);
    assert_eq!(sys.env.token_balance(&sys.stake_vault), 0);
}
