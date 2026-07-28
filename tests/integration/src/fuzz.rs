//! A stateful fuzzer for staking and governance, with [`INVARIANTS.md`] as the
//! oracle.
//!
//! [`INVARIANTS.md`]: ../../../docs/INVARIANTS.md
//!
//! Random operation sequences run against the real BPF programs, and after
//! **every** operation the aggregate invariants are read back out of the
//! accounts. Those are the invariants unit tests cannot reach —
//! `Σ position.amount == vault.amount` needs real positions and a real vault —
//! and they are exactly where a rounding or ordering mistake hides.
//!
//! # Why not Trident
//!
//! Trident is the obvious choice and does not fit this workspace yet: its
//! generated harness is built against the `solana-sdk` 2.x / Anchor 0.31 line,
//! while `anchor-lang` 1.1.2 resolves the Solana crates at 3.x. Adding it puts
//! two major versions of the SDK in one dependency graph — the breakage that
//! already forces `litesvm` to be pinned at `=0.13.1`. Rechecking is a one-line
//! change in `Cargo.toml`; until it moves, this is the equivalent built on the
//! runtime the suite already uses.
//!
//! # What makes this more than a loop of random calls
//!
//! **Deterministic.** The mint and every actor key are derived from the seed, so
//! the state a sequence produces is a pure function of `(seed, ops)` and a
//! failure is reproducible from the seed rather than from a captured trace.
//!
//! **Shrinking.** A failing sequence is reduced by deleting operations that are
//! not needed to reproduce it, so the report is the short case.
//!
//! **Negative expectations.** The oracle is not only "nothing blew up". The
//! fuzzer knows which positions have already voted and which proposals have
//! already executed, so a call that *succeeds* when it had to fail is a
//! violation — that is §4.1 and §4.5 checked in the direction that matters.
//!
//! **Anti-vacuity.** Operations are *expected* to be rejected sometimes:
//! unstaking a locked position must fail. A fuzzer where everything fails proves
//! nothing while looking busy, so per-operation acceptance is counted and
//! asserted, and every rejection must name an error from a known list. An
//! unrecognised error is itself a finding.

use std::collections::{BTreeMap, BTreeSet};

use anchor_lang::prelude::Pubkey;
use helix_governance::instructions::realm::RealmParams;
use helix_governance::state::{Proposal, ProposalAction, ProposalState, Realm, VoteChoice};
use helix_staking::state::{LockTier, Pool, Position};
use solana_keypair::Keypair;
use solana_signer::Signer as _;

use crate::{pda, System, TransferFee};

/// SplitMix64. Ten lines, no dependency, and identical output on every platform
/// — which is what "reproducible from the seed" requires.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n.max(1)
    }

    fn range(&mut self, low: u64, high: u64) -> u64 {
        low + self.below(high.saturating_sub(low).max(1))
    }

    /// A keypair drawn from the stream, so actors are seed-derived rather than
    /// random. See the determinism note in the module header.
    fn keypair(&mut self) -> Keypair {
        let mut bytes = [0u8; 32];
        for chunk in bytes.chunks_mut(8) {
            chunk.copy_from_slice(&self.next_u64().to_le_bytes());
        }
        Keypair::new_from_array(bytes)
    }
}

/// One step of a generated sequence.
///
/// Indices are stored raw and taken modulo the live set at execution time, so a
/// pre-generated sequence stays valid however the shrinker rearranges it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Stake {
        actor: usize,
        amount: u64,
        tier: u8,
    },
    Unstake {
        position: usize,
        bps: u16,
    },
    Claim {
        position: usize,
    },
    FundRewards {
        amount: u64,
    },
    SetRewardRate {
        rate: u64,
    },
    SetPaused {
        paused: bool,
    },
    Propose {
        position: usize,
    },
    Activate {
        proposal: usize,
    },
    Vote {
        proposal: usize,
        position: usize,
        choice: u8,
    },
    Finalize {
        proposal: usize,
    },
    Queue {
        proposal: usize,
    },
    Execute {
        proposal: usize,
    },
    Cancel {
        proposal: usize,
    },
    Warp {
        seconds: i64,
    },

    // The two operations below read chain state to decide how far to move the
    // clock, and they are why the timed paths are reachable at all.
    //
    // Governance runs on hours-to-days and stake locks run on months. A single
    // random warp distribution cannot serve both: narrow enough to sit inside a
    // voting window, it never expires a 180-day lock; wide enough to expire one,
    // it steps clean over every window it passes. The first measured campaign
    // did exactly that — 18 proposals activated, 7 votes cast, none executed.
    //
    // Fuzzing a timed state machine means being able to land *on* its
    // boundaries, not merely near them.
    /// Jump to just past whatever deadline `proposal` is waiting on — the close
    /// of voting, or the end of its timelock.
    WarpToDeadline {
        proposal: usize,
    },
    /// Jump to just past `position`'s `lock_end`.
    WarpToUnlock {
        position: usize,
    },
}

