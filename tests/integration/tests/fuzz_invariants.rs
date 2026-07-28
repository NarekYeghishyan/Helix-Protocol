//! Stateful fuzzing over staking and governance — ROADMAP phases 6.1 and 6.2.
//!
//! The generator, oracle and shrinker live in `helix_integration_tests::fuzz`; this
//! file is the campaign, plus the tests that keep the campaign honest. Three
//! things have to hold for a fuzzer to be worth its runtime:
//!
//! 1. **It exercises the protocol.** `the_fuzzer_is_not_vacuous` fails if any
//!    operation stops being accepted, or stops being rejected. A generator that
//!    drifts until every `set_reward_rate` is unaffordable still passes every
//!    invariant, and proves nothing.
//! 2. **The oracle would notice.** `the_oracle_notices_corrupted_state` writes
//!    bad values straight into the accounts and requires the matching invariant
//!    to name itself. Without it, an oracle that silently returned `Ok` would be
//!    indistinguishable from a correct protocol.
//! 3. **A failure is actionable.** `the_shrinker_reduces_to_the_minimal_case`
//!    checks the delta-debugging loop against a predicate whose minimum is
//!    known, because the real thing — reducing an actual bug — is a code path
//!    that never runs while the programs are correct.

use anchor_lang::{AnchorSerialize, Discriminator};
use helix_governance::state::{Proposal, ProposalState};
use helix_integration_tests::fuzz::{sequence, shrink, shrink_with, Coverage, Fuzzer, Op};
use helix_integration_tests::TransferFee;
use helix_staking::state::Pool;

/// Sequence length, set by measurement rather than taste.
///
/// The governance lifecycle is six ordered steps against one proposal, each
/// gated on state and most of them on the clock, so the chance of a random
/// sequence assembling one decays fast with distance. At 60 operations the
/// campaign never reached `queue`: proposals piled up in `Voting` and `Defeated`
/// and the whole second half of the state machine — the timelock, expiry, double
/// execution — went untested while every invariant passed.
///
/// Measured across 16 seeds: 60 ops reached `queue` 0 times, 90 twice, 120 three
/// times, 150 ten times with `execute` landing 3 times and `TimelockNotElapsed`
/// and `ProposalExpired` both exercised. Past 150 the curve flattens and the
/// runtime does not.
const OPS: usize = 150;

/// Runs one seed per iteration and returns the merged coverage.
///
/// On failure the sequence is shrunk before it is reported, so what lands in the
/// output is the short case rather than the full-length original.
fn sweep(fee: Option<TransferFee>, seeds: std::ops::Range<u64>, len: usize) -> Coverage {
    let mut coverage = Coverage::default();

    for seed in seeds {
        let ops = sequence(seed, len);
        let mut fuzzer = Fuzzer::new(fee, seed);

        if let Some((step, why)) = fuzzer.run(&ops) {
            let minimal = shrink(fee, seed, &ops);
            let listing = minimal
                .iter()
                .map(|op| format!("  {op:?}"))
                .collect::<Vec<_>>()
                .join("\n");
            panic!(
                "seed {seed} (fee {fee:?}) violated an invariant at step {step} of {len}\n\
                 {why}\n\n\
                 minimal reproduction, {} of {len} operations:\n{listing}\n\n\
                 reproduce with: Fuzzer::new({fee:?}, {seed}).run(&…)",
                minimal.len(),
            );
        }

        coverage.merge(&fuzzer.coverage);
    }

    coverage
}

/// The campaign. Split across three tests so `cargo test` runs them in parallel.
#[test]
fn random_sequences_preserve_every_invariant() {
    sweep(None, 0..8, OPS);
}

#[test]
fn random_sequences_preserve_every_invariant_continued() {
    sweep(None, 8..16, OPS);
}

/// The same properties over a mint that withholds a fee on every transfer, so
/// the amount an instruction is asked to move is never the amount that arrives.
///
/// §1.1 and §1.3 are the ones at risk: a program that credits the *argument*
/// rather than the observed vault delta passes every fee-free sequence and
/// leaves the pool insolvent here.
#[test]
fn a_fee_bearing_mint_preserves_every_invariant() {
    let fee = Some(TransferFee {
        basis_points: 250,
        maximum_fee: 100_000,
    });
    sweep(fee, 100..106, OPS);
}

