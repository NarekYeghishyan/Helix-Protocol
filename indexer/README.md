# Indexer

Reconstructs Helix protocol state from the events the four programs emit.

Every state transition emits an Anchor event carrying an on-chain timestamp, so
history is reconstructable from transaction logs without polling account state.
This crate does the reconstructing — decode, attribute, fold.

```bash
cargo test -p helix-indexer                                        # 50 unit tests
cargo test -p helix-indexer --all-features                          # incl. transport, RPC, store
cargo test -p helix-integration-tests --test indexer_reconciliation # 8 against the chain
HELIX_RPC_URL=http://127.0.0.1:8899 \
  cargo test -p helix-integration-tests --test rpc_source_live -- --test-threads=1  # 6 live
HELIX_DATABASE_URL=postgres://helix:helix@127.0.0.1:55432/helix \
  cargo test -p helix-integration-tests --test store_postgres                       # 10 live
```

## What it is not

**Nothing compiled by default does I/O.** Two modules do: [`rpc.rs`](./src/rpc.rs)
reads a cluster and [`store.rs`](./src/store.rs) writes to Postgres, and both are
behind features that are off.

That is deliberate, not unfinished. Ingestion is the part that cannot be tested
without a cluster. Decoding and folding are the parts where the bugs that corrupt
analytics actually live. Keeping the default build pure means they can be tested
against the real programs **today** — which is what
[`indexer_reconciliation.rs`](../tests/integration/tests/indexer_reconciliation.rs)
does: real transactions, the runtime's own logs, and the resulting projection
compared to the accounts those transactions wrote, field by field.

An analytics stack that claims to match the chain and cannot demonstrate it is a
rumour.

The same split runs through ingestion. *Deciding* what to do with what a source returns —
holding the unfinalised tail, noticing it has been replaced, rebuilding, promoting to
final, advancing the cursor — is [`ingest.rs`](./src/ingest.rs) and is tested against a
scripted source that rolls slots back on demand, because a cluster cannot be asked to
fork. *Talking to an RPC node* is [`rpc.rs`](./src/rpc.rs), tested against a validator
with the four programs deployed. *Keeping* what finalised is [`store.rs`](./src/store.rs),
tested against a real Postgres.

## The RPC source

```bash
cargo run -p helix-indexer --features rpc --bin helix-index -- \
  --url http://127.0.0.1:8899 --once
```

```text
following http://127.0.0.1:8899 (in memory only, not persisted) — Ctrl-C to stop
+3 applied, 0 finalized, 0 rows stored | cursor slot 0 | 3 unfinalized |
1 pools, 2 positions, 0 proposals, 1 treasuries, 1 realms | 0 orphaned
```

It speaks JSON-RPC directly rather than through `solana-rpc-client`, which is published at
4.2.0-rc while this workspace resolves the Solana crates at 3.x — the version split that
also ruled out Trident. The surface an indexer needs is three read methods carrying no
signatures and no account decoding, so the trade was a large graph for a thin wrapper.

Four things a real endpoint does that a fake would not have taught us, each producing a
projection that is merely *incorrect* rather than one that errors:

| | |
|---|---|
| `getSignaturesForAddress` answers **newest-first** | `LogSource` is specified in ledger order, and running totals are *assigned*, so a reversed batch settles on the oldest value in it |
| `limit` counts **from the newest end** | "the 100 after my cursor" is not expressible; the walk descends to the cursor and reverses, and refuses rather than guesses past `max_scan` |
| **Failed transactions are in the signature list** | their writes were rolled back and their events are still in the log |
| **`logMessages` is nullable** | a node with logs disabled answers successfully and reports that nothing was emitted |

