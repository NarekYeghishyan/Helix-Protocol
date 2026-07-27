# governance

Proposals, voting, quorum and timelock. Holds no transferable assets — its only
output is a PDA signature that the treasury and the staking pool accept.

## Why flash loans do not work here

The standard attack on token governance: borrow a large balance, vote, repay, all in
one transaction. The usual defences are historical snapshots (expensive on Solana, and
gameable by borrowing *before* the snapshot block) or vote-escrow.

Helix uses neither. A position may vote only if:

```text
position.lock_end >= proposal.voting_ends_at
```

You can only vote with stake you are contractually unable to withdraw before the vote
closes. A position opened inside the attacking transaction has `lock_end` at or near
`now`, strictly less than any live proposal's end, so borrowed capital carries exactly
zero weight.

This is *stronger* than a snapshot — the capital must stay locked across the entire
voting window, not merely exist at one block — and it costs one comparison, because the
staking design was chosen to make it cheap. It lives in
[`instructions/vote.rs`](./src/instructions/vote.rs).

## Lifecycle

```text
Draft ──activate──▶ Voting ──finalize──┬─▶ Succeeded ──queue──▶ Queued
                                       │                          │
                                       └─▶ Defeated          (eta elapsed,
                                                              before expiry)
                                                                  │
      Cancelled ◀── guardian veto, any pre-execution state ──  Executed
```

- **Quorum**: `for + against + abstain >= quorum_bps` of snapshotted weight.
- **Approval**: `for >= approval_bps` of `for + against`. Abstentions count toward
  quorum but not approval — standard Compound/OZ semantics, and what makes abstaining a
  meaningful act rather than just not voting.
- **Snapshot**: `total_weighted` is fixed at activation, never re-read. Reading it live
  at finalisation would let a whale defeat a proposal by staking more after seeing how
  the vote was going, inflating the denominator until quorum failed.
- **Timelock**: `eta` is computed at queue time and never recomputed, so changing
  `timelock_delay` afterwards cannot shorten the delay on something already queued.
- **Expiry**: a queued proposal must execute within `EXECUTION_GRACE_PERIOD`. Otherwise
  a proposal passed under one set of conditions could lie dormant and then execute into
  a completely different world a year later.

`activate` and `finalize` are **permissionless**: both are pure functions of on-chain
state, so there is nothing to decide, only to record. Making them permissioned would
let whoever held the permission strand a proposal they disliked forever.

## The guardian

May veto a proposal before execution, and may do nothing else. It cannot create,
activate, pass, queue or execute anything. A guardian that could also *pass* proposals
would not be a safety mechanism — it would be an admin key with a reassuring name.

Vetoing an already-executed proposal is refused: the effect has happened, and recording
it as cancelled would misreport what the chain did.

## Actions are a closed set

`ProposalAction` is an enum — `Signal`, `TreasuryTransfer`, `SetStakingRewardRate` — not
a blob of serialised instruction data.

General-purpose governance (SPL Governance, OZ Governor) lets a proposal carry arbitrary
CPIs. That is more flexible and much harder to reason about: a voter must decode raw
instruction bytes to know what they are approving. Here the set of things governance
*can* do is fixed at deploy time and visible in the IDL, so a voter reads the variant
and knows the blast radius. Extending the set requires a program upgrade, which is
itself governed.

There is one instruction per variant rather than a single `execute` with
`remaining_accounts`. It costs some repetition and buys two things worth more: the
accounts each action touches appear in the IDL, and every account is a typed Anchor
account rather than a bare `AccountInfo` validated by hand.

Execution reads its parameters by destructuring `proposal.action` — the amount and
destination are whatever the *voters approved*, never what the executing caller passes
in.

## Replay protection

`authorize_execution` sets `Executed` **before** any CPI. Marking afterwards would leave
a window in which a re-entrant call could observe the proposal still `Queued` and
execute it twice (`INVARIANTS.md` §4.5).

## Tests

`cargo test -p helix-governance` — 17 unit tests over the tally and lifecycle:
quorum counting abstentions, cross-multiplied thresholds that lose nothing to rounding,
unanimous-but-below-quorum being defeated, everyone-abstaining approving nothing,
supermajority boundaries, and the guardian's veto window.
