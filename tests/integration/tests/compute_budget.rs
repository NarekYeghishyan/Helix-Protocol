//! Compute-unit benchmarks — `INVARIANTS.md` §6.3.
//!
//! §6.3 claims that no instruction's compute cost grows with the number of
//! stakers or voters. Until now that followed from reading the code: no handler
//! loops over a user-growable set, so nothing *should* scale. That is a sound
//! argument and not a measurement, and compute has non-obvious contributors —
//! account deserialisation, CPI depth, and PDA derivation especially.
//!
//! These tests measure it, and produce the compute table in
//! [`docs/TESTING.md`](../../../docs/TESTING.md). Run with `--nocapture` to print it.
//!
//! # Three confounds, all controlled
//!
//! Each is larger than the effect being measured, so the controls are not
//! fastidiousness — without them this file measures nothing.
//!
//! **PDA bump search.** Anchor derives a bump on chain whenever the constraint is
//! a bare `bump` rather than `bump = <stored>`. That compiles to
//! `find_program_address`, which tries 255, then 254, and so on until the
//! candidate is off-curve, paying a syscall per attempt. Which bump a seed set
//! lands on is effectively random, so two stakers can differ by thousands of
//! units for reasons unrelated to pool size — see
//! `pda_bump_search_costs_more_than_pool_size_ever_does`, which measures it at
//! exactly 1500 CU per attempt. Probe keypairs here are ground onto bump 255.
//!
//! **Every PDA descends from the mint**, so `System::bootstrap`'s random mint
//! moves the whole table by multiples of 1500 between runs. An early draft
//! compared two `System`s built on different mints, read that noise as signal,
//! and "confirmed" a mechanism that was not there — it flipped sign on the next
//! run. These tests use [`fixed_mint`].
//!
//! **The accumulator's first move off zero.** The reward maths runs in `u128`,
//! which SBF has no native instruction for; LLVM emits software routines whose
//! cost tracks operand bit-length. Zero is the cheapest operand there is, so the
//! instruction that first lifts `reward_per_token` off zero is cheaper than every
//! one after it. That is a one-off state transition, not a staker count, so
//! [`warm_pool`] gets it out of the way before the first checkpoint.
//!
//! What survives all three is a 31-CU quantum of runtime noise
//! ([`COMPUTE_NOISE_FLOOR_CU`]), three orders of magnitude below any growth worth
//! finding. A tolerance wide enough to absorb the confounds *uncontrolled* would
//! have been wide enough to hide exactly what this file exists to look for.

use anchor_lang::prelude::Pubkey;
use helix_governance::state::{ProposalAction, VoteChoice};
use helix_integration_tests::bootstrap::HOUR;
use helix_integration_tests::{pda, System};
use helix_staking::state::LockTier;
use solana_keypair::Keypair;
use solana_signer::Signer as _;

/// Counts at which every measurement is repeated. Spanning 64x makes linear
/// growth impossible to miss: an O(n) handler costs 64 times more count-dependent
/// work at the last checkpoint than the first.
const CHECKPOINTS: [u64; 4] = [1, 4, 16, 64];

/// The runtime's default compute limit for a single instruction. A transaction
/// may raise it to 1.4M, but an instruction needing a raised limit is one every
/// wallet and integrator has to know about.
const DEFAULT_INSTRUCTION_BUDGET: u64 = 200_000;

/// The canonical — first tried, therefore cheapest — PDA bump.
const CANONICAL_BUMP: u8 = 255;

const STAKE_AMOUNT: u64 = 1_000_000_000;
const REWARD_FUNDING: u64 = 200_000_000_000;
const REWARD_RATE: u64 = 1_000_000;
const REWARD_PERIOD: i64 = 100_000;
/// Long enough that a probe's share of emissions survives truncation to a
/// non-zero figure even with 64 stakers diluting it. `claim` rejects a zero
/// balance, and a rejected instruction measures the constraint block rather than
/// the handler.
const ACCRUAL: i64 = HOUR;

