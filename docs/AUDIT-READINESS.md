# Audit readiness

*ROADMAP 6.3 and 6.4 — self-audit report, and the brief for an external one.*

The purpose of this document is to spend an auditor's time well. It says what has been
verified and by what method, what has not, and — most usefully — **where the remaining
risk probably is**, inferred from where the findings so far actually came from.

Nothing here replaces an external review. A self-audit is written by the engineer who
wrote the code and shares every blind spot that produced the defects; that is not a
disclaimer, it is the reason the section on *how* findings were found matters more than
the count.

---

## 1. Scope

| Program | Lines | Instructions | Holds authority over |
|---|---:|---:|---|
| [`token-manager`](../programs/token-manager) | 1,068 | 10 | The HLX mint authority |
| [`staking`](../programs/staking) | 1,814 | 9 | Stake and reward vaults |
| [`governance`](../programs/governance) | 2,537 | 23 | Nothing transferable |
| [`treasury`](../programs/treasury) | 1,319 | 8 | Protocol funds and vesting streams |
| **Total** | **6,738** | **50** | |

Anchor 1.1.2, Solana 3.1.10, Token-2022. Line counts include unit tests and doc comments,
which are a large share — the programs themselves are smaller than the figure suggests.

**Out of scope for an audit:** [`indexer/`](../indexer), [`ops/`](../ops) and
[`tests/`](../tests) hold no funds and sign nothing — though `ops/` decides what the
bootstrap transaction *contains*, which is worth a read for that reason. They are worth reading as evidence
about the programs, not as attack surface.