/// Every operation must be both accepted and rejected somewhere in the campaign.
///
/// The bounds are deliberately loose — this is a guard against a generator that
/// has drifted into only ever doing one thing, not a measurement to be tuned
/// against. Where a bound is 0, the reason is written next to it.
#[test]
fn the_fuzzer_is_not_vacuous() {
    let mut coverage = sweep(None, 0..8, OPS);
    coverage.merge(&sweep(None, 8..16, OPS));

    // Operations whose acceptance and rejection are both meaningful.
    for kind in [
        "stake",
        "unstake",
        "claim",
        "set_reward_rate",
        "propose",
        "activate",
        "vote",
        "finalize",
        "queue",
        "execute",
        "cancel",
    ] {
        assert!(
            coverage.accepted(kind) > 0,
            "no `{kind}` was ever accepted, so nothing downstream of it was tested\n{}",
            coverage.summary()
        );
        assert!(
            coverage.rejected(kind) > 0,
            "no `{kind}` was ever rejected, so the guards on it went unexercised\n{}",
            coverage.summary()
        );
    }

    // These have no failure mode the fuzzer can reach. `fund_rewards` and
    // `set_paused` are signed by the pool authority with in-range arguments;
    // `warp` and `warp_to_deadline` only move the clock. Only acceptance is
    // asserted.
    for kind in [
        "fund_rewards",
        "set_paused",
        "warp",
        "warp_to_deadline",
        "warp_to_unlock",
    ] {
        assert!(
            coverage.accepted(kind) > 0,
            "no `{kind}` was ever accepted\n{}",
            coverage.summary()
        );
    }

    // The reward machinery is only under test when emissions are actually
    // running, which needs an affordable rate *and* time passing under it.
    assert!(
        coverage.accepted("claim") >= 10,
        "only {} claims succeeded across the campaign — emissions are barely \
         running, so §1.2 and §3.x are close to vacuous\n{}",
        coverage.accepted("claim"),
        coverage.summary()
    );
}

/// Corrupt the accounts behind the oracle's back and require it to object, by
/// section.
///
/// This is the mutation test for the fuzzer itself. An oracle that always
/// returned `Ok` would make every other test in this file pass.
#[test]
fn the_oracle_notices_corrupted_state() {
    // A scenario that reaches every part of the oracle: a locked position large
    // enough to vote, a proposal, and a vote cast on it.
    let script = [
        Op::Stake {
            actor: 0,
            amount: 5_000_000,
            tier: 3, // Gold: a 180-day lock, so it out-lives the voting period
        },
        Op::FundRewards { amount: 10_000_000 },
        // Affordable: 3/second across a 30-day period is 7.8M of a 10M vault.
        Op::SetRewardRate { rate: 3 },
        Op::Warp { seconds: 3_600 },
        // The clock alone does not move `reward_per_token`; some instruction has
        // to call `update_rewards` and write it back. Without this the
        // accumulator is still 0 and the §3.1 corruption below underflows
        // instead of being caught.
        Op::Claim { position: 0 },
        Op::Propose { position: 0 },
        Op::Activate { proposal: 0 },
        Op::Vote {
            proposal: 0,
            position: 0,
            choice: 0,
        },
    ];

    // Every corruption is applied to a system built the same way, so a failure
    // is attributable to the corruption and not to the scenario.
    /// An invariant section paired with a corruption that must trip it.
    type Corruption<'a> = (&'a str, &'a dyn Fn(&mut Fuzzer));

    let planted: [Corruption; 5] = [
        ("§1.3", &|f: &mut Fuzzer| {
            let mut pool: Pool = f.sys.env.anchor_account(&f.sys.pool);
            pool.total_staked += 1;
            overwrite(f, f.sys.pool, &pool, Pool::DISCRIMINATOR);
        }),
        ("§1.4", &|f: &mut Fuzzer| {
            let mut pool: Pool = f.sys.env.anchor_account(&f.sys.pool);
            pool.total_weighted -= 1;
            overwrite(f, f.sys.pool, &pool, Pool::DISCRIMINATOR);
        }),
        ("§3.1", &|f: &mut Fuzzer| {
            let mut pool: Pool = f.sys.env.anchor_account(&f.sys.pool);
            pool.reward_per_token -= 1;
            overwrite(f, f.sys.pool, &pool, Pool::DISCRIMINATOR);
        }),
        ("§4.3", &|f: &mut Fuzzer| {
            let key = f.proposal_key(0);
            let mut proposal: Proposal = f.sys.env.anchor_account(&key);
            proposal.for_votes = proposal.total_weight_snapshot + 1;
            overwrite(f, key, &proposal, Proposal::DISCRIMINATOR);
        }),
        ("§4.6", &|f: &mut Fuzzer| {
            let key = f.proposal_key(0);
            let mut proposal: Proposal = f.sys.env.anchor_account(&key);
            // Voting -> Executed skips finalise, queue and the timelock.
            proposal.state = ProposalState::Executed;
            overwrite(f, key, &proposal, Proposal::DISCRIMINATOR);
        }),
    ];

    for (section, corrupt) in planted {
        let mut fuzzer = Fuzzer::new(None, 1);
        assert!(
            fuzzer.run(&script).is_none(),
            "the scenario itself failed, before any corruption"
        );
        // The §3.1 watermark and the §4.6 state are both remembered from the
        // previous check, so the oracle has to have run once for them to mean
        // anything. `run` above did that.
        corrupt(&mut fuzzer);

        let violation = fuzzer.check_invariants().expect_err(&format!(
            "{section} was corrupted and the oracle said nothing"
        ));
        assert!(
            violation.starts_with(section),
            "corrupting {section} was reported as: {violation}"
        );
    }
}