impl Op {
    /// The bucket this operation is counted under in [`Coverage`].
    pub fn kind(&self) -> &'static str {
        match self {
            Op::Stake { .. } => "stake",
            Op::Unstake { .. } => "unstake",
            Op::Claim { .. } => "claim",
            Op::FundRewards { .. } => "fund_rewards",
            Op::SetRewardRate { .. } => "set_reward_rate",
            Op::SetPaused { .. } => "set_paused",
            Op::Propose { .. } => "propose",
            Op::Activate { .. } => "activate",
            Op::Vote { .. } => "vote",
            Op::Finalize { .. } => "finalize",
            Op::Queue { .. } => "queue",
            Op::Execute { .. } => "execute",
            Op::Cancel { .. } => "cancel",
            Op::Warp { .. } => "warp",
            Op::WarpToDeadline { .. } => "warp_to_deadline",
            Op::WarpToUnlock { .. } => "warp_to_unlock",
        }
    }

    /// The operation mix.
    ///
    /// The weights are not arbitrary and they are not free parameters:
    /// `the_fuzzer_is_not_vacuous` fails if any operation stops being both
    /// accepted and rejected, so a mix that drifts into only ever creating
    /// proposals nobody votes on is a red test, not a quiet loss of coverage.
    /// Every number here was set by reading that test's output.
    fn generate(rng: &mut Rng) -> Self {
        match rng.below(100) {
            0..=11 => Op::Stake {
                actor: rng.below(64) as usize,
                // MIN_STAKE_AMOUNT gets its own mass rather than the sliver of
                // probability a uniform range over 500..20,000,000 would give
                // it. Boundaries are where the guards are.
                amount: match rng.below(8) {
                    0 => rng.range(1, 1_000), // below the minimum
                    1 => 1_000,               // exactly the minimum
                    _ => rng.range(1_000, 20_000_000),
                },
                // Skewed off Flexible. A flexible position unlocks immediately,
                // so it can never satisfy `lock_end >= voting_ends_at` and can
                // never vote — a uniform quarter of every stake being ineligible
                // was the second largest sink in the vote funnel. Kept at an
                // eighth rather than removed: it is the tier that makes `unstake`
                // reachable without waiting out a lock.
                tier: match rng.below(8) {
                    0 => 0,                 // Flexible
                    n => (n % 3 + 1) as u8, // Bronze, Silver, Gold
                },
            },
            12..=19 => Op::Unstake {
                position: rng.below(64) as usize,
                bps: match rng.below(4) {
                    0 => 10_000,                           // exactly everything
                    1 => rng.range(10_001, 12_000) as u16, // more: must be refused
                    _ => rng.range(1, 10_000) as u16,
                },
            },
            20..=27 => Op::Claim {
                position: rng.below(64) as usize,
            },
            28..=30 => Op::FundRewards {
                amount: rng.range(1_000, 500_000_000),
            },
            31..=34 => Op::SetRewardRate {
                // Tuned against the funding above so a rate is sometimes
                // affordable and sometimes not. `set_reward_rate` has to cover
                // `rate × (end − now)` out of the vault up front, so this range
                // and `REWARD_PERIOD` are one decision, not two.
                rate: rng.below(400),
            },
            35..=36 => Op::SetPaused {
                paused: rng.below(2) == 0,
            },
            37..=42 => Op::Propose {
                position: rng.below(64) as usize,
            },
            43..=50 => Op::Activate {
                proposal: rng.below(16) as usize,
            },
            51..=68 => Op::Vote {
                proposal: rng.below(16) as usize,
                position: rng.below(64) as usize,
                // Skewed toward `For`. Under a 50.01% approval threshold a
                // uniform three-way split defeats nearly everything, and a
                // proposal that never succeeds never reaches Queued or Executed
                // — so the second half of the lifecycle would go untested.
                choice: match rng.below(4) {
                    0 | 1 => 0, // For
                    2 => 1,     // Against
                    _ => 2,     // Abstain
                },
            },
            69..=74 => Op::Finalize {
                proposal: rng.below(16) as usize,
            },
            75..=79 => Op::Queue {
                proposal: rng.below(16) as usize,
            },
            80..=84 => Op::Execute {
                proposal: rng.below(16) as usize,
            },
            // Deliberately rare. A guardian veto is terminal, so at any higher
            // weight it retires proposals faster than the lifecycle can walk
            // them to `Executed`.
            85 => Op::Cancel {
                proposal: rng.below(16) as usize,
            },
            86..=92 => Op::WarpToDeadline {
                proposal: rng.below(16) as usize,
            },
            93..=95 => Op::WarpToUnlock {
                position: rng.below(64) as usize,
            },
            // Log-uniform rather than uniform, and capped at three days: the two
            // targeted operations above cover the long boundaries, so this only
            // has to supply ordinary drift between them.
            _ => Op::Warp {
                seconds: match rng.below(4) {
                    0 => rng.range(1, 60),
                    1 => rng.range(60, 3_600),
                    2 => rng.range(3_600, 86_400),
                    _ => rng.range(86_400, 3 * 86_400),
                } as i64,
            },
        }
    }
}