/// A fixed mint, so every `System` these tests build derives identical PDAs with
/// identical bumps.
///
/// `System::bootstrap` generates a random mint, and every PDA in the protocol
/// descends from it. Anchor derives several of those bumps on chain, at 1500 CU
/// per attempt, so a random mint moves the figures below by multiples of 1500
/// between runs — enough to swamp everything being measured, and enough to make
/// the published table an anecdote. The value itself is arbitrary; only its
/// constancy matters.
fn fixed_mint() -> Keypair {
    Keypair::new_from_array([7u8; 32])
}

/// Generates keypairs until one satisfies `wanted`.
///
/// Used to pin PDA bumps. The bound is generous: the predicates here hold with
/// probability ~1/2 or ~1/4 per attempt, so exhausting it means the predicate is
/// unsatisfiable rather than that the search was unlucky.
fn grind(wanted: impl Fn(&Pubkey) -> bool) -> Keypair {
    for _ in 0..100_000 {
        let candidate = Keypair::new();
        if wanted(&candidate.pubkey()) {
            return candidate;
        }
    }
    panic!("no keypair satisfied the predicate in 100,000 attempts");
}

/// Builds a pool holding `stakers` positions of `each`, with rewards flowing and
/// the accumulator already off zero.
fn warm_pool(stakers: u64, each: u64) -> System {
    let mut sys = System::bootstrap_with_mint(fixed_mint(), None, 0);
    sys.fund_rewards(REWARD_FUNDING);
    let period_end = sys.env.now() + REWARD_PERIOD;
    sys.set_reward_rate(REWARD_RATE, period_end);

    for _ in 0..stakers {
        sys.add_staker(Keypair::new(), each, LockTier::Bronze);
    }

    // Let time pass, then force an accumulator update. Without this, the first
    // measured instruction pays the zero-to-non-zero transition described in the
    // module docs, and the cost gets charged to pool size.
    sys.env.warp_forward(ACCRUAL);
    sys.set_reward_rate(REWARD_RATE, period_end);

    sys
}

/// Every measurement taken against one probe position.
#[derive(Debug)]
struct Probe {
    stake: u64,
    claim: u64,
    unstake: u64,
}

/// Opens a position, claims its rewards and closes it, returning what each step
/// cost. The owner key is ground so the position PDA lands on the canonical bump.
fn measure_probe(sys: &mut System) -> Probe {
    let pool = sys.pool;
    let position_id = sys.next_position_id();
    let owner = grind(|pk| pda::position(&pool, pk, position_id).1 == CANONICAL_BUMP);
    let (position, _) = pda::position(&pool, &owner.pubkey(), position_id);
    let (tokens, _) = sys.prepare_staker(&owner.pubkey(), STAKE_AMOUNT);

    // Flexible carries no lock, so the probe can be unstaked in the same pass
    // rather than needing a clock warp that would also change what has accrued.
    let ix = sys.stake_ix_for(
        &owner.pubkey(),
        &tokens,
        position_id,
        STAKE_AMOUNT,
        LockTier::Flexible,
    );
    let stake = sys.env.compute_units(&[ix], &[&owner]);

    sys.env.warp_forward(ACCRUAL);

    let ix = sys.claim_ix_for(&owner.pubkey(), &tokens, position);
    let claim = sys.env.compute_units(&[ix], &[&owner]);

    let ix = sys.unstake_ix_for(&owner.pubkey(), &tokens, position, STAKE_AMOUNT);
    let unstake = sys.env.compute_units(&[ix], &[&owner]);

    Probe {
        stake,
        claim,
        unstake,
    }
}

/// The runtime's own measurement noise, and the floor below which nothing here
/// can claim to have measured anything.
///
/// A measurement occasionally lands 31 CU high — always exactly 31, never more,
/// and never low. It is not the code under test: twenty stakes by twenty
/// different owners inside one pool measured 24,261 CU each to the unit, and
/// every piece of protocol state was checked identical across a pair that
/// differed (`total_weighted`, `reward_per_token` before and after,
/// `last_update_ts`, the settled amount). It appears on the first measurement
/// after a clock warp and on freshly built `System`s. Beyond that it is
/// unattributed.
///
/// Set to two quanta. It is 0.2% of the smallest instruction measured here, while
/// the effect it must not hide — cost proportional to staker count — would be
/// thousands of units across a 64x sweep. Nothing in this file rests on
/// distinguishing 31 CU from zero.
const COMPUTE_NOISE_FLOOR_CU: u64 = 64;

