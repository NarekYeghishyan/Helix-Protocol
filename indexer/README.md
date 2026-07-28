# Indexer

Reconstructs Helix protocol state from the events the four programs emit.

Every state transition emits an Anchor event carrying an on-chain timestamp, so
history is reconstructable from transaction logs without polling account state.
This crate does the reconstructing — decode, attribute, fold.

```bash
cargo test -p helix-indexer                                        # 23 unit tests
cargo test -p helix-integration-tests --test indexer_reconciliation # 7 against the chain
```

## What it is not

There is no RPC client, no database driver and no network I/O anywhere in this
crate. That is deliberate, not unfinished.

Ingestion is the part that cannot be tested without a cluster. Decoding and
folding are the parts where the bugs that corrupt analytics actually live. Keeping
them pure means they can be tested against the real programs **today**, before
anything is deployed — which is what [`indexer_reconciliation.rs`](../tests/integration/tests/indexer_reconciliation.rs)
does: real transactions, the runtime's own logs, and the resulting projection
compared to the accounts those transactions wrote, field by field.

An analytics stack that claims to match the chain and cannot demonstrate it is a
rumour. What remains — the subscriber, the backfill, the Postgres binding, the
read API — is [Phase 4 remainder](../docs/ROADMAP.md#phase-4--indexer-and-analytics-api).

## Why Rust, and why it links the programs

The crate depends on the four program crates directly, so:

- **the event types are the programs' own.** A field added to `Staked` appears
  here without anyone editing this crate; a field removed is a compile error
  rather than a column that quietly stops being populated.
- **derived figures use the programs' arithmetic.** No second implementation of
  the weight table to drift out of sync at the next upgrade.

A TypeScript client re-declares the schema from the IDL and re-implements every
derived figure. The two copies agree until the first program change, and nothing
compares them.

## The three problems worth knowing about

**Attribution is not obvious.** The runtime interleaves every program's output
into one flat list, so a `Program data:` line says nothing about who emitted it.
The only way to know is to track the invoke stack — and in `execute_treasury_transfer`,
the deepest call stack in the protocol, both events land *after* a deeper program
has already returned:

```text
Program <governance> invoke [1]
Program <treasury> invoke [2]
Program <token-2022> invoke [3]
Program <token-2022> success
Program data: ...        <- treasury's, at depth 2
Program <treasury> success
Program data: ...        <- governance's, at depth 1
Program <governance> success
```

"Most recent invoke" hands the first to Token-2022 and drops it. "The
transaction's program" hands both to governance. Both are wrong, and on a
single-program transaction both look right — which is why the nested case has its
own test.

**Delivery is at-least-once, at best.** Confirmed logs get redelivered, a backfill
overlaps a live stream, and a reorg makes both happen at once. `Analytics::apply_transaction`
is idempotent on `(signature, log_index)`. The log index is not optional: one
transaction routinely emits several events, and two can be byte-identical —
staking the same amount into the same pool twice in one transaction is legitimate.

**Silence is never assumed to mean nothing happened.** A truncated log or an
undecodable payload is surfaced as an `Anomaly` rather than skipped, and an event
for an entity never seen created is recorded in `orphaned`. An indexer that drops
what it cannot read reports wrong numbers precisely when something unusual
happened, which is exactly when someone is looking at them.

## Verified how

The reconciliation tests were **mutation-tested**. Attributing events to the
outermost program instead of the innermost — a one-word change, `stack.first()`
for `stack.last()` — turns 5 unit tests and 2 reconciliation tests red:

```text
a_nested_cpi_attributes_each_event_to_its_own_program            FAILED
the_indexed_proposal_matches_the_chain_through_execution         FAILED
  the treasury spend was attributed to the wrong program, or lost
  left: 0   right: 2500000

the_indexed_pool_matches_the_chain_through_a_full_staking_lifecycle  ok  <-- still passes
indexed_tvl_follows_credited_amounts_on_a_fee_bearing_mint           ok  <-- still passes
replaying_a_transaction_double_counts_nothing                        ok  <-- still passes
```

The three that stay green are the single-program flows. Under this mutation the
innermost program *is* the outermost, so they cannot distinguish the two — which
is the entire argument for testing the nested case separately.

## What building it found

`Unstaked` did not carry the position's remaining vote weight, so reconstructing
`pool.total_weighted` from the event stream meant re-running `LockTier::apply_weight`
off chain. That is a second implementation of the weight table: correct today,
silently wrong the day the table changes. The event now carries `weighted_amount`.

The general rule it produced: **an event that cannot be folded into state without
recomputation is an incomplete event.** Applied across all 34, this is the only
one that failed it.

A second, found by a test bug rather than by design: `treasury_balance` returned 0
for a treasury whose deposits predated the indexer, with nothing marking the
figure as computed from partial history. A dashboard would have shown an empty
treasury with complete confidence. Entities materialised by a non-creation event
are now recorded in `orphaned`.

## Layout

| Module | Responsibility |
|---|---|
| [`event.rs`](./src/event.rs) | The 34 event types, and decoding one from its wire form |
| [`logs.rs`](./src/logs.rs) | Attributing `Program data:` lines to the invocation that emitted them |
| [`projection.rs`](./src/projection.rs) | Folding events into queryable state, exactly once each |
| [`sql/schema.sql`](./sql/schema.sql) | Postgres DDL for the persistent form — **written, not yet exercised** |

## Read views

`Analytics` exposes what the dashboard needs: `tvl`, `apr_bps`,
`staker_distribution`, `proposal_history`, `treasury_balance`,
`committed_to_streams`.

`apr_bps` returns `None` rather than a number when nothing is staked. A rate over
an empty pool is not an infinite APR, it is an undefined one, and returning a
figure is how dashboards end up showing `∞%`.