/// Writes `value` into `key`, discriminator included, bypassing the programs.
fn overwrite<T: AnchorSerialize>(
    fuzzer: &mut Fuzzer,
    key: anchor_lang::prelude::Pubkey,
    value: &T,
    discriminator: &[u8],
) {
    let mut account = fuzzer
        .sys
        .env
        .svm
        .get_account(&key)
        .expect("account to corrupt is missing");

    let mut data = discriminator.to_vec();
    value.serialize(&mut data).expect("serialisation failed");
    account.data = data;

    fuzzer
        .sys
        .env
        .svm
        .set_account(key, account)
        .expect("set_account failed");
}

/// The shrinker, against a predicate whose minimal failing case is known.
///
/// The predicate stands in for a real bug: "fails when a claim happens while the
/// pool is paused". A correct reduction has to keep exactly those two operations
/// and delete the other fifty-eight, in order.
#[test]
fn the_shrinker_reduces_to_the_minimal_case() {
    let ops = sequence(7, OPS);

    let pause = Op::SetPaused { paused: true };
    let claim = Op::Claim { position: 3 };
    let mut seeded = ops.clone();
    seeded.insert(10, pause);
    seeded.insert(40, claim);

    let reduced = shrink_with(&seeded, |candidate| {
        let paused_at = candidate.iter().position(|op| *op == pause);
        let claimed_at = candidate.iter().position(|op| *op == claim);
        matches!((paused_at, claimed_at), (Some(p), Some(c)) if p < c)
    });

    assert_eq!(
        reduced,
        vec![pause, claim],
        "the shrinker left {} operations where 2 reproduce the failure",
        reduced.len()
    );
}

/// The same seed twice produces the same chain state.
///
/// This is what makes a reported seed a reproduction rather than a hint. It is
/// also what the shrinker depends on: delta debugging over a non-deterministic
/// predicate reduces to noise.
#[test]
fn a_run_is_reproducible_from_its_seed() {
    let ops = sequence(31, OPS);

    let fingerprint = |seed: u64| {
        let mut fuzzer = Fuzzer::new(None, seed);
        assert!(fuzzer.run(&ops).is_none());

        let pool: Pool = fuzzer.sys.env.anchor_account(&fuzzer.sys.pool);
        (
            pool.total_staked,
            pool.total_weighted,
            pool.reward_per_token,
            pool.total_rewards_accrued,
            pool.total_rewards_paid,
            pool.position_count,
            fuzzer.sys.env.token_balance(&fuzzer.sys.stake_vault),
            fuzzer.sys.env.token_balance(&fuzzer.sys.reward_vault),
            fuzzer.coverage.summary(),
        )
    };

    assert_eq!(
        fingerprint(31),
        fingerprint(31),
        "two runs of seed 31 diverged"
    );
}