/// Rejections that are correct behaviour rather than defects.
///
/// What is *absent* carries the meaning. `MathOverflow` reaching a user is a
/// finding even though the instruction failed safely: a checked operation
/// saturated on inputs someone could supply. So are the mismatch errors
/// (`PositionPoolMismatch`, `PoolMismatch`, `ActionAccountMismatch`) and the
/// authority errors (`NotAuthority`, `NotGuardian`, `NotPositionOwner`) — this
/// fuzzer always signs with the right key and always passes matching accounts,
/// so seeing one would mean the program disagrees about who is who.
const EXPECTED_REJECTIONS: &[&str] = &[
    // ------------------------------------------------------------- staking
    "DepositsPaused",
    "BelowMinimumStake",
    "ZeroAmount",
    "PositionLocked",
    "InsufficientStake",
    "RewardRateTooHigh",
    "InvalidRewardPeriod",
    "InsufficientRewardFunding",
    "NothingToClaim",
    "ZeroAfterFees",
    // ---------------------------------------------------------- governance
    "InvalidProposalState",
    "VotingNotStarted",
    "VotingEnded",
    "VotingStillOpen",
    "InsufficientLockDuration",
    "PositionNotInSnapshot",
    "ZeroWeight",
    "BelowProposalThreshold",
    "TimelockNotElapsed",
    "ProposalExpired",
    "MissingSnapshot",
    // A second vote from the same position collides with the `init` on its
    // VoteRecord PDA, which is §4.1 enforced by construction. `Op::Vote` also
    // asserts the rejection is *required*, so this entry only keeps the run
    // going — it is not what proves the invariant.
    "already in use",
    // ---------------------------------------------------------------- token
    // An actor that has spent its balance.
    "insufficient funds",
];