**Deliberately not built**, so absence reads as a decision rather than an oversight:
arbitrary-CPI governance, cross-chain bridging, NFT functionality, confidential transfers,
liquid staking derivatives. Reasons in [ROADMAP.md](./ROADMAP.md#explicitly-out-of-scope).

## 2. What state the code is in

| | |
|---|---|
| Tests | 212 — 117 unit, 95 runtime against the real BPF programs |
| Invariants | 58 documented, all verified — [INVARIANTS.md](./INVARIANTS.md) |
| Fuzzing | Stateful, invariants as the oracle, 22 sequences × 150 operations per run |
| Compute | Every instruction measured; worst is 17.9% of the default budget |
| Lints | `clippy -D warnings`, `fmt --check`, `cargo audit`, all clean in CI |
| Deployed | **No.** Nothing has executed against a real cluster |

## 3. Where the findings came from

Eleven findings. The interesting column is the last one.

| ID | Severity | Status | Found by |
|---|---|---|---|
| F-1 | Medium | Open, mitigated | Manual review of the initialisers |
| F-2 | High | Fixed | Writing a runtime test for the solvency guard |
| F-3 | High | Fixed | Reading the build log — the error the exit code hides |
| F-4 | High | Fixed | Stating the coverage gap explicitly, then closing it |
| F-5 | Critical if deployed | Open, Phase 7 | Manual review of deployment posture |
| F-6 | Low | Accepted | Adversarial modelling |
| F-7 | Informational | Fixed | Manual review |
| F-8 | Medium | Fixed | **Attempting to test an instruction** and finding it unreachable |
| F-9 | Low | Fixed | **Writing the deployment runbook** step by step |
| F-10 | High | Fixed | **Stateful fuzzing** |
| F-11 | High | Fixed | **Applying review question 1 below**, to the one instruction never asked it |

Grouped by method:

- **Manual review: 4** (F-1, F-5, F-6, F-7). All are posture or design-trade findings.
  Manual review found no arithmetic or state-machine bug at all — though F-7, the most
  trivial-looking of the four, turned out to have a High hiding in its fix.
- **Trying to exercise the code: 3** (F-2, F-8, F-9). Every one was an
  instruction that could not be reached, or a guard that could never pass. All three are
  invisible to a per-program test suite because each individual piece is correct.
- **Reading tool output nobody reads: 1** (F-3). `anchor build` reports SBF stack-frame
  overflows as `Error:` and exits 0.
- **Fuzzing: 1** (F-10), a High, and the only one found by generated input.
- **Applying a checklist written after the earlier findings: 1** (F-11), also a High.

That last row is the one worth dwelling on. Review question 1 below was written *because*
F-2, F-8 and F-9 turned out to be the same defect. Asking it of every privileged
instruction then immediately produced a fourth instance — `update_realm_params`, whose
signer no transaction could ever produce — and that one was a path to the treasury.
Four of the eleven findings are now the same shape, which says the checklist is worth
running exhaustively rather than opportunistically.

**What that suggests for an external audit.** Reading the code found the things a careful
reader finds — and none of the five bugs. Every one lived in *composition*: between two
correct halves (F-2), between an instruction and the set of transactions that can produce
its signer (F-8, F-9, F-11), and between two guards that each looked sufficient alone
(F-10).

Four of the eleven were the same structural defect — **an instruction gated on a signature
no code path can produce** — which is why two review questions are now standing, and why
the first has been applied to every privileged instruction rather than left as advice:

1. For every privileged instruction, *which concrete transaction produces its signer?*
2. When an authority is transferred, does the recipient also gain every *power* that
   authority carries?
3. For every monotonic counter, *what else reads it?* Added after `close_position`.
   `pool.position_count` is a PDA seed and an electorate boundary as well as a count, and
   the obvious rent-reclamation fix decrements it — which silently reopens F-10. A counter
   whose name suggests "how many exist now" but whose meaning is "how many have ever
   existed" is a trap that costs nothing to check for and a High to miss.

## 4. What an auditor should not need to re-derive

Each of these has evidence attached, so time spent re-checking them is time not spent
elsewhere. They are listed so they can be *spot-checked* rather than reproduced.

| Claim | Evidence |
|---|---|
| Deposits credit the observed vault delta, not the `amount` argument | `staking_transfer_fee.rs`, run against a real fee-bearing mint and mutation-tested |
| No instruction's cost grows with staker or voter count | 64 stakers and 1 staker at equal total weight cost bit-identical compute — [TESTING.md](./TESTING.md#compute-cost) |
| The bootstrap fits one transaction | Measured: 748 bytes, 17 accounts, against the 1232-byte cap |
| The aggregate solvency invariants hold under arbitrary operation order | Fuzz oracle, checked after every operation, over a fee-bearing mint too |
| The event log reconstructs on-chain state | [`indexer_reconciliation.rs`](../tests/integration/tests/indexer_reconciliation.rs) compares the projection to the accounts field by field |
| Only the governance executor can move treasury funds | Runtime negative test per threat-model attack |

Mutation testing was used on the three claims where a passing test proves least — the
transfer-fee accounting, the indexer's CPI attribution, and `close_position` leaving
`pool.position_count` alone. In each case the injected bug turns the targeted tests red
while the tests that *cannot* distinguish the mutation stay green, which is the property
that makes the suite meaningful rather than merely large.

The third is the one to spot-check, because the mutation is the implementation most people
would write: decrementing the counter on close. It reopens
[F-10](./SECURITY-ASSESSMENT.md#f-10--post-snapshot-weight-could-vote) — a High closed two
phases earlier — through a change about reclaiming rent. See
[TESTING.md](./TESTING.md#mutation-testing-and-why-it-is-the-real-check).

## 5. What is still open

| ID | Severity | Why it is still open |
|---|---|---|
| F-5 | Critical **if deployed** | The upgrade authority is unmigrated. Currently theoretical — nothing is deployed — and the dominant risk the moment that changes. Phase 7 |
| F-1 | Medium | Initialisers are front-runnable. Mitigated by the atomic bootstrap, which is measured and tested, and now *detected* by `helix-bootstrap --verify`; the residual window cannot be closed in-program without a deployer gate |
| F-6 | Low | A compromised guardian key can veto every proposal. Accepted: the alternative is a guardian that cannot stop anything |

**Nothing else is open, and the last three invariants closed with a correction attached.**
An earlier version of this section said §5.3 and §5.5 "need a deployment to assert
against". That was wrong. Both are runtime properties, and a runtime test is LiteSVM, not a
cluster — the same distinction this document makes everywhere else, missed on the one row
where it was load-bearing. They were untested because nobody had written them, and the
invariant table meanwhile named tests for both that did not exist.

§5.8 was the more interesting one. It asserted "initialisers cannot install an unintended
authority", which is not a property these programs have and cannot be made into one; it was
F-1 written in the grammar of an invariant, and so it could never fail. It now asserts what
is true and checkable — an unintended authority is detected before anything of value is
deposited — and the suite runs that check against a system whose pool really was front-run.

Worth an auditor's attention as a pattern rather than a finding: an invariant that cannot
fail is indistinguishable from one that always holds, and the table gave no way to tell
them apart.

## 6. Limits of everything above

**Nothing has run on a cluster.** LiteSVM executes the real BPF programs faithfully, but
it is not a validator. Transaction fees, congestion, priority-fee dynamics and reorg
behaviour are entirely unexercised. Anything about how this system behaves under load is
currently unevidenced.

**The fuzzer explores what its generator can reach.** It is stateful and its coverage is
asserted rather than assumed, but the operation mix is hand-designed, and it took three
rounds of measurement before the campaign reached `execute` at all — see
[TESTING.md](./TESTING.md#what-it-took-to-make-the-fuzzer-reach-anything). Whole regions of
the state space are reachable only because someone noticed they were not.

**No formal verification**, and no economic analysis. Whether the lock-tier weights or the
quorum parameters produce sensible governance dynamics is a question this document does not
touch; every claim here is about the code doing what it says, not about what it says being
a good idea.

**Written by the author of the code.** F-2, F-8, F-9, F-10 and F-11 were all found by
building something that had to *use* the programs — a test, a runbook, a fuzzer, a
checklist — rather than by reading them. That is a strong hint about the shape of what remains: an external reviewer
should expect the residue to be in composition and reachability rather than in any single
handler.

## 7. Brief for an external audit

**Ask for:** a full review of the four programs, with emphasis on cross-program authority
flow and the governance state machine.

**Provide:** this document, [SECURITY-ASSESSMENT.md](./SECURITY-ASSESSMENT.md),
[INVARIANTS.md](./INVARIANTS.md) as the property set to attack,
[THREAT-MODEL.md](./THREAT-MODEL.md) for what is already considered, and the test suite as
a statement of what is believed to be true.

**Point them at**, in priority order:

1. **The governance state machine.** Two of the four real bugs were here, including the
   only one fuzzing found. It has the most states, the most clock dependencies, and the
   most ways for two individually-correct guards to leave a gap between them.
2. **The reward accumulator's rounding.** Every division truncates toward the pool by
   design, and the liability estimate deliberately over-states debt. The directions are
   tested individually; whether they compose to a bound that holds under every ordering is
   exactly the kind of question a fresh reader is better placed to answer.
3. **Reachability of every privileged instruction.** Three findings of the same shape
   already. Ask specifically for the two review questions in §3 to be applied
   exhaustively.
4. **Token-2022 extension interaction.** Transfer fees are handled and tested. Other
   extensions — transfer hooks especially — are not, and a mint carrying one has not been
   considered.

**Do not spend the budget on:** re-deriving the compute characteristics, the bootstrap
transaction size, or the fee accounting. Those are measured, and §4 lists where.
