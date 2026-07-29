# Threat model

Attacks this system is designed against, how the defence works, and what remains
out of scope. Written to be read by someone deciding whether to trust the code.

## Trust assumptions

| Actor | Trusted with | Explicitly **not** trusted with |
|-------|--------------|--------------------------------|
| Admin (token-manager) | Registering minters, pausing new deposits | Minting (no key holds mint authority), treasury funds, passing proposals |
| Guardian (governance) | Cancelling a proposal before execution | Creating, passing, queueing or executing anything |
| Upgrade authority | Program upgrades during the beta window | Anything after authority burn — see [Upgrade path](#upgrade-path) |
| Stakers | Nothing. All staker input is hostile until validated. | — |

The design goal is that a full compromise of the admin key cannot move user funds, and
a full compromise of the guardian key can only stop things happening, never cause them.

---

## Attacks and defences

### A1 — Flash-loan governance capture

*Borrow a large HLX balance, stake it, vote, unstake, repay, all atomically.*

**Defence.** A position votes only if `lock_end >= proposal.voting_ends_at`. A position
created inside the attacking transaction has `lock_end == now`, which is strictly less
than any live proposal's end. The borrowed stake carries zero weight.

Note this is stronger than a snapshot at proposal creation, which is vulnerable to
borrowing *before* the snapshot block. Here the capital must be genuinely locked
across the entire voting window.

### A2 — Reward vault drain by rounding

*Deposit and withdraw repeatedly, keeping a rounding remainder each time.*

**Defence.** Both fixed-point divisions truncate toward zero, so every rounding error
accrues to the pool, never the user. Invariant 3.3 asserts the on-chain payout is
never greater than the exact rational entitlement, differentially against a
big-rational computation in the test harness.

### A3 — Transfer-fee accounting mismatch

*Stake a Token-2022 mint with a transfer fee; the vault receives less than the credited
amount; withdraw the full credit and drain the difference from other stakers.*

**Defence.** Deposits credit the observed vault balance delta, not the instruction's
`amount` argument (invariant 2.1). This is the failure mode most likely to be missed,
because it only appears on mints with the extension enabled — the test suite runs the
whole staking flow twice, once on a plain mint and once on a fee-bearing mint.

### A2b — Post-snapshot vote stuffing

*Wait for a proposal to open, then stake heavily and vote, clearing a quorum measured
before your capital existed.*

**Defence.** `activate_proposal` records `position_count_snapshot` alongside the weight
snapshot, and `cast_vote` requires `position.position_id < position_count_snapshot`.
Position ids come from a pool-wide monotonic counter, so that comparison is exactly "this
position existed when the denominator was measured".

Distinct from A1 and not implied by it. A1 is about capital that can leave before the vote
closes; this attack uses capital locked for 180 days, which satisfies A1's gate completely.
A1 is commitment forward in time, this is membership backward in time, and a system can
have one without the other — as this one did until a stateful fuzzer generated the
operations in an order no hand-written test had.
[F-10](./SECURITY-ASSESSMENT.md#f-10--post-snapshot-weight-could-vote), fixed, pinned by
`a_position_opened_after_the_snapshot_cannot_vote` and by invariants §4.3 and §4.13.

### A4 — Compute exhaustion / unbounded iteration

*Grow the staker set until reward distribution exceeds the compute budget and the pool
is permanently stuck.*

**Defence.** No instruction iterates over stakers, positions, or voters. Reward
distribution is O(1) via the accumulator; vote tallies are running totals updated per
vote.

Measured, not assumed. Across a 64× sweep in staker count `stake` and `unstake` are
bit-identical, and the 64th vote on a proposal costs the same as the first. `claim` moves
0.8%, and reaching the same staked total with 64 stakers or with one costs bit-identical
compute — so the staker set is not what moves it. Every instruction has better than 4×
headroom against the default budget, the worst being `execute_treasury_transfer` at 17.9%.
See [invariant §6.3](./INVARIANTS.md#6-liveness) and
[TESTING.md](./TESTING.md#compute-cost).

### A5 — Double voting / vote replay

**Defence.** `VoteRecord` is a PDA seeded `["vote", proposal, position]` created with
`init`. A second vote from the same position fails at account creation, before any
handler logic runs. Seeding by position rather than by wallet lets a multi-position
holder vote their full weight while keeping each position's vote once-only.

### A6 — Proposal execution replay

*Execute a passed proposal repeatedly to drain the treasury in multiples.*

**Defence.** `execute` requires state `Queued` and sets `Executed` before any CPI. The
state machine rejects a second attempt, and invariant 4.5 asserts it.

### A7 — Treasury spend outside governance

*Call the treasury's transfer instruction directly.*

**Defence.** The spend handler requires a signature from the governance execution PDA
for the configured realm, which only the governance program can produce, and only
inside `execute`. Invariant 5.1 asserts a direct call fails.

### A8 — Timelock bypass

**Defence.** `eta` is set at queue time from the on-chain clock and compared at
execute time. There is no admin path that shortens or skips it; changing
`timelock_delay` is itself a governance proposal, and the change applies only to
proposals queued after it takes effect.

### A9 — Rent-reclaim replay

*Close a position account, then re-derive and re-open it to replay accrued rewards.*

**Defence, in three layers.** The outer one is that the address cannot be reoccupied at
all: `position_id` must equal `pool.position_count`, which only ever increases, and
`close_position` deliberately does not decrement it. There is no id at which a closed
position's PDA can be re-created.

Behind that, `close_position` refuses while `pending_rewards` is non-zero, so there is
never an unclaimed credit on an account being deallocated. And behind *that*, even if a
PDA could be re-initialised, the fresh account snaps `reward_per_token_paid` to the
*current* accumulator, so no historical rewards are reachable.

The first layer is the one that took work, and it is the reason this entry is worth
re-reading rather than skimming: the natural implementation of rent reclamation decrements
the counter, which reopens [A2b](#a2b--post-snapshot-vote-stuffing) — not this attack.
See [F-7](./SECURITY-ASSESSMENT.md#f-7--position-accounts-never-closed).

### A10 — PDA seed collision

*Craft inputs where two logically distinct accounts derive the same address.*

**Defence.** All seeds are fixed-length or length-prefixed — no raw user strings are
concatenated. `position_id` is a `u64` in little-endian bytes, not a user-supplied
`Vec<u8>`. Bumps are stored at init and passed to `seeds`/`bump` on every subsequent
use, so only canonical bumps are ever accepted (invariant 5.5, now asserted at runtime:
`canonical_bumps` checks every stored bump against `find_program_address`, and
`a_non_canonical_derivation_is_refused` builds a real second PDA at a lower bump and
confirms the program rejects it as `unstake`'s vault authority).

### A11 — Malicious proposal drains treasury in one shot

**Defence, partial.** This is governance working as designed — if an attacker holds
genuine locked majority stake, they control the DAO. Mitigations reduce blast radius
rather than prevent it: the per-epoch spend limit caps a single drain, the timelock
gives users a window to exit, and the guardian can veto. A determined majority
attacker with patience still wins. That is a property of token governance, not a bug,
and pretending otherwise would be dishonest.

---

## Out of scope

- **Oracle manipulation** — the protocol reads no external prices.
- **Validator-level censorship / MEV** — not addressable at the program layer.
- **Frontend and RPC compromise** — mitigated operationally (SRI, pinned RPC,
  transaction simulation before signing), not by the programs.
- **Compromise of a user's own wallet key.**
- **Majority stake capture**, per A11.

## Upgrade path

Programs ship upgradeable, with the authority held by a 3-of-5 Squads multisig during
beta. The intended end state is transferring upgrade authority to the governance
program itself, so upgrades follow the same timelocked proposal path as spends. The
`docs/RUNBOOK.md` deployment checklist treats authority transfer as an explicit,
verifiable step rather than something that happens by default — an unmigrated upgrade
authority is the most common gap between "audited" and "actually safe".

## Reporting

See [SECURITY.md](../SECURITY.md).