/// Asserts a series shows no growth beyond `budget`, reporting the whole series
/// on failure — the shape of the growth is what identifies its cause.
fn assert_no_growth(label: &str, counts: &[u64], measured: &[u64], budget: u64) {
    let min = *measured.iter().min().expect("no measurements");
    let max = *measured.iter().max().expect("no measurements");
    let span = counts.last().unwrap() / counts.first().unwrap();
    let series: Vec<String> = counts
        .iter()
        .zip(measured)
        .map(|(n, cu)| format!("n={n}: {cu} CU"))
        .collect();

    println!(
        "  {label}: {} CU drift ({:.2}%) across a {span}x sweep{}",
        max - min,
        (max - min) as f64 * 100.0 / min as f64,
        if max == min { ", bit-identical" } else { "" },
    );

    assert!(
        max - min <= budget,
        "{label} drifted {} CU across the sweep, over its {budget} CU budget — {}. \
         Either a handler now touches something that grows with the set, or one of \
         the confounds documented at the top of this file has come back.",
        max - min,
        series.join(", ")
    );
}

#[test]
fn staking_compute_does_not_grow_with_staker_count() {
    let mut sys = warm_pool(1, STAKE_AMOUNT);

    let mut stake = Vec::new();
    let mut claim = Vec::new();
    let mut unstake = Vec::new();

    for n in CHECKPOINTS {
        while sys.next_position_id() < n {
            sys.add_staker(Keypair::new(), STAKE_AMOUNT, LockTier::Bronze);
        }

        let probe = measure_probe(&mut sys);
        stake.push(probe.stake);
        claim.push(probe.claim);
        unstake.push(probe.unstake);
    }

    println!("\nstakers   stake    claim  unstake");
    for (i, n) in CHECKPOINTS.iter().enumerate() {
        println!(
            "{:>7}  {:>6}  {:>7}  {:>7}",
            n, stake[i], claim[i], unstake[i]
        );
    }

    // `stake` and `unstake` show nothing beyond the runtime's own noise, usually
    // measuring bit-identical at all four counts.
    assert_no_growth("stake", &CHECKPOINTS, &stake, COMPUTE_NOISE_FLOOR_CU);
    assert_no_growth("unstake", &CHECKPOINTS, &unstake, COMPUTE_NOISE_FLOOR_CU);

    // `claim` genuinely moves, by roughly 244 CU. It divides by `total_weighted`,
    // which grows with the staker set, and SBF has no native 128-bit arithmetic —
    // LLVM emits software routines whose cost tracks operand bit-length. Sixty-four
    // times the stakers is six more bits.
    //
    // Logarithmic in staked *value*, not linear in staker *count*, which is the
    // distinction §6.3 exists to draw.
    // `claim_compute_tracks_staked_value_not_staker_count` separates the two and
    // shows the whole effect survives with the staker count pinned at one.
    assert_no_growth("claim", &CHECKPOINTS, &claim, SETTLEMENT_DRIFT_BUDGET_CU);
}

/// Drift budget for `claim`, which settles a position against the accumulator and
/// so pays for the bit-length of what it settles.
///
/// Sized to catch what §6.3 rules out while tolerating what it does not. A
/// per-staker cost of even 25 CU — an order of magnitude below a single account
/// read — would blow this budget at the last checkpoint. The bit-length effect
/// being tolerated is 244 CU, and it does not grow with the count.
const SETTLEMENT_DRIFT_BUDGET_CU: u64 = 512;