/// What each operation did, so a run that looked busy and did nothing can be
/// told apart from one that exercised the protocol.
///
/// Rejections are counted *by reason*, not just totalled. "vote: 6 accepted, 107
/// rejected" says the operation is nearly vacuous but not why; "107 ×
/// InvalidProposalState" says the votes are landing on proposals that are not
/// open, and "107 × InsufficientLockDuration" says something else entirely. One
/// of those is fixed by generating more proposals and the other by staking
/// longer locks, and the number alone does not distinguish them.
#[derive(Default, Debug)]
pub struct Coverage(BTreeMap<&'static str, Outcomes>);

#[derive(Default, Debug)]
struct Outcomes {
    accepted: usize,
    /// Rejection reason -> count. The reason is the [`EXPECTED_REJECTIONS`]
    /// entry that matched, so it is a stable key rather than a log line.
    rejected: BTreeMap<&'static str, usize>,
}

impl Coverage {
    fn record(&mut self, op: &Op, reason: Option<&'static str>) {
        let entry = self.0.entry(op.kind()).or_default();
        match reason {
            None => entry.accepted += 1,
            Some(reason) => *entry.rejected.entry(reason).or_default() += 1,
        }
    }

    pub fn accepted(&self, kind: &str) -> usize {
        self.0.get(kind).map_or(0, |o| o.accepted)
    }

    pub fn rejected(&self, kind: &str) -> usize {
        self.0
            .get(kind)
            .map_or(0, |o| o.rejected.values().sum::<usize>())
    }

    pub fn merge(&mut self, other: &Coverage) {
        for (kind, outcomes) in &other.0 {
            let entry = self.0.entry(kind).or_default();
            entry.accepted += outcomes.accepted;
            for (reason, count) in &outcomes.rejected {
                *entry.rejected.entry(reason).or_default() += count;
            }
        }
    }

    /// One line per operation: how many were accepted, and what turned the rest
    /// away, commonest first.
    pub fn summary(&self) -> String {
        self.0
            .iter()
            .map(|(kind, outcomes)| {
                let mut reasons: Vec<_> = outcomes.rejected.iter().collect();
                reasons.sort_by_key(|(_, count)| std::cmp::Reverse(**count));
                let reasons = reasons
                    .iter()
                    .map(|(reason, count)| format!("{count}×{reason}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("  {kind:<17} {:>4} ok   {reasons}", outcomes.accepted)
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

struct Actor {
    owner: Keypair,
    tokens: Pubkey,
}

struct PositionRef {
    actor: usize,
    key: Pubkey,
}

pub struct Fuzzer {
    pub sys: System,
    actors: Vec<Actor>,
    positions: Vec<PositionRef>,
    proposals: Vec<Pubkey>,

    /// `(proposal index, position index)` pairs that have already voted. A vote
    /// they repeat must be rejected — §4.1.
    voted: BTreeSet<(usize, usize)>,
    /// Proposals already executed. A second execution must be rejected — §4.5.
    executed: BTreeSet<usize>,
    /// Each proposal's state as of the previous check, for the §4.6 edge test.
    seen_states: Vec<ProposalState>,
    /// Highest `reward_per_token` seen — §3.1.
    watermark: u128,

    pub coverage: Coverage,
}

const ACTORS: usize = 4;
const ACTOR_FUNDING: u64 = 500_000_000;
/// Short enough that a rate in the generated range is sometimes affordable.
/// `set_reward_rate` must cover `rate × (end − now)` out of the vault up front,
/// so a long period makes every non-zero rate unaffordable and the reward paths
/// go untested.
const REWARD_PERIOD: i64 = 30 * 86_400;

/// Realm settings for the campaign, and the fix for a real problem.
///
/// The end-to-end tests run a realm at the protocol minimums — a 1-hour voting
/// period and a 1-hour timelock — because they drive one proposal by hand and
/// want to warp seconds. The fuzzer's clock has to do two jobs at once: expire a
/// 180-day stake lock *and* land inside a voting window. With a 1-hour window it
/// cannot: the first measured campaign activated 18 proposals, voted on 7, and
/// executed none, because every warp large enough to matter to staking stepped
/// clean over governance.
///
/// So the fuzz realm runs on the staking timescale. These are ordinary values
/// well inside the ranges `RealmParams::validate` accepts — a 3-day vote and a
/// 1-day timelock — and `min_weight_to_propose` is set high enough that small
/// positions are actually turned away, which is what the parameter is for.
pub const FUZZ_REALM_PARAMS: RealmParams = RealmParams {
    quorum_bps: 2_000,   // 20%
    approval_bps: 5_001, // simple majority
    voting_period: 3 * 86_400,
    timelock_delay: 86_400,
    min_weight_to_propose: 1_000_000,
};

impl Fuzzer {
    /// Builds a system whose mint and actors are derived from `seed`.
    pub fn new(fee: Option<TransferFee>, seed: u64) -> Self {
        // A stream of its own, so changing how operations are generated does not
        // change which keys a seed produces.
        let mut rng = Rng::new(seed ^ 0x5EED_0000_0000_0001);

        let mut sys = System::bootstrap_with_mint(rng.keypair(), fee, 0);
        sys.set_realm_params(FUZZ_REALM_PARAMS);
        let actors = (0..ACTORS)
            .map(|_| {
                let owner = rng.keypair();
                let (tokens, _) = sys.prepare_staker(&owner.pubkey(), ACTOR_FUNDING);
                Actor { owner, tokens }
            })
            .collect();

        Self {
            sys,
            actors,
            positions: Vec::new(),
            proposals: Vec::new(),
            voted: BTreeSet::new(),
            executed: BTreeSet::new(),
            seen_states: Vec::new(),
            watermark: 0,
            coverage: Coverage::default(),
        }
    }

    /// Runs `ops` in order, checking the oracle after each.
    ///
    /// Returns the index of the first violation and what it was, or `None` if
    /// the whole sequence held.
    pub fn run(&mut self, ops: &[Op]) -> Option<(usize, String)> {
        for (i, op) in ops.iter().enumerate() {
            if let Err(violation) = self.step(*op) {
                return Some((i, violation));
            }
            if let Err(violation) = self.check_invariants() {
                return Some((i, format!("after {op:?}: {violation}")));
            }
        }
        None
    }

    /// Applies one operation.
    ///
    /// `Err` means the *fuzzer* found something wrong — an unrecognised
    /// rejection, or a call that succeeded when it had to fail. An instruction
    /// failing for a listed reason is a normal outcome and returns `Ok`.
    fn step(&mut self, op: Op) -> Result<(), String> {
        // Set when the fuzzer's own bookkeeping says this call cannot legally
        // succeed. Checked against the outcome below.
        let mut must_fail: Option<&str> = None;

        let outcome = match op {
            Op::Stake {
                actor,
                amount,
                tier,
            } => {
                let actor = actor % self.actors.len();
                let tier = [
                    LockTier::Flexible,
                    LockTier::Bronze,
                    LockTier::Silver,
                    LockTier::Gold,
                ][tier as usize % 4];
                let owner = self.actors[actor].owner.insecure_clone();
                let tokens = self.actors[actor].tokens;
                let position_id = self.sys.next_position_id();

                let ix = self
                    .sys
                    .stake_ix_for(&owner.pubkey(), &tokens, position_id, amount, tier);
                let result = self.sys.env.try_send(&[ix], &[&owner]);

                if result.is_ok() {
                    let (key, _) = pda::position(&self.sys.pool, &owner.pubkey(), position_id);
                    self.positions.push(PositionRef { actor, key });
                }
                result
            }

            Op::Unstake { position, bps } => {
                let Some(index) = self.position_index(position) else {
                    return Ok(());
                };
                let held: Position = self.sys.env.anchor_account(&self.positions[index].key);
                let amount = (held.amount as u128 * bps as u128 / 10_000) as u64;
                let (owner, tokens) = self.holder(index);
                let ix = self.sys.unstake_ix_for(
                    &owner.pubkey(),
                    &tokens,
                    self.positions[index].key,
                    amount.max(1),
                );
                self.sys.env.try_send(&[ix], &[&owner])
            }

            Op::Claim { position } => {
                let Some(index) = self.position_index(position) else {
                    return Ok(());
                };
                let (owner, tokens) = self.holder(index);
                let ix = self
                    .sys
                    .claim_ix_for(&owner.pubkey(), &tokens, self.positions[index].key);
                self.sys.env.try_send(&[ix], &[&owner])
            }

            Op::FundRewards { amount } => {
                let ix = self.sys.fund_rewards_ix(amount);
                self.sys.env.try_send(&[ix], &[])
            }

            Op::SetRewardRate { rate } => {
                let end = self.sys.env.now() + REWARD_PERIOD;
                let ix = self.sys.set_reward_rate_ix(rate, end);
                self.sys.env.try_send(&[ix], &[])
            }

            Op::SetPaused { paused } => {
                let ix = self.sys.set_paused_ix(paused);
                self.sys.env.try_send(&[ix], &[])
            }

            Op::Propose { position } => {
                let Some(index) = self.position_index(position) else {
                    return Ok(());
                };
                let (owner, _) = self.holder(index);
                let proposal_id = self.sys.next_proposal_id();
                let ix = self.sys.create_proposal_ix_for(
                    &owner.pubkey(),
                    proposal_id,
                    ProposalAction::Signal,
                    self.positions[index].key,
                );
                let result = self.sys.env.try_send(&[ix], &[&owner]);

                if result.is_ok() {
                    let (key, _) = pda::proposal(&self.sys.realm, proposal_id);
                    self.proposals.push(key);
                }
                result
            }

            Op::Activate { proposal } => {
                let Some(index) = self.proposal_in(proposal, ProposalState::Draft) else {
                    return Ok(());
                };
                let ix = self.sys.activate_ix(self.proposals[index]);
                self.sys.env.try_send(&[ix], &[])
            }

            Op::Vote {
                proposal,
                position,
                choice,
            } => {
                let Some(p) = self.proposal_open_for_voting(proposal) else {
                    return Ok(());
                };
                let Some(v) = self.eligible_voter(p, position) else {
                    return Ok(());
                };
                if self.voted.contains(&(p, v)) {
                    must_fail = Some("§4.1 a position voted twice on the same proposal");
                }

                let choice = [VoteChoice::For, VoteChoice::Against, VoteChoice::Abstain]
                    [choice as usize % 3];
                let (owner, _) = self.holder(v);
                let ix = self.sys.vote_ix_for(
                    &owner.pubkey(),
                    self.proposals[p],
                    self.positions[v].key,
                    choice,
                );
                let result = self.sys.env.try_send(&[ix], &[&owner]);

                if result.is_ok() {
                    self.voted.insert((p, v));
                }
                result
            }

            Op::Finalize { proposal } => {
                let Some(index) = self.proposal_ready_to_finalize(proposal) else {
                    return Ok(());
                };
                let ix = self.sys.advance_ix(self.proposals[index], true);
                self.sys.env.try_send(&[ix], &[])
            }

            Op::Queue { proposal } => {
                let Some(index) = self.proposal_in(proposal, ProposalState::Succeeded) else {
                    return Ok(());
                };
                let ix = self.sys.advance_ix(self.proposals[index], false);
                self.sys.env.try_send(&[ix], &[])
            }

            Op::Execute { proposal } => {
                let Some(index) = self.proposal_in(proposal, ProposalState::Queued) else {
                    return Ok(());
                };
                if self.executed.contains(&index) {
                    must_fail = Some("§4.5 a proposal executed twice");
                }

                let ix = self.sys.execute_signal_ix(self.proposals[index]);
                let result = self.sys.env.try_send(&[ix], &[]);

                if result.is_ok() {
                    self.executed.insert(index);
                }
                result
            }

            Op::Cancel { proposal } => {
                let Some(index) = self.proposal_index(proposal) else {
                    return Ok(());
                };
                if self.executed.contains(&index) {
                    must_fail = Some("§4.7 an executed proposal was cancelled after the fact");
                }
                let ix = self.sys.cancel_ix(self.proposals[index]);
                let guardian = self.sys.guardian.insecure_clone();
                self.sys.env.try_send(&[ix], &[&guardian])
            }

            Op::Warp { seconds } => {
                self.sys.env.warp_forward(seconds);
                Ok(())
            }

            Op::WarpToDeadline { proposal } => {
                // Only Voting and Queued have a deadline to move to — the other
                // states leave `voting_ends_at` and `eta` at zero. Picking
                // blindly and giving up on anything else wasted most of this
                // operation's budget, which is the whole reason the lifecycle
                // stalled short of `queue`.
                let Some(index) = self.proposal_awaiting_a_deadline(proposal) else {
                    return Ok(());
                };
                let proposal: Proposal = self.sys.env.anchor_account(&self.proposals[index]);
                let deadline = match proposal.state {
                    ProposalState::Voting => proposal.voting_ends_at,
                    ProposalState::Queued => proposal.eta,
                    _ => return Ok(()),
                };
                self.warp_to(deadline);
                Ok(())
            }

            Op::WarpToUnlock { position } => {
                let Some(index) = self.position_index(position) else {
                    return Ok(());
                };
                let held: Position = self.sys.env.anchor_account(&self.positions[index].key);
                self.warp_to(held.lock_end);
                Ok(())
            }
        };

        match (outcome, must_fail) {
            (Ok(()), Some(why)) => Err(format!("{op:?} succeeded but {why}")),
            (Ok(()), None) => {
                self.coverage.record(&op, None);
                Ok(())
            }
            (Err(err), _) => match EXPECTED_REJECTIONS.iter().find(|name| err.contains(*name)) {
                Some(reason) => {
                    self.coverage.record(&op, Some(reason));
                    Ok(())
                }
                None => Err(format!(
                    "{op:?} was rejected for an unrecognised reason: {err}"
                )),
            },
        }
    }

    /// Moves the clock to one second past `timestamp`, or not at all if it has
    /// already gone by. Never backwards.
    fn warp_to(&mut self, timestamp: i64) {
        let delta = timestamp - self.sys.env.now() + 1;
        if delta > 0 {
            self.sys.env.warp_forward(delta);
        }
    }

    /// The address of the `index`-th proposal this run created, for tests that
    /// need to reach past the oracle and into the account itself.
    pub fn proposal_key(&self, index: usize) -> Pubkey {
        self.proposals[index]
    }

    fn holder(&self, position: usize) -> (Keypair, Pubkey) {
        let actor = &self.actors[self.positions[position].actor];
        (actor.owner.insecure_clone(), actor.tokens)
    }

    fn position_index(&self, raw: usize) -> Option<usize> {
        (!self.positions.is_empty()).then(|| raw % self.positions.len())
    }

    fn proposal_index(&self, raw: usize) -> Option<usize> {
        (!self.proposals.is_empty()).then(|| raw % self.proposals.len())
    }

    /// Picks a proposal, preferring one already in `wanted`.
    ///
    /// Without this the campaign never gets past `finalize`. The governance
    /// lifecycle is seven ordered steps, so choosing targets uniformly means the
    /// chance of assembling propose → activate → vote → finalize → queue →
    /// execute against *one* proposal decays geometrically: the first measured
    /// run reached `activate` 18 times, `vote` 6, and `queue` never. Ninety-odd
    /// percent of governance operations were bouncing off `InvalidProposalState`
    /// and the deep states — the timelock, double execution, the quorum
    /// arithmetic — went untested.
    ///
    /// Biasing toward legal transitions is what a stateful fuzzer's generator is
    /// *for*; the alternative spends its whole budget re-proving that the state
    /// machine rejects nonsense. The guards still get exercised, because the
    /// fallback below fires whenever nothing is in `wanted` — and
    /// `the_fuzzer_is_not_vacuous` fails if any operation stops being rejected.
    ///
    /// Resolution reads chain state, but chain state is itself a pure function
    /// of `(seed, ops)`, so a run stays reproducible and the shrinker stays
    /// meaningful.
    fn proposal_in(&self, raw: usize, wanted: ProposalState) -> Option<usize> {
        let matching: Vec<usize> = (0..self.proposals.len())
            .filter(|i| self.proposal_state(*i) == Some(wanted))
            .collect();

        if matching.is_empty() {
            return self.proposal_index(raw);
        }
        Some(matching[raw % matching.len()])
    }

    /// A position that could actually carry a vote on `proposal`.
    ///
    /// Quorum is what the campaign kept failing to reach: proposals were being
    /// finalised `Defeated` eleven times for every one that survived, because a
    /// vote needs roughly a fifth of the pool behind it and each `Op::Vote` was
    /// picking a position uniformly — usually one that had already voted, or was
    /// flexible-tier, or was staked after the snapshot. One vote landed per
    /// proposal where three were needed.
    ///
    /// Selecting an eligible voter is the difference between exercising the
    /// tally arithmetic and exercising the rejection path over and over. The
    /// rejection path is still reached through the fallback, and through every
    /// proposal for which no eligible voter is left.
    fn eligible_voter(&self, proposal_index: usize, raw: usize) -> Option<usize> {
        let proposal: Proposal = self.sys.env.anchor_account(&self.proposals[proposal_index]);

        let eligible: Vec<usize> = (0..self.positions.len())
            .filter(|v| {
                if self.voted.contains(&(proposal_index, *v)) {
                    return false;
                }
                let held: Position = self.sys.env.anchor_account(&self.positions[*v].key);
                held.weighted_amount > 0
                    && held.lock_end >= proposal.voting_ends_at
                    && held.position_id < proposal.position_count_snapshot
            })
            .collect();

        if eligible.is_empty() {
            return self.position_index(raw);
        }
        Some(eligible[raw % eligible.len()])
    }

    /// A proposal whose voting window is open *right now*.
    ///
    /// State alone is not enough. A proposal stays in `Voting` after its window
    /// closes — nothing moves it until someone finalises — so selecting on state
    /// sent most votes at proposals that could only answer `VotingEnded`. It was
    /// the single largest sink in the funnel: 100 of 243 vote attempts.
    fn proposal_open_for_voting(&self, raw: usize) -> Option<usize> {
        let now = self.sys.env.now();
        self.proposal_matching(raw, |p| {
            p.state == ProposalState::Voting && now < p.voting_ends_at
        })
    }

    /// A proposal in `Voting` whose window has closed, so finalising is legal.
    fn proposal_ready_to_finalize(&self, raw: usize) -> Option<usize> {
        let now = self.sys.env.now();
        self.proposal_matching(raw, |p| {
            p.state == ProposalState::Voting && now >= p.voting_ends_at
        })
    }

    /// Picks a proposal satisfying `wanted`, falling back to any proposal at all.
    ///
    /// The fallback is what keeps the guards under test: when nothing matches,
    /// the operation goes out anyway and gets refused. `the_fuzzer_is_not_vacuous`
    /// fails if that stops happening.
    fn proposal_matching(&self, raw: usize, wanted: impl Fn(&Proposal) -> bool) -> Option<usize> {
        let matching: Vec<usize> = (0..self.proposals.len())
            .filter(|i| {
                let p: Proposal = self.sys.env.anchor_account(&self.proposals[*i]);
                wanted(&p)
            })
            .collect();

        if matching.is_empty() {
            return self.proposal_index(raw);
        }
        Some(matching[raw % matching.len()])
    }

    /// A proposal with a clock deadline pending — one in `Voting` or `Queued`.
    fn proposal_awaiting_a_deadline(&self, raw: usize) -> Option<usize> {
        let waiting: Vec<usize> = (0..self.proposals.len())
            .filter(|i| {
                matches!(
                    self.proposal_state(*i),
                    Some(ProposalState::Voting) | Some(ProposalState::Queued)
                )
            })
            .collect();

        if waiting.is_empty() {
            return None;
        }
        Some(waiting[raw % waiting.len()])
    }

    /// Every proposal's end state, for diagnosing where a campaign stalls.
    pub fn final_proposal_states(&self) -> Vec<ProposalState> {
        (0..self.proposals.len())
            .filter_map(|i| self.proposal_state(i))
            .collect()
    }

    fn proposal_state(&self, index: usize) -> Option<ProposalState> {
        let key = *self.proposals.get(index)?;
        let proposal: Proposal = self.sys.env.anchor_account(&key);
        Some(proposal.state)
    }

    /// The oracle: every aggregate invariant, read from the accounts.
    ///
    /// Cited by section so a failure points at the documented property rather
    /// than at a bare inequality.
    pub fn check_invariants(&mut self) -> Result<(), String> {
        self.check_staking()?;
        self.check_governance()
    }

    fn check_staking(&mut self) -> Result<(), String> {
        let pool: Pool = self.sys.env.anchor_account(&self.sys.pool);
        let positions: Vec<Position> = self
            .positions
            .iter()
            .map(|p| self.sys.env.anchor_account(&p.key))
            .collect();

        // §3.1 — the accumulator never runs backwards.
        if pool.reward_per_token < self.watermark {
            return Err(format!(
                "§3.1 reward_per_token went backwards: {} then {}",
                self.watermark, pool.reward_per_token
            ));
        }
        self.watermark = pool.reward_per_token;

        // §1.3 — the pool's total is the sum of its positions.
        let staked: u64 = positions.iter().map(|p| p.amount).sum();
        if pool.total_staked != staked {
            return Err(format!(
                "§1.3 pool.total_staked ({}) != Σ position.amount ({staked})",
                pool.total_staked
            ));
        }

        // §1.1 — and that total is actually in the vault.
        let vault = self.sys.env.token_balance(&self.sys.stake_vault);
        if vault != staked {
            return Err(format!(
                "§1.1 stake_vault ({vault}) != Σ position.amount ({staked})"
            ));
        }

        // §1.4 — weight likewise, and each position's weight still derives from
        // its own principal. Partial withdrawal recomputes weight from the
        // remainder rather than subtracting proportionally, so this has to stay
        // exact however many have happened.
        let weighted: u64 = positions.iter().map(|p| p.weighted_amount).sum();
        if pool.total_weighted != weighted {
            return Err(format!(
                "§1.4 pool.total_weighted ({}) != Σ weighted ({weighted})",
                pool.total_weighted
            ));
        }
        for (p, position) in self.positions.iter().zip(&positions) {
            let expected = position
                .tier
                .apply_weight(position.amount)
                .map_err(|e| format!("§1.4 weight overflowed for {}: {e:?}", p.key))?;
            if position.weighted_amount != expected {
                return Err(format!(
                    "§1.4 {} holds weight {} but {} of {:?} weighs {expected}",
                    p.key, position.weighted_amount, position.amount, position.tier
                ));
            }
        }

        // §1.2 — the solvency invariant, and the reason this fuzzer exists.
        //
        // Projected to *now* rather than to the pool's last update: emissions
        // accrue with the clock whether or not anyone has touched the pool, so
        // checking at the stale watermark would miss a pool that has already
        // promised more than it holds. `update_rewards` is a pure function of
        // state and time, so a clone can be advanced without writing anything —
        // and using the program's own function means this is not a second
        // implementation of the accrual rule.
        let now = self.sys.env.now();
        let mut projected = pool.clone();
        projected
            .update_rewards(now)
            .map_err(|e| format!("§1.2 update_rewards failed at {now}: {e:?}"))?;

        let mut owed: u128 = 0;
        for position in &positions {
            owed += position
                .earned(projected.reward_per_token)
                .map_err(|e| format!("§1.2 earned() failed: {e:?}"))? as u128;
        }
        let reward_vault = self.sys.env.token_balance(&self.sys.reward_vault);
        if owed > reward_vault as u128 {
            return Err(format!(
                "§1.2 positions are owed {owed} but the reward vault holds {reward_vault}"
            ));
        }

        // §3.2 — booked liability never understates what has been paid.
        if pool.total_rewards_accrued < pool.total_rewards_paid {
            return Err(format!(
                "§3.2 accrued ({}) < paid ({})",
                pool.total_rewards_accrued, pool.total_rewards_paid
            ));
        }

        // Every position that ever opened is still counted. Positions are never
        // closed, so the counter and the fuzzer's own list must agree exactly.
        if pool.position_count as usize != self.positions.len() {
            return Err(format!(
                "pool.position_count ({}) != positions opened ({})",
                pool.position_count,
                self.positions.len()
            ));
        }

        Ok(())
    }

    fn check_governance(&mut self) -> Result<(), String> {
        let realm: Realm = self.sys.env.anchor_account(&self.sys.realm);
        if realm.proposal_count as usize != self.proposals.len() {
            return Err(format!(
                "realm.proposal_count ({}) != proposals created ({})",
                realm.proposal_count,
                self.proposals.len()
            ));
        }

        for (index, key) in self.proposals.iter().enumerate() {
            let proposal: Proposal = self.sys.env.anchor_account(key);

            // §4.3 — no more weight voted than was snapshotted. Checked for
            // every proposal after every operation, so a vote on one cannot
            // corrupt the tally of another.
            let cast = proposal
                .total_votes()
                .map_err(|e| format!("§4.3 total_votes overflowed on proposal {index}: {e:?}"))?;
            if cast > proposal.total_weight_snapshot {
                return Err(format!(
                    "§4.3 proposal {index} counted {cast} of a {} snapshot \
                     (for {}, against {}, abstain {})",
                    proposal.total_weight_snapshot,
                    proposal.for_votes,
                    proposal.against_votes,
                    proposal.abstain_votes
                ));
            }

            // §4.6 — the state moved along a documented edge, or not at all.
            match self.seen_states.get(index) {
                None => self.seen_states.push(proposal.state),
                Some(&before) => {
                    if before != proposal.state && !is_legal_transition(before, proposal.state) {
                        return Err(format!(
                            "§4.6 proposal {index} went {before:?} -> {:?}, which is not a \
                             documented transition",
                            proposal.state
                        ));
                    }
                    self.seen_states[index] = proposal.state;
                }
            }
        }

        Ok(())
    }
}

/// The proposal lifecycle, as an edge set.
///
/// Written out rather than derived from the program's own guards on purpose: a
/// state machine checked against a copy of itself checks nothing. This is the
/// table from the architecture review, and it is the thing the program is being
/// tested against.
fn is_legal_transition(from: ProposalState, to: ProposalState) -> bool {
    use ProposalState::*;
    matches!(
        (from, to),
        (Draft, Voting)
            | (Draft, Cancelled)
            | (Voting, Succeeded)
            | (Voting, Defeated)
            | (Voting, Cancelled)
            | (Succeeded, Queued)
            | (Succeeded, Cancelled)
            | (Queued, Executed)
            | (Queued, Cancelled)
    )
}

/// Generates a sequence of `len` operations from `seed`.
pub fn sequence(seed: u64, len: usize) -> Vec<Op> {
    let mut rng = Rng::new(seed);
    (0..len).map(|_| Op::generate(&mut rng)).collect()
}

/// Reduces a failing sequence by deleting operations it does not need.
///
/// Straightforward delta debugging: try removing each operation in turn, keep
/// the removal if the failure survives. A sixty-step trace usually reduces to a
/// handful, and the difference between those two is whether anyone can act on
/// the report.
pub fn shrink(fee: Option<TransferFee>, seed: u64, ops: &[Op]) -> Vec<Op> {
    shrink_with(ops, |candidate| {
        Fuzzer::new(fee, seed).run(candidate).is_some()
    })
}

/// The reduction itself, over an arbitrary failure predicate.
///
/// Split out so the shrinker can be tested on a predicate whose minimal case is
/// known, rather than only on a real bug — which, if the programs are correct,
/// is a test that can never run.
pub fn shrink_with(ops: &[Op], mut fails: impl FnMut(&[Op]) -> bool) -> Vec<Op> {
    let mut current = ops.to_vec();
    let mut i = 0;
    while i < current.len() {
        let mut without = current.clone();
        without.remove(i);
        if fails(&without) {
            current = without;
        } else {
            i += 1;
        }
    }
    current
}
