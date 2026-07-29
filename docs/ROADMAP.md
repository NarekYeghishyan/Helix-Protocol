# Technical roadmap

*Deliverable 3 — feature prioritisation, architecture recommendations, milestones,
timeline estimates.*

This is both the plan for Helix and the honest inventory of what is not yet built.
Anything not listed as **Done** is not done, however confidently the rest of the
documentation describes the design.

Estimates are in engineer-days for one experienced Solana engineer, and assume the
toolchain is already set up. They are estimates, not commitments; the confidence column
says how much to trust each one.

---

## Status summary

| Phase | Scope | Status |
|-------|-------|--------|
| 0 | Toolchain, workspace, CI | ✅ Done |
| 1 | Four programs, unit-tested | ✅ Done |
| 2 | Integration tests against a validator | ✅ Done — 74 runtime tests; found and fixed F-8 |
| 3 | Devnet deployment + verifiable builds | ◐ Bootstrap measured, planned, *verifiable after the fact*, and now submitted to a real cluster; F-9 fixed; the devnet deploy itself is blocked on funding |
| 4 | Indexer + analytics API | ✅ Done — decode, ingestion, the RPC source, the read API and the Postgres binding, each tested against the real thing |
| 5 | Dashboard + wallet integration | ◐ Analytics views, wallet connection and cluster switching built; the write flows are the next phase |
| 6 | Fuzzing + external audit prep | ✅ Done — compute benchmarked, fuzzing found F-10, audit scoped in [AUDIT-READINESS.md](./AUDIT-READINESS.md) |
| 7 | Governance migration (burn the admin keys) | ◐ 7.1 and 7.2 reachable and tested — F-11 fixed; 7.3 needs a deployment |

---

## Prioritisation, and why in this order

The ordering follows one rule: **retire the largest unknown next**, not the most
visible feature.

That unknown used to be the cross-program wiring, which had only been proven to
*compile*. It is now executed end to end by 98 runtime tests and driven in random order by
a fuzzer, and along the way it produced F-8, F-9, F-10 and F-11 — so the rule paid for
itself and that particular unknown is retired.

Ingestion was the next one, and it is now retired too. The reorg half was always testable
offline and was done first; the transport half needed a cluster, which turned out not to
need devnet — `solana-test-validator` is a real RPC endpoint that airdrops without limit,
so the only thing the faucet was ever blocking was *devnet specifically*. The programs are
deployed to one, the bootstrap has been submitted to one, and the projection is compared
against accounts read back over JSON-RPC. What a local validator still cannot do is fork,
which is why reorg handling stays on a scripted source.

Persistence was the next one, and it is retired too. It also produced the phase's most
uncomfortable finding, which had nothing to do with databases: the indexer could not decode
`RealmParamsUpdated` or `RealmAuthorityChanged` — the two events that record governance
becoming self-governing, added to the program in Phase 7.1 and never added to the decoder.
On a live chain both would have arrived as "I do not recognise this". `event.rs` argues that
using the programs' own structs makes a changed field a compile error, and that is true; it
says nothing about a *new type*, because nothing in Rust enumerates the types in a module.
That hole is closed by checking the list against the IDLs `anchor build` generates, which
cannot drift from the programs the way a hand-written list can.

**The largest unknown now is the write path from a browser.** Every flow that spends,
stakes or votes has been driven from Rust and never from a wallet, and the failure modes
there — a simulation that disagrees with the signed transaction, an Anchor error code
rendered as `0x1771`, a stale blockhash — are not reachable from the test suite. Phase 5.2
and 5.3 are now unblocked: they were waiting on a deployment, and a local validator is one.

A dashboard is also what a stakeholder can see, so finishing it now happens to be both the
visible choice and the correct one, which has not been true at any earlier point in this
roadmap.

---

## Phase 2 — Integration tests

*≈4–6 days, high confidence*

The top priority. Everything after this depends on it.