/// Separates the two things the sweep above confounds: adding stakers also grows
/// `total_weighted`, so the sweep cannot say which of the two `claim` responds to.
///
/// This reaches the *same* `total_weighted` two different ways — sixty-four
/// stakers holding one unit each, or one staker holding sixty-four — and compares
/// them. Staker count differs by 64x between the two; total staked weight is
/// identical. If cost tracked the count they would differ; if it tracks the
/// numbers they must match.
///
/// The only comparison in this file that spans two separately built `System`s.
#[test]
fn claim_compute_tracks_staked_value_not_staker_count() {
    let baseline = claim_cost(1, STAKE_AMOUNT);
    let by_count = claim_cost(64, STAKE_AMOUNT);
    let by_size = claim_cost(1, STAKE_AMOUNT * 64);

    // 64x the stakers at the same total value.
    let count_effect = by_count.abs_diff(by_size);
    // 64x the value at the same staker count.
    let value_effect = baseline.abs_diff(by_size);

    println!(
        "\nclaim, at equal total weight reached two ways\n  \
         1 staker  x1  (baseline): {baseline} CU\n  \
         64 stakers x1           : {by_count} CU\n  \
         1 staker  x64           : {by_size} CU\n  \
         effect of 64x the stakers: {count_effect} CU\n  \
         effect of 64x the value  : {value_effect} CU"
    );

    assert!(
        count_effect <= COMPUTE_NOISE_FLOOR_CU,
        "sixty-four stakers cost {count_effect} CU more than one staker holding the \
         same total, past the {COMPUTE_NOISE_FLOOR_CU} CU noise floor. Staker count \
         is therefore a variable in its own right and §6.3 does not hold"
    );

    // The comparison above only means something if the measurement can detect a
    // difference at all. Holding the count at one while growing the value moves
    // compute several times past the noise floor — so the instrument works, and
    // what it responds to is the accumulator's u128 arithmetic rather than the
    // staker set.
    assert!(
        value_effect > 3 * COMPUTE_NOISE_FLOOR_CU,
        "a 64x change in staked value moved claim by only {value_effect} CU, close \
         enough to the {COMPUTE_NOISE_FLOOR_CU} CU noise floor that this comparison \
         cannot distinguish a real null result from an insensitive measurement"
    );
}

/// Measures one probe's claim against a pool of `stakers` positions of `each`.
fn claim_cost(stakers: u64, each: u64) -> u64 {
    measure_probe(&mut warm_pool(stakers, each)).claim
}

#[test]
fn governance_compute_does_not_grow_with_voter_count() {
    let mut sys = System::bootstrap_with_mint(fixed_mint(), None, 0);

    // The proposer needs weight, and a lock outlasting the voting window.
    sys.fund_voter(STAKE_AMOUNT);
    let proposer_position = sys.stake(0, STAKE_AMOUNT, LockTier::Bronze);
    let proposal = sys.create_proposal(0, ProposalAction::Signal, proposer_position);
    sys.activate(proposal);

    let mut measured = Vec::new();
    let mut votes_cast = 0u64;

    for k in CHECKPOINTS {
        while votes_cast < k - 1 {
            cast_one_vote(&mut sys, proposal);
            votes_cast += 1;
        }
        measured.push(cast_one_vote(&mut sys, proposal));
        votes_cast += 1;
    }

    println!("\nprior votes on the proposal   cast_vote");
    for (i, k) in CHECKPOINTS.iter().enumerate() {
        println!("{:>27}  {:>9}", k - 1, measured[i]);
    }

    // Tallies are accumulated on the proposal account, never recounted from the
    // vote records, so the sixty-fourth vote costs what the first did. Unlike the
    // staking path there is no u128 arithmetic here at all, and this measures
    // bit-identical.
    assert_no_growth("cast_vote", &CHECKPOINTS, &measured, COMPUTE_NOISE_FLOOR_CU);
}

/// Adds a fresh voter, stakes, and votes — returning what the vote cost.
///
/// The key is ground so the *vote record* lands on the canonical bump.
/// `vote_record` is an `init` account seeded on the position, so its derivation
/// cost varies per voter for reasons unrelated to how many votes precede it.
fn cast_one_vote(sys: &mut System, proposal: Pubkey) -> u64 {
    let pool = sys.pool;
    let position_id = sys.next_position_id();
    let owner = grind(|pk| {
        let (position, _) = pda::position(&pool, pk, position_id);
        pda::vote_record(&proposal, &position).1 == CANONICAL_BUMP
    });

    // Bronze locks for 30 days, well past the one-hour voting window, so the
    // flash-loan gate lets the position vote.
    let staker = sys.add_staker(owner, STAKE_AMOUNT, LockTier::Bronze);

    let ix = sys.vote_ix_for(
        &staker.owner.pubkey(),
        proposal,
        staker.position,
        VoteChoice::For,
    );
    sys.env.compute_units(&[ix], &[&staker.owner])
}

