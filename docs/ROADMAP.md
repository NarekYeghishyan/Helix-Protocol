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
| 2 | Integration tests against a validator | 🟡 In progress — authority chain + Token-2022 verified; staking withdrawal and vesting remain |
| 3 | Devnet deployment + verifiable builds | ⬜ Not started |
| 4 | Indexer + analytics API | ⬜ Not started |
| 5 | Dashboard + wallet integration | ⬜ Not started |
| 6 | Fuzzing + external audit prep | ⬜ Not started |
| 7 | Governance migration (burn the admin keys) | ⬜ Not started |

---

## Prioritisation, and why in this order

The ordering follows one rule: **retire the largest unknown next**, not the most
visible feature.

Right now the largest unknown is not a missing feature, it is that the cross-program
wiring has only been proven to *compile*. The reward maths, the vesting schedule and
the tally arithmetic are tested directly, but "governance's executor PDA can actually
sign a treasury spend" is currently an assertion about types, not an observed fact.
Building a dashboard before proving that would mean building UI on top of an unverified
protocol — and if the wiring is wrong, some of it has to be rebuilt.

A dashboard is what a stakeholder can see, so there is real pressure to build it early.
It is the wrong call, and the reason is worth stating plainly: a demo over unproven
programs creates confidence that the system does not yet deserve.

---

## Phase 2 — Integration tests *(≈4–6 days, high confidence)*

The top priority. Everything after this depends on it.

| Milestone | Deliverable | Est. | Status |
|---|---|---|---|
| 2.1 | Harness: LiteSVM fixtures, mint/token-account/clock-warp helpers, PDA derivations | 1d | ✅ Done |
| 2.2 | Staking deposit path and weight derivation | 1d | ✅ Done |
| 2.3 | **Fee-bearing mint path** — staking re-run against a Token-2022 transfer-fee mint | 1d | ✅ Done |
| 2.4 | Governance end-to-end: create → activate → vote → finalize → queue → execute, incl. a treasury spend actually landing | 1–2d | ✅ Done |
| 2.5 | Negative tests: the attacks in [THREAT-MODEL.md](./THREAT-MODEL.md) must each fail | 1d | ✅ Done |
| 2.2b | Staking lifecycle: accrue → claim → unlock → unstake, with clock warping | 1d | ✅ Done |
| 2.6 | **Fix F-8** — add the missing `ProposalAction` variants so vesting, spend-cap and executor migration are reachable at all | 0.5d | ⬜ **next** |
| 2.7 | Vesting runtime: create → cliff → claim → revoke, with token movement | 1d | ⬜ blocked on 2.6 |

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

## Phase 3 — Devnet deployment *(≈2–3 days, high confidence)*

| Milestone | Deliverable | Est. |
|---|---|---|
| 3.1 | Deploy all four programs to devnet; record IDs in [RUNBOOK.md](./RUNBOOK.md) | 0.5d |
| 3.2 | Idempotent bootstrap script: mint → pool → realm → treasury, correctly wired | 1d |
| 3.3 | Verifiable builds (`solana-verify`), so deployed bytecode reproduces from source | 0.5d |
| 3.4 | Upgrade authority to a 3-of-5 Squads multisig | 0.5d |

3.2 is where the authority wiring becomes real: the staking pool's authority must
become the governance executor PDA, and the treasury's `governance_executor` must match.
Both are two-step handovers by design, so the script has to drive both halves.

## Phase 4 — Indexer and analytics API *(≈4–5 days, medium confidence)*

Every state transition already emits an event carrying an on-chain timestamp, so history
is reconstructable without polling account state. Nothing is built yet.

| Milestone | Deliverable | Est. |
|---|---|---|
| 4.1 | Event subscriber (Anchor `EventParser` over logs), with reorg handling | 1.5d |
| 4.2 | Postgres schema + idempotent upserts keyed on `(signature, log_index)` | 1d |
| 4.3 | Backfill from genesis slot, resumable | 1d |
| 4.4 | Read API: TVL, APR, staker distribution, proposal history, treasury flows | 1d |

**Recommendation:** make ingestion idempotent from the start and treat every event as
possibly-redelivered. Confirmed-commitment logs can still be rolled back, and an indexer
that assumes exactly-once delivery reports wrong numbers precisely when something has
gone wrong elsewhere.

Medium confidence because it depends on RPC provider behaviour (log retention, webhook
reliability) that is hard to predict from outside.

## Phase 5 — Dashboard *(≈5–7 days, medium confidence)*

| Milestone | Deliverable | Est. |
|---|---|---|
| 5.1 | Next.js app, wallet-adapter, cluster switching | 1d |
| 5.2 | Stake / unstake / claim flows with simulation before signing | 2d |
| 5.3 | Governance UI: proposal list, vote, lifecycle state, timelock countdown | 2d |
| 5.4 | Analytics views over the Phase 4 API | 1–2d |

**Recommendation:** simulate every transaction before presenting it for signature, and
surface the decoded Anchor error rather than a raw code. The specific error enums exist
so the UI can say "position is still locked" instead of "custom program error: 0x1771".

## Phase 6 — Fuzzing and audit prep *(≈4–6 days, low confidence)*

| Milestone | Deliverable | Est. |
|---|---|---|
| 6.1 | Trident stateful fuzzing over staking and governance | 2d |
| 6.2 | Invariant harness: assert every [INVARIANTS.md](./INVARIANTS.md) property after each fuzz step | 1.5d |
| 6.3 | Self-audit report; resolve findings | 1–2d |
| 6.4 | Scope and brief an external audit | 0.5d |

Low confidence: fuzzing finds what it finds. The 1–2 days for 6.3 assumes the findings
are shallow, which is exactly the assumption fuzzing exists to test.

## Phase 7 — Governance migration *(≈2 days, high confidence)*

The point of the whole design, and easy to leave undone forever.

| Milestone | Deliverable | Est. |
|---|---|---|
| 7.1 | Realm authority → the realm's own executor PDA, so parameter changes need a vote | 0.5d |
| 7.2 | Token-manager admin → governance | 0.5d |
| 7.3 | Program upgrade authority → governance, or burned | 1d |

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
| No integration tests | Cross-program wiring unverified at runtime | Phase 2 |
| Token metadata not initialised | Mint has no on-chain name/symbol; needs the Token-2022 metadata extension CPI plus a realloc for variable-length fields | Phase 3 |
| `Position` accounts are never closed | Rent is not reclaimed on full exit | Phase 2 |
| No compute-unit benchmarks | Invariant §6.3 (flat compute vs. staker count) is argued from code structure, not measured | Phase 6 |
| Single reward mint per pool | Multi-reward pools need a per-reward accumulator | Deferred; no demand |