| Milestone | Deliverable | Est. | Status |
|---|---|---|---|
| 2.1 | Harness: LiteSVM fixtures, mint/token-account/clock-warp helpers, PDA derivations | 1d | ✅ Done |
| 2.2 | Staking deposit path and weight derivation | 1d | ✅ Done |
| 2.3 | **Fee-bearing mint path** — staking re-run against a Token-2022 transfer-fee mint | 1d | ✅ Done |
| 2.4 | Governance end-to-end: create → activate → vote → finalize → queue → execute, incl. a treasury spend actually landing | 1–2d | ✅ Done |
| 2.5 | Negative tests: the attacks in [THREAT-MODEL.md](./THREAT-MODEL.md) must each fail | 1d | ✅ Done |
| 2.2b | Staking lifecycle: accrue → claim → unlock → unstake, with clock warping | 1d | ✅ Done |
| 2.6 | **Fix F-8** — add the missing `ProposalAction` variants so vesting, spend-cap and executor migration are reachable at all | 0.5d | ✅ Done |
| 2.7 | Vesting runtime: create → cliff → claim → revoke, with token movement | 1d | ✅ Done |

**2.3 was the one that mattered most, and it is done.** The programs credit the observed
vault delta rather than the `amount` argument — correct, but on a plain SPL mint the two
are identical, so every unit test passed either way. It is now verified against a real
fee-bearing mint and mutation-tested: injecting the bug fails three tests with a
30,000-unit vault shortfall, while the plain-mint test stays green.

**2.4 is done**, and it verified the central claim of the architecture: a passed,
timelocked proposal moves treasury funds, and nothing else can.