/// Measures the confound the benchmarks above control for, and shows it is larger
/// than the effect they measure.
#[test]
fn pda_bump_search_costs_more_than_pool_size_ever_does() {
    let mut sys = warm_pool(1, STAKE_AMOUNT);

    let canonical = stake_at_bump(&mut sys, CANONICAL_BUMP);

    // Four attempts down from 255. Each failed attempt is one more syscall, so
    // the gap should be four times the per-attempt cost.
    let off_bump = CANONICAL_BUMP - 4;
    let expensive = stake_at_bump(&mut sys, off_bump);

    let attempts = u64::from(CANONICAL_BUMP - off_bump);
    let per_attempt = (expensive - canonical) / attempts;

    println!(
        "\nstake at bump 255: {canonical} CU\n\
         stake at bump {off_bump}: {expensive} CU\n\
         {attempts} extra derivation attempts, {per_attempt} CU each"
    );

    assert!(
        expensive > canonical,
        "a lower bump should cost more, not less: {expensive} vs {canonical}"
    );

    // The runtime charges a flat fee per attempt. Asserting a band rather than
    // the exact figure survives a toolchain that reprices the syscall, while
    // still failing if the cost stops being flat and per-attempt.
    assert!(
        (1_000..=2_000).contains(&per_attempt),
        "expected a flat per-attempt derivation cost near 1500 CU, measured {per_attempt}"
    );

    // The point of the test: this noise dwarfs the effect §6.3 is about, which is
    // why the benchmarks above pin the bump instead of allowing a tolerance.
    assert!(
        expensive - canonical > SETTLEMENT_DRIFT_BUDGET_CU,
        "bump search ({} CU) came out smaller than the drift budget ({SETTLEMENT_DRIFT_BUDGET_CU} CU) \
         it is supposed to justify controlling for",
        expensive - canonical
    );
}

/// Opens one position whose PDA lands on `bump`, returning the stake cost.
fn stake_at_bump(sys: &mut System, bump: u8) -> u64 {
    let pool = sys.pool;
    let position_id = sys.next_position_id();
    let owner = grind(|pk| pda::position(&pool, pk, position_id).1 == bump);
    let (tokens, _) = sys.prepare_staker(&owner.pubkey(), STAKE_AMOUNT);

    let ix = sys.stake_ix_for(
        &owner.pubkey(),
        &tokens,
        position_id,
        STAKE_AMOUNT,
        LockTier::Flexible,
    );
    sys.env.compute_units(&[ix], &[&owner])
}