All four are asserted in
[`rpc_source_live.rs`](../tests/integration/tests/rpc_source_live.rs) or in `rpc.rs`'s own
tests. Three of those assertions were vacuous when first written and were only found by
mutation testing — see
[TESTING.md](../docs/TESTING.md#the-live-tests-and-the-three-that-were-not-testing-their-claims).

## The backfill is a different traversal, not the same one reversed

The live poll chases a cursor upward. A backfill walks from the tip toward genesis, and the
second row of the table above is why it is not simply the poll with a sign flipped:
`before` and `limit` express "the newest N older than X" exactly, and "the oldest N newer
than X" not at all. A descent is the traversal that API was designed for, which is why
[`DescendingSource::fetch_before`](./src/source.rs) needs one request per page where
`fetch` needs a walk.

**The hard part is not reading, it is where the result may go.** Most of the projection
assigns running totals the events carry — the property that makes redelivery harmless — and
that property depends entirely on ledger order. Fold an older `RewardRateChanged` after a
newer one and the newer rate is overwritten by the older. No error, no anomaly, a plausible
number.

So [`Backfill`](./src/ingest.rs) owns no `Analytics` at all. It yields
`SettledTransaction`s for the `events` table, whose key is `(signature, log_index)` and
whose stated purpose is that everything else is derivable from it — an order-independent
destination. The projection is then rebuilt from those rows in slot order by
[`Analytics::replay`](./src/projection.rs). A `Backfill` holding a projection would invite
exactly the fold it exists to prevent, so it does not have one to offer.

Three smaller decisions, each because the obvious alternative is quietly wrong:

- **[`Descent`](./src/source.rs) is a separate type from `Cursor`, not a reused one.** It
  means the opposite thing — everything at or *above* its slot has been read. Reusing
  `Cursor` with an inverted sense would have compiled everywhere and been wrong in one
  direction.
- **A page carries its range, not just its contents.** A page of nothing but *failed*
  transactions is ordinary for a program someone is spamming: it has nothing to fold and is
  not the end of history. Reading `transactions.is_empty()` as genesis stops the descent
  early and reports the rest complete. `DescentPage::covered` is what actually terminates
  it.
- **The descent refuses the unfinalised tail and moves past it anyway.** It starts at the
  tip, inside the range the live stream owns. It skips those transactions — a row a fork can
  revoke is a number that was never true — and counts them, rather than stalling on them
  waiting for finality that the live stream is already handling.

The two meet by overlapping rather than by meeting exactly, which needs no coordination
between them because both write the same idempotent rows.

Verified two ways. `ingest.rs` drives it against a scripted ledger for the properties a
cluster cannot demonstrate on demand, and `rpc_source_live.rs` descends a real validator and
asserts the result reconstructs the same projection the *forward* pass builds — two
traversals of one history, compared against each other and against the accounts. Both
descent assertions were mutation-tested: handing pages back newest-first, and taking the
next page's bound before truncation rather than after, each fail them.

**What is not built is the store binding.** The `backfill` row of `cursors` has no writer
yet, so a descent is resumable in memory and not across a restart, and nothing yet clears
`partial_history` when one completes. That is the half that needs a database to test, and
it is scoped in [ROADMAP 4.3](../docs/ROADMAP.md#phase-4--indexer-and-analytics-api).

## Persistence

```bash
docker run -d --name helix-postgres -e POSTGRES_PASSWORD=helix -e POSTGRES_USER=helix \
  -e POSTGRES_DB=helix -p 55432:5432 postgres:16-alpine

cargo run -p helix-indexer --features rpc,postgres --bin helix-index -- \
  --url http://127.0.0.1:8899 \
  --database-url postgres://helix:helix@127.0.0.1:55432/helix
```

```text
database is empty — ingesting from the start of available history
following http://127.0.0.1:8899 (persisting to Postgres) — Ctrl-C to stop
+16 applied, 24 finalized, 26 rows stored | cursor slot 518 | 0 unfinalized |
5 pools, 5 positions, 0 proposals, 5 treasuries, 5 realms | 0 orphaned

$ # ...and again, as a restarted process
resuming at slot 518 | 5 pools, 5 positions, 0 proposals, 5 treasuries, 5 realms restored
following http://127.0.0.1:8899 (persisting to Postgres) — Ctrl-C to stop
+0 applied, 0 finalized, 0 rows stored | cursor slot 518 | 0 unfinalized | ...
```

24 transactions finalised but only 16 carried anything: `getSignaturesForAddress` on a
program id also returns the transactions that *deployed* it, and those emit no events.

### The rehearsal that reported an indexer newer than its chain

The first attempt at the run above, against a validator still holding the previous build,
printed this five times and reported zero realms:

```text
anomaly: 31PhgSLZ...xpLXZ UndecodableData { log_index: 23, program: Governance }
```

`RealmInitialized` had just gained `min_weight_to_propose`, so the deployed program was
emitting the old, shorter body and Borsh refused it — correctly, because the alternative
is decoding a prefix and inventing the rest. It is the mirror of the case
[`event.rs`](./src/event.rs) documents: usually an anomaly means the indexer is *older*
than the chain, and here it meant the opposite.

Both directions land in the same place, which is the point. The one outcome that never
happens is a plausible number with a missing field silently defaulted — and the run says
so with a signature attached, so the transaction can be looked up rather than guessed at.
After redeploying, the anomalies are gone and the realms appear.

Four properties, in the order they matter. Each fails silently rather than loudly,
which is why each has a test that injures it deliberately.

| Property | Why the alternative is worse than an error |
|---|---|
| **The cursor never gets ahead of the rows** | A crash between writing slot 900's rows and its cursor, in the other order, resumes at 901 and never asks for 900 again. Nothing reports it; the figures are just permanently a little too small. Every write and the cursor share one database transaction |
| **Only finalised state is written** | A row a fork can revoke is a number that was never true. The unfinalised tail is re-read from the source instead — seconds of chain, and it makes every stored row unconditionally correct |
| **Replay changes nothing** | Redelivery is routine. Inserts are `ON CONFLICT DO NOTHING`; upserts assign rather than accumulate and are guarded on `updated_at_slot`, so a backfill replaying slot 200 cannot overwrite a live write at slot 500 |
| **A loaded projection is the one that was saved** | Including the part that is not a number — see below |

**That last one is the interesting one.** The persisted rows carry the *result* of
folding an event. They do not carry the fact that it was folded, and that fact is
load-bearing: most of the projection assigns running totals the events supply,
which is what makes replay safe, but `Staked`, `Unstaked`, `RewardsClaimed` and
`StreamClaimed` publish a delta and no cumulative figure, so those four genuinely
accumulate. For them, idempotency *is* the applied set — and the applied set lives
in the process that is restarting.

So `load` restores it, for every event at or above the cursor's slot. That bound is
exact rather than cautious: below the cursor the ingestor refuses the transaction
outright as `FinalizedHistoryChanged`, so those events can never be re-folded; at
the cursor's own slot they can, because the cursor resumes mid-slot by signature
and an RPC node that has pruned that signature from its address index serves the
whole slot again. One slot deep, not the whole of history.

Without it the first redelivery after a restart double-counts, silently, only for
the transactions straddling the restart.

### What the binding changed about the schema

`sql/schema.sql` had been written and reviewed a phase earlier. Executing it moved
three things:

- **`payload` is `BYTEA`, not `JSONB`.** The table's stated purpose is that
  everything else is derivable from it, and derivable means decodable by
  `HelixEvent::decode` — the same function the live log goes through. Producing
  JSON would have meant a hand-written encoder per event type, which is a second
  declaration of the event schema in a crate whose whole justification is not
  having one. The queryable facets are columns instead.
- **`events.orphaned_at` is gone.** It marked rows whose slot was rolled back, and
  it could never have been written: only finalised transactions are stored, and
  finalised history does not change. A column nothing can write is a claim about
  behaviour the system does not have — the same defect as an invariant that cannot
  fail. Orphans are a `kind` in `ingestion_anomalies`, which is what they are.
- **`ingestion_anomalies.signature` finally has a writer.** It had been in the
  primary key from the start; the code could not supply it, because `Anomaly`
  carries a log-line index and nothing else. "Line 12 was truncated" with no
  transaction to re-fetch is a statistic, not a report.

`schema.sql` is `include_str!`d by `Store::migrate`, so the file that documents the
schema is the file that creates it.

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

## Reorgs

Confirmed is not final. A slot the cluster has already served can be rolled back and
replaced, so an indexer that folds every transaction straight into one projection has no
way to un-fold the ones that turn out never to have happened — and it will be wrong
quietly, because nothing about the arithmetic looks unusual afterwards.

Two projections and a replay buffer:

```text
  finalized  ── state through the cluster's finalized slot; never rewound
  pending    ── transactions above it, kept in order so they can be replayed
  head       ── finalized + pending, which is what queries read
```

Every poll re-reads the whole unfinalised range rather than asking for a diff. That is the
point: a rollback then shows up as the source *disagreeing* with the buffer, which can be
detected, rather than as a transaction simply never being mentioned again, which cannot.

On disagreement, `head` is rebuilt from `finalized` and the source's current view replayed
over it. Rebuilding rather than reversing is deliberate — inverting an arbitrary fold
needs every projection field to have an inverse, and `saturating_sub` does not.

A contradiction *below* the finality watermark is refused outright rather than treated as
a reorg. Finalised history does not change, so a source that reports otherwise is either
lying or serving a different ledger, and every number downstream is suspect.

**What this cannot do:** a source that silently omits a transaction — an RPC provider
dropping a log, not a rollback — is indistinguishable from that transaction never
existing. The defence is `orphaned`: a later event referring to an entity that was never
created is the symptom, and it is reported.

The reorg path is exercised over real program output too, not only synthetic events:
`the_ingestor_survives_a_reorg_and_still_matches_the_chain` captures logs from the real
BPF programs, rolls back two slots, and checks the rebuilt projection.

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
recomputation is an incomplete event.** Applied across all 38, one other failed it:
`RealmInitialized` announced a realm without `min_weight_to_propose`, so the field
was unlearnable until the first `RealmParamsUpdated` — an update that may never
happen. It carries the field now.

A second, found by a test bug rather than by design: `treasury_balance` returned 0
for a treasury whose deposits predated the indexer, with nothing marking the
figure as computed from partial history. A dashboard would have shown an empty
treasury with complete confidence. Entities materialised by a non-creation event
are now recorded in `orphaned`.

## Layout

| Module | Responsibility |
|---|---|
| [`event.rs`](./src/event.rs) | The 38 event types, and decoding one from its wire form |
| [`logs.rs`](./src/logs.rs) | Attributing `Program data:` lines to the invocation that emitted them |
| [`projection.rs`](./src/projection.rs) | Folding events into queryable state, exactly once each |
| [`source.rs`](./src/source.rs) | The `LogSource` trait, and a scripted source that can roll a slot back on demand |
| [`ingest.rs`](./src/ingest.rs) | Driving a source into the projection, safely across reorgs |
| [`rpc.rs`](./src/rpc.rs) | The one module that opens a socket, behind the `rpc` feature |
| [`api.rs`](./src/api.rs) | The read model — pure functions from a projection to serialisable views |
| [`server.rs`](./src/server.rs) | HTTP transport, behind the `server` feature |
| [`store.rs`](./src/store.rs) | Persisting what finalised, behind the `postgres` feature |
| [`sql/schema.sql`](./sql/schema.sql) | The DDL `Store::migrate` executes, by `include_str!` |

## The read API

```bash
cargo run -p helix-indexer --features server --bin helix-api
```

```text
GET /health
GET /pools/{address}[?finality=head]
GET /pools/{address}/stakers[?limit=50]
GET /realms/{address}/proposals
GET /treasuries/{address}
```

It serves whatever projection it is given. `helix-index --database-url` is what puts
something in one across a restart; without it a fresh process starts empty and says so on
startup, because a demo that looks live and is not is worse than one that admits it.

Three decisions shape every response, each because the obvious alternative is quietly
wrong.

**Finality is part of the answer.** Every response carries the projection it came from and
the slot it reflects:

```json
{ "meta": { "finality": "finalized", "slot": 1, "pending_transactions": 0 },
  "data": { "total_staked": "1000", "apr_bps": null, ... } }
```

Serving one view without saying which invites a dashboard to display a TVL that later
decreases for no visible reason. `?finality=head` is opt-in, and an unrecognised value
falls back to `finalized` — a typo should not silently promote a caller to data a fork can
take back.

**Amounts are strings.** JSON numbers are IEEE-754 doubles, exact only below 2^53. Token
amounts are `u64`; for a 9-decimal mint, 2^53 base units is about nine million tokens.
That is reachable, and the failure mode is silent rounding in whatever parses it rather
than an error. `an_amount_past_the_double_precision_limit_survives_a_json_round_trip`
pins it, and asserts the value genuinely does not fit in a double so the test cannot go
vacuous.

**Undefined is `null`, never zero.** A rate over an empty pool is not an infinite APR or a
zero one, it is undefined. Returning `0` puts a plausible, wrong number on a dashboard.