**2.6 exists because writing these tests found a real hole.** There was no way to
construct a transaction that creates a vesting stream — governance has no
`ProposalAction` variant that produces the executor signature for it. Vesting, the spend
cap and executor migration are all currently unreachable on chain
([F-8](./SECURITY-ASSESSMENT.md#f-8--governance-gated-treasury-instructions-are-unreachable)).
Every unit test passed and the code compiled; the gap was in what governance is *able to
ask for*.

**2.5 defines "done" for security work.** A threat model whose defences have no failing
test is a document, not a control.

**Recommendation:** adopt LiteSVM for speed (millisecond tests, no validator process)
and keep a smaller Surfpool suite for anything touching real Token-2022 extension
behaviour. Time-dependent tests need clock warping, which is why unit tests take `now`
as a parameter rather than reading `Clock` directly — that decision was made for this
phase.

## Phase 3 — Devnet deployment

*≈2–3 days, high confidence*

| Milestone | Deliverable | Est. |
|---|---|---|
| 3.0 | **Measure** whether the atomic bootstrap fits one transaction | 0.5d | ✅ Done — 748 B / 17 accounts |
| 3.1 | Fund a devnet keypair (~20 SOL peak) and deploy all four programs | 0.5d | ⬜ blocked on faucet |
| 3.2 | Bootstrap planner driving the measured single transaction | 1d | ✅ Done — [`ops/`](../ops); submission still needs a funded key |
| 3.3 | Verifiable builds (`solana-verify`), so deployed bytecode reproduces from source | 0.5d | ⬜ |
| 3.4 | Upgrade authority to a 3-of-5 Squads multisig | 0.5d | ⬜ |
| 3.5 | **Fix F-9** — the whole token-manager admin surface, so the last authority can migrate | 0.5d | ✅ Done |
| 3.6 | **Post-deploy authority audit** — `--verify` as a command with an exit code, not a checklist line | 0.5d | ✅ Done — closes invariant §5.8 |

3.0 turned a recommendation into a fact and improved the design. Because
`initialize_pool` and `initialize_treasury` take their privileged party as an argument,
the bootstrap can name the realm's executor PDA at initialisation — there is never a
moment when a human key controls emissions or the treasury, and the two-step handover this
roadmap previously assumed for them is unnecessary.

Writing that sequence down exposed
[F-9](./SECURITY-ASSESSMENT.md#f-9--token-manager-admin-cannot-be-handed-to-governance):
the token-manager admin *must* start as a human key (only an admin can register the first
minter), and there is no `ProposalAction` that lets governance accept the handover. Same
defect as F-8, found the same way.

**3.2 is a planner, not a script, and it is tested by being executed.** The bootstrap is a
one-shot transaction against an open front-running window — there is no rehearsal, and a
wrong account is discovered on mainnet at the one moment an attacker is watching for it.
So the instruction set lives in [`ops/`](../ops) as a library, `helix-bootstrap` prints it,
and [`bootstrap_atomicity.rs`](../tests/integration/tests/bootstrap_atomicity.rs) executes
**that same function** against the real BPF programs. The plan an operator reads is the
plan the suite has run, rather than a second implementation that resembles it.

The tool reports the transaction size against the 1232-byte cap, every derived address, and
who will hold each authority afterwards — and **refuses to emit anything** if an authority
would be a key rather than the executor PDA. A separate test asserts the size it reports is
the size that actually gets sent, because a reassuring wrong number is worse than none.

It does not submit. `solana-rpc-client` is at 4.2.0-rc while this workspace resolves the
Solana crates at 3.x, and the graph already carries eight duplicated `solana-*` crates;
adding a second major version of the SDK to send one transaction is the trade that ruled
out Trident. `--json` emits the instructions for whatever client the operator already has.

**It has now been sent, though not to devnet.** `rpc_source_live.rs` bootstraps against a
local validator using `helix_ops::plan` — the same function `helix-bootstrap` prints — so
the transaction has been through preflight, a mempool and confirmation rather than only
through LiteSVM. That is not the same as a devnet deploy, and 3.1 stays open; what it
retires is the possibility that the plan only works in-process.

**3.6 is the half that was missing, and it is the half that answers a different question.**
Everything in 3.2 is *what will happen*. `--verify` is *what did* — it takes the four
authorities as read off the chain and names each one that is not what the plan said, exit
code 1. (1) and (2) in F-1 remove the window and the vulnerability; neither tells an
operator whether what they actually did worked, and a mitigation you cannot confirm
afterwards is an assumption. Invariant §5.8 used to assert "initialisers cannot install an
unintended authority", which is simply false of these programs; it now asserts the thing
that is true and can fail, and the suite runs it against a system where the pool really
was front-run.

**Cost note.** The four programs total 1.43 MB, ~9.94 SOL of rent-exemption, and
deployment needs roughly double that at peak for the buffers. Devnet's CLI airdrop is
capped at 2 SOL and rate-limits hard, so this needs
[faucet.solana.com](https://faucet.solana.com) rather than `solana airdrop`.

**What is blocked is devnet, not deployment.** All four programs deploy cleanly to a local
validator under the upgradeable loader at the IDs in `Anchor.toml`, and the live tests run
against them there. So the loader path, the program IDs, the bootstrap transaction and the
indexer are all exercised; what devnet adds is a shared cluster others can point at, real
rent, and a real upgrade authority to migrate in 7.3. That is worth doing and it is not
where the risk was.

## Phase 4 — Indexer and analytics API

*≈4–5 days, medium confidence*

Every state transition emits an event carrying an on-chain timestamp, so history is
reconstructable without polling account state. The decoding half is built and verified;
nothing yet talks to a cluster or a database.

| Milestone | Deliverable | Est. | Status |
|---|---|---|---|
| 4.0 | Event decoding and log attribution, reconciled against the chain | 1.5d | ✅ Done |
| 4.1 | Event subscriber over RPC logs, with reorg handling | 1.5d | ✅ Done — [`rpc.rs`](../indexer/src/rpc.rs) + `helix-index`, verified against a validator |
| 4.2 | Postgres schema + idempotent upserts keyed on `(signature, log_index)` | 1d | ✅ Done — [`store.rs`](../indexer/src/store.rs), 9 tests against a real database |
| 4.3 | Backfill from genesis slot, resumable | 1d | ◐ Paging and cursor resumption tested against a live cluster and durable across a restart; the second, downward cursor the schema allows is unwritten |
| 4.4 | Read API: TVL, APR, staker distribution, proposal history, treasury flows | 1d | ✅ Done — [`api.rs`](../indexer/src/api.rs) + an axum transport behind the `server` feature |

**4.0 is deliberately the half that can be verified without a cluster.** Ingestion cannot
be tested offline; decoding and folding are where the bugs that corrupt analytics
actually live. [`indexer/`](../indexer) therefore contains no RPC client and no database
driver, and [`indexer_reconciliation.rs`](../tests/integration/tests/indexer_reconciliation.rs)
runs real transactions, captures the runtime's own logs, and compares the resulting
projection to the accounts those transactions wrote — field by field.

It found two things. `Unstaked` did not carry the position's remaining vote weight, so
`pool.total_weighted` could only be reconstructed by re-running the tier table off chain —
a second implementation that agrees with the program until the table changes. And
`treasury_balance` silently returned 0 for a treasury whose deposits predated the indexer,
with nothing marking the figure as computed from partial history. Both fixed; see
[W-8](./ARCHITECTURE-REVIEW.md#weaknesses).

**4.1 and 4.3 are split the same way 4.0 was**, and for the same reason: the parts that
cannot be tested without a cluster are separated from the parts where the bugs live.
[`source.rs`](../indexer/src/source.rs) is a `LogSource` trait. Everything that *decides
what to do* with what a source returns — hold the unfinalised tail, detect that it has
been replaced, rebuild, promote to final, advance the cursor — is in
[`ingest.rs`](../indexer/src/ingest.rs) and is driven by a scripted source that can roll a
slot back on demand. Devnet cannot be asked to fork; a fake can.

**The RPC half is now written, and it speaks JSON-RPC rather than using an SDK client.**
`solana-rpc-client` is at 4.2.0-rc while this workspace resolves the Solana crates at 3.x
— the same version split that ruled out Trident — and the whole surface an indexer needs
is three read methods carrying no signatures and no account decoding. It lives behind an
`rpc` feature that is off by default, so the crate's default build still has no socket in
it and `cargo test --workspace` still does not build a TLS stack to test a fold.

It is verified by [`rpc_source_live.rs`](../tests/integration/tests/rpc_source_live.rs)
against a validator with the four programs actually deployed. That mattered more than
expected: **three of its six tests passed while testing nothing**, and only mutation
testing said so — the details are in
[TESTING.md](./TESTING.md#the-live-tests-and-the-three-that-were-not-testing-their-claims),
including a fourth mutation that survives because measuring the node showed the hazard was
not real.

The design is two projections and a replay buffer. `finalized` never rewinds, `head` is
what queries read, and a reorg rebuilds `head` from `finalized` rather than trying to
reverse the fold — inverting an arbitrary projection needs every field to have an inverse,
and `saturating_sub` does not. A contradiction *below* the finality watermark is not a
reorg and is refused outright: finalised history does not change, so either the source is
lying or the stored cursor belongs to another ledger.

**Recommendation, now acted on:** make ingestion idempotent from the start and treat
every event as possibly-redelivered. Confirmed-commitment logs can still be rolled back,
and an indexer that assumes exactly-once delivery reports wrong numbers precisely when
something has gone wrong elsewhere. The projection is keyed on `(signature, log_index)`,
and events carry running totals rather than deltas so that assignment — which is
replay-safe — beats accumulation, which is not.

**4.4 answers with its own uncertainty attached.** Three decisions shape the responses,
each because the obvious alternative is quietly wrong:

- **Finality is part of the answer.** Every response names which projection it came from
  and the slot it reflects. An API serving one without saying which invites a dashboard to
  show a TVL that later drops for no visible reason. `?finality=head` is opt-in;
  the default is the view that never gets revised.
- **Amounts are strings.** JSON numbers are IEEE-754 doubles, exact below 2^53. Token
  amounts are `u64`, and for a 9-decimal mint 2^53 base units is about nine million
  tokens — reachable, and the failure is silent rounding in the browser rather than an
  error. Same hazard `schema.sql` avoids with `NUMERIC(20, 0)` over `BIGINT`.
- **Undefined is `null`.** APR over an empty pool is undefined, not zero. A plausible
  wrong number on a dashboard is worse than a gap.

The split is the same one used twice already: the read model is pure and always compiled,
the axum wiring is behind a `server` feature so `cargo test --workspace` does not pay for
an async runtime to test functions with no I/O. Routing that contains logic is routing
nobody tests.

**4.2 is where the schema stopped being a design and started being code.** Three things in
it did not survive execution, and all three are the same kind of error — a column that
reads well and can never be written:

- `payload JSONB` became `BYTEA`. The table's stated purpose is that everything else is
  derivable from it, and derivable means decodable by `HelixEvent::decode`. JSON would have
  needed a hand-written encoder per event type — a second declaration of the event schema,
  in the one crate whose entire justification is not having one.
- `events.orphaned_at` is gone. It marked rows whose slot was rolled back, and nothing can
  write it: only finalised transactions are stored, and finalised history does not change.
  Same defect as invariant §5.8 before it was corrected — a claim that cannot fail.
- `ingestion_anomalies.signature` finally has a writer. It had been in the primary key from
  the start while `Anomaly` carried only a log-line index, so the code could not supply it.
  "Line 12 was truncated" with no transaction to re-fetch is a statistic, not a report.

The property that took the most thought is the one that is not about SQL at all. The
persisted rows carry the *result* of folding an event and not the fact that it was folded —
and for `Staked`, `Unstaked`, `RewardsClaimed` and `StreamClaimed` that fact is the only
thing making replay safe, because those four publish a delta and no running total to
assign. A projection restored without its applied set double-counts the first redelivery
after a restart, silently, and only for the transactions straddling it. `load` restores
that set for every event at or above the cursor's slot, which is provably the whole range a
source can serve again.

**And the phase found a decoding gap that had nothing to do with storage.** The indexer did
not know `RealmParamsUpdated` or `RealmAuthorityChanged`, added to governance in 7.1 — the
two events recording that governance now answers only to itself. Both would have arrived as
`Anomaly::UndecodableData`. `event_coverage.rs` now compares the decoder's list against the
IDLs, so the class cannot recur.

Medium confidence on 4.3's remainder because it depends on RPC provider behaviour (log
retention, webhook reliability) that is hard to predict from outside.

## Phase 5 — Dashboard

*≈5–7 days, medium confidence*

| Milestone | Deliverable | Est. | Status |
|---|---|---|---|
| 5.1 | Next.js app, wallet-adapter, cluster switching | 1d | ✅ Done |
| 5.2 | Stake / unstake / claim flows with simulation before signing | 2d | ⬜ **Now the priority** — unblocked by the local validator |
| 5.3 | Governance UI: proposal list, vote, lifecycle state, timelock countdown | 2d | ◐ Proposal list built; voting is next |
| 5.4 | Analytics views over the Phase 4 API | 1–2d | ✅ Done |

**5.1 and 5.4 are the half that does not need a chain**, and they were built first for that
reason. The write flows were blocked on a deployment; they no longer are, because
`solana-test-validator` is one. What is left is the part of the system that has never been
driven from a wallet, which is now the largest unknown in the project.

Two things the UI does that most dashboards do not, both inherited from decisions the API
already made:

- **Finality is on screen.** Every panel shows which projection it is reading, the slot,
  and how many of its transactions a fork could still take back. A dashboard that silently
  served `head` shows a TVL that sometimes decreases for no visible reason.
- **Three kinds of nothing look different.** "The indexer is not answering", "the indexer
  has never seen this address" and "this pool genuinely has no stakers" are distinct
  states. Rendering all three as an empty table is lying by omission — and right now
  every panel is in one of the first two, which is exactly when it matters.

Amounts are formatted through `BigInt`, never a JavaScript number: the API sends strings
precisely because `u64` exceeds what a JSON number represents exactly, and `Number(...)` on
the client throws that away. Writing the test for it found a real bug — `BigInt("")` is
`0n` rather than a throw, so a missing field rendered as a confident zero.

**Recommendation:** simulate every transaction before presenting it for signature, and
surface the decoded Anchor error rather than a raw code. The specific error enums exist
so the UI can say "position is still locked" instead of "custom program error: 0x1771".

## Phase 6 — Fuzzing and audit prep

*≈4–6 days, low confidence*

| Milestone | Deliverable | Est. | Status |
|---|---|---|---|
| 6.0 | **Compute benchmarks** against staker and voter count, to measure invariant §6.3 rather than argue it | 1d | ✅ Done |
| 6.1 | Stateful fuzzing over staking and governance | 2d | ✅ Done — **not** with Trident, see below |
| 6.2 | Invariant harness: assert every [INVARIANTS.md](./INVARIANTS.md) property after each fuzz step | 1.5d | ✅ Done — found [F-10](./SECURITY-ASSESSMENT.md#f-10--post-snapshot-weight-could-vote) |
| 6.3 | Self-audit report; resolve findings | 1–2d | ✅ Done — [AUDIT-READINESS.md](./AUDIT-READINESS.md) |
| 6.4 | Scope and brief an external audit | 0.5d | ✅ Done — brief in the same document |

6.0 was pulled forward out of order because it needed nothing that Phase 3 is blocked on.
It confirmed §6.3 and produced the compute table in
[TESTING.md](./TESTING.md#compute-cost): every instruction has better than 4× headroom
against the default budget, and reaching the same staked total with 64 stakers or with one
costs bit-identical compute. Most of the work was not the measurement but identifying what
made an earlier draft of it unreproducible — PDA bump derivation, at 1,500 CU an attempt,
varying with a randomly generated mint.

**6.1 does not use Trident, and the reason is checkable.** Its newest release
(0.13.0-rc.4) pins `solana-sdk ^2.3`; `anchor-lang` 1.1.2 resolves the Solana crates at
3.x. Adding it would put two major versions of the SDK in one dependency graph — the same
breakage that already forces `litesvm` to `=0.13.1`. The equivalent is built on LiteSVM
instead, in [`fuzz.rs`](../tests/integration/src/fuzz.rs): a seeded generator, an oracle
that reads every aggregate invariant out of the accounts after **every** operation, and a
delta-debugging shrinker. Re-check the pin when Trident moves; it is a one-line change.

**6.2 paid for itself.** §4.3 (`for + against + abstain <= total_weight_snapshot`) had sat
at ◐ — reasoned about, never asserted over real accounts. The oracle asserted it after
every step and found weight staked *after* activation voting anyway, inflating the quorum
numerator against a fixed denominator. That is
[F-10](./SECURITY-ASSESSMENT.md#f-10--post-snapshot-weight-could-vote), High severity,
fixed. Every scripted governance test had staked its voters before activating, because that
is the order a person writes; the generator had no such habit.

Most of the work was not writing the fuzzer but making it reach anything. The governance
lifecycle is six ordered, clock-gated steps, and a uniform generator spent its whole budget
bouncing off `InvalidProposalState`: the first measured campaign activated 18 proposals,
cast 7 votes and executed none. Getting to `execute` took a state-aware generator, an
eligibility-aware voter selection, and a sequence length chosen by measuring the funnel
rather than by taste — all recorded in
[TESTING.md](./TESTING.md#what-it-took-to-make-the-fuzzer-reach-anything).

Low confidence on the rest: fuzzing finds what it finds. The 1–2 days for 6.3 assumes the
findings are shallow, which is exactly the assumption fuzzing exists to test — and the
first campaign already returned a High.

## Phase 7 — Governance migration

*≈2 days, high confidence*

The point of the whole design, and easy to leave undone forever.

| Milestone | Deliverable | Est. | Status |
|---|---|---|---|
| 7.1 | Realm authority → the realm's own executor PDA, so parameter changes need a vote | 0.5d | ✅ Reachable and tested; the migration itself runs at deploy |
| 7.2 | Token-manager admin → governance | 0.5d | ✅ Reachable and tested (F-9) |
| 7.3 | Program upgrade authority → governance, or burned | 1d | ⬜ Needs a deployment |

**7.1 was not a migration, it was a missing instruction.** The realm authority could not
be moved at all: `update_realm_params` is gated on `realm.authority`, and no
`ProposalAction` produced that signature. So the parameters defining what "passing" means
belonged permanently to whoever initialised the realm — and lowering quorum to the 0.01%
floor turns a dust position into a treasury transfer.
[F-11](./SECURITY-ASSESSMENT.md#f-11--the-rules-of-governance-were-owned-from-outside-it),
High, fixed with `UpdateRealmParams` and `SetRealmAuthority`.

7.1 and 7.2 are now *possible* and proven by runtime tests. Neither is *done* in the sense
that matters, because both migrate an authority on a chain nothing is deployed to. The
runbook runs them; the post-deploy checklist asserts the result.

Until 7.3, "decentralised" is aspirational: whoever holds the upgrade authority can
replace every guarantee in this repository. An unmigrated upgrade authority is the most
common gap between "audited" and "actually safe", and [RUNBOOK.md](./RUNBOOK.md) treats
it as an explicit, verifiable step rather than something assumed to have happened.

---

## Explicitly out of scope

Named so that absence reads as a decision rather than an oversight:

- **NFT functionality.** No compelling reason for this protocol to mint NFTs; a
  membership or receipt NFT would be a real feature, not a bolt-on.
- **Cross-chain bridging.** Bridges are the single largest source of losses in the
  space; adding one would dominate the threat model.
- **Arbitrary-CPI governance.** A deliberate design choice, not a gap — see
  [governance/README.md](../programs/governance/README.md#actions-are-a-closed-set).
- **Confidential transfers.** Token-2022 supports them; they would break the analytics
  the dashboard exists to provide.
- **Liquid staking derivatives.** Would require rethinking the lock-gated vote weight
  that flash-loan resistance depends on, since a transferable receipt token
  reintroduces exactly the rentable voting power the design removes.

## Known technical debt

| Item | Impact | When |
|---|---|---|
| ~~No integration tests~~ | ~~Cross-program wiring unverified at runtime~~ | Done — Phase 2, 98 runtime tests |
| ~~`Position` accounts are never closed~~ | ~~Rent is not reclaimed on full exit~~ | Done — `close_position`, [F-7](./SECURITY-ASSESSMENT.md#f-7--position-accounts-never-closed) |
| Token metadata not initialised | Mint has no on-chain name/symbol; needs the Token-2022 metadata extension CPI plus a realloc for variable-length fields | Phase 3 |
| Single reward mint per pool | Multi-reward pools need a per-reward accumulator | Deferred; no demand |

The `Position` row is worth a second look before it is forgotten, because the fix was not
the boring one it looked like. Reclaiming the rent means deallocating an account whose
address is seeded by `pool.position_count` — the same counter governance snapshots to
decide who was in the electorate. The obvious implementation decrements it and thereby
reopens [F-10](./SECURITY-ASSESSMENT.md#f-10--post-snapshot-weight-could-vote), a High that
a previous phase had closed. Two items on a technical-debt list can interact; nothing on
the list says which.