#[test]
fn every_instruction_fits_the_default_compute_budget() {
    let mut sys = System::bootstrap_with_mint(fixed_mint(), None, 0);
    let mut rows: Vec<(&str, u64)> = Vec::new();

    // ------------------------------------------------------------- staking
    let ix = sys.fund_rewards_ix(REWARD_FUNDING);
    rows.push(("staking::fund_rewards", sys.env.compute_units(&[ix], &[])));

    let period_end = sys.env.now() + REWARD_PERIOD;
    let ix = sys.set_reward_rate_ix(REWARD_RATE, period_end);
    rows.push((
        "staking::set_reward_rate",
        sys.env.compute_units(&[ix], &[]),
    ));

    // One actor drives the whole table, ground so that both accounts Anchor
    // derives on chain for them — the position and their vote record — land on
    // the canonical bump. Without that the figures below move by up to 7,500 CU
    // between runs, and a benchmark nobody can reproduce is an anecdote.
    let proposal_id = 0;
    let (proposal, _) = pda::proposal(&sys.realm, proposal_id);
    let pool = sys.pool;
    let position_id = sys.next_position_id();
    let actor = grind(|pk| {
        let (position, bump) = pda::position(&pool, pk, position_id);
        bump == CANONICAL_BUMP && pda::vote_record(&proposal, &position).1 == CANONICAL_BUMP
    });
    let (position, _) = pda::position(&pool, &actor.pubkey(), position_id);
    let (tokens, _) = sys.prepare_staker(&actor.pubkey(), STAKE_AMOUNT);

    let ix = sys.stake_ix_for(
        &actor.pubkey(),
        &tokens,
        position_id,
        STAKE_AMOUNT,
        LockTier::Bronze,
    );
    rows.push(("staking::stake", sys.env.compute_units(&[ix], &[&actor])));

    sys.env.warp_forward(ACCRUAL);
    let ix = sys.claim_ix_for(&actor.pubkey(), &tokens, position);
    rows.push(("staking::claim", sys.env.compute_units(&[ix], &[&actor])));

    // ---------------------------------------------------------- governance
    let destination = sys.new_token_account(&sys.env.payer_pubkey());
    sys.fund_treasury(STAKE_AMOUNT);

    let ix = sys.create_proposal_ix_for(
        &actor.pubkey(),
        proposal_id,
        ProposalAction::TreasuryTransfer {
            destination,
            amount: STAKE_AMOUNT / 2,
        },
        position,
    );
    rows.push((
        "governance::create_proposal",
        sys.env.compute_units(&[ix], &[&actor]),
    ));

    let ix = sys.activate_ix(proposal);
    rows.push((
        "governance::activate_proposal",
        sys.env.compute_units(&[ix], &[]),
    ));

    let ix = sys.vote_ix_for(&actor.pubkey(), proposal, position, VoteChoice::For);
    rows.push((
        "governance::cast_vote",
        sys.env.compute_units(&[ix], &[&actor]),
    ));

    sys.env.warp_forward(HOUR + 1);
    let ix = sys.advance_ix(proposal, true);
    rows.push((
        "governance::finalize_proposal",
        sys.env.compute_units(&[ix], &[]),
    ));

    let ix = sys.advance_ix(proposal, false);
    rows.push((
        "governance::queue_proposal",
        sys.env.compute_units(&[ix], &[]),
    ));

    // The deepest call stack in the system: governance verifies the proposal,
    // signs as the executor PDA, CPIs into the treasury, which CPIs into
    // Token-2022. If anything is near a limit, it is this.
    sys.env.warp_forward(HOUR + 1);
    let ix = sys.execute_treasury_transfer_ix(proposal, destination);
    rows.push((
        "governance::execute_treasury_transfer",
        sys.env.compute_units(&[ix], &[]),
    ));

    // ------------------------------------------------------------ treasury
    let ix = sys.fund_treasury_ix(STAKE_AMOUNT);
    rows.push(("treasury::deposit", sys.env.compute_units(&[ix], &[])));

    // -------------------------------------------------------------- report
    println!("\n| Instruction | CU | % of the 200k default |");
    println!("|---|---:|---:|");
    for (name, cu) in &rows {
        let pct = (*cu as f64) * 100.0 / (DEFAULT_INSTRUCTION_BUDGET as f64);
        println!("| `{name}` | {cu} | {pct:.1}% |");
    }

    let (worst_name, worst_cu) = *rows.iter().max_by_key(|(_, cu)| *cu).expect("no rows");
    println!(
        "\nworst: {worst_name} at {worst_cu} CU, {:.1}% of the default budget\n",
        (worst_cu as f64) * 100.0 / (DEFAULT_INSTRUCTION_BUDGET as f64)
    );

    for (name, cu) in &rows {
        assert!(
            *cu < DEFAULT_INSTRUCTION_BUDGET,
            "{name} needs {cu} CU, over the {DEFAULT_INSTRUCTION_BUDGET} default — \
             every caller would have to prepend a ComputeBudget instruction"
        );
    }

    // The claim worth making is not "it fits" but "it fits with room to spare".
    // A program sitting just inside the budget breaks on the next Anchor or
    // Token-2022 release that costs a few thousand units more.
    let quarter = DEFAULT_INSTRUCTION_BUDGET / 4;
    assert!(
        worst_cu < quarter,
        "{worst_name} at {worst_cu} CU leaves under 4x headroom against the \
         {DEFAULT_INSTRUCTION_BUDGET} default"
    );
}
