# Invariants

Properties that must hold after **every** instruction, in every state.

Each row names the test that asserts it and whether that test **exists yet**. The status
column is the honest part of this document: most aggregate invariants can only be checked
against real accounts, so they are unreachable by unit tests and currently unverified.
An invariant with no test is a design intention, not a guarantee.

| | Meaning |
|---|---|
| ✅ | Asserted by a passing test today |
| ◐ | The arithmetic is unit-tested; the aggregate property over real accounts is not |
| ⬜ | No test yet — see [ROADMAP.md](./ROADMAP.md) for the phase that adds it |

Notation: `Σ` sums over all accounts of that type belonging to the pool/realm.

**Current totals across 49 invariants: 34 ✅ · 7 ◐ · 8 ⬜.**

The authority chain is now verified end to end at runtime: a passed, timelocked proposal
moves treasury funds, and a direct call to the treasury is refused. The Token-2022 fee
invariants (§2) are verified against a real fee-bearing mint.

Two things worth knowing about how much to trust these:

**The fee tests were mutation-tested.** Reverting the fix so deposits credit the `amount`
argument instead of the vault delta makes three of them fail with a 30,000-unit shortfall
between positions and vault. The plain-mint test still passes under that mutation, which
is exactly why unit tests alone could never have caught it.

**The negative tests assert specific error names, not merely that something failed.** An
earlier draft accepted any error containing `0x`, and that permissiveness hid a test which
was passing for the wrong reason — the guardian had no lamports, so its vote failed on
rent rather than on authorisation. Tightening the assertion exposed it. A negative test
that does not name its expected failure is not a test.

What remains ⬜ is concentrated in staking's withdrawal lifecycle, vesting token movement,
and compute benchmarking.

---

## 1. Solvency

| # | Invariant | Test | Status |
|---|-----------|------|--------|
| 1.1 | `Σ position.amount == stake_vault.amount` | `fee_bearing_mint_preserves_vault_solvency` | ✅ |
| 1.2 | `Σ (position.pending + earned(position)) <= reward_vault.amount` | `liability_is_never_understated` | ◐ |
| 1.3 | `pool.total_staked == Σ position.amount` | `fee_bearing_mint_preserves_vault_solvency` | ✅ |
| 1.4 | `pool.total_weighted == Σ position.weighted_amount` | `weighted_amount_is_derived_from_the_credited_amount` | ◐ |
| 1.5 | `Σ stream.claimed <= Σ stream.total_amount` | `outstanding_tracks_the_unclaimed_remainder` | ◐ |
| 1.6 | `Σ (stream.total_amount - stream.claimed) <= treasury_vault.amount` | `uncommitted_excludes_stream_obligations` | ◐ |

1.2 is the one that matters. A reward pool that can promise more than it holds is
insolvent from that moment, and the failure surfaces much later as a confusing transfer
error for whoever claims last.

The guard enforcing it lives in `set_reward_rate`, and it is where
[F-2](./SECURITY-ASSESSMENT.md#f-2--reward-liability-computed-from-deposits) was found:
liability was computed from deposits rather than accruals, which made the check reject
every non-zero rate. Liability is now `accrued - paid`, and deliberately **over**-states
debt by the retained rounding dust — a liability estimate must never be too low.

## 2. Token-2022 transfer fees

| # | Invariant | Test | Status |
|---|-----------|------|--------|
| 2.1 | Credited stake == observed vault balance delta, never the `amount` argument | `fee_bearing_mint_credits_the_delta_not_the_argument` | ✅ |
| 2.2 | Depositing `n` into a fee-bearing mint credits `< n`, and 1.1 still holds | `fee_bearing_mint_preserves_vault_solvency` | ✅ |
| 2.3 | Weight is derived from the credited amount, not the argument | `weighted_amount_is_derived_from_the_credited_amount` | ✅ |

If the mint carries a transfer-fee extension, `transfer_checked` moves `amount` but the
vault receives `amount - fee`. Crediting `amount` breaks 1.1 immediately and lets the pool
be drained by repeated deposit/withdraw cycles. Every deposit path reads the vault balance
before and after and credits the difference.

**Verified against a real fee-bearing mint** in
[`staking_transfer_fee.rs`](../tests/integration/tests/staking_transfer_fee.rs), which
runs the same flow twice — once on a plain mint, once on a mint with a 1% transfer fee —
and asserts the difference.

This was the most dangerous invariant in the project precisely because it looked covered.
On a plain SPL mint the delta and the argument are identical, so the entire unit suite
passes either way. The mutation test confirms it: injecting the bug fails three
integration tests but leaves the plain-mint test green.

## 3. Reward accounting

| # | Invariant | Test | Status |
|---|-----------|------|--------|
| 3.1 | `reward_per_token` is monotonically non-decreasing | `accumulator_is_monotonic` | ✅ |
| 3.2 | Booked liability never understates what positions can claim | `liability_is_never_understated` | ✅ |
| 3.3 | Rounding always favours the pool: `Σ earned <= exact_entitlement` | `rounding_always_favours_the_pool` | ✅ |
| 3.4 | A position opened and closed within one timestamp earns 0 | `open_and_close_within_one_timestamp_earns_nothing` | ✅ |
| 3.5 | `update_rewards` is idempotent within a timestamp | `update_is_idempotent_within_a_timestamp` | ✅ |
| 3.6 | A stale timestamp never rewinds the pool or double-credits | `clock_never_runs_backwards` | ✅ |
| 3.7 | Emissions stop at `reward_period_end` | `emissions_stop_at_period_end` | ✅ |
| 3.8 | Idle time (no stake) accrues no liability and pays nobody | `idle_time_accrues_no_liability` | ✅ |
| 3.9 | A position earns nothing for time before it existed | `a_position_earns_nothing_for_time_before_it_existed` | ✅ |

3.3 is checked differentially: an exact computation in the test harness versus the
on-chain fixed-point result, asserting the on-chain value is never larger.

## 4. Governance integrity

| # | Invariant | Test | Status |
|---|-----------|------|--------|
| 4.1 | One `VoteRecord` per `(proposal, position)` — double voting impossible | `a_position_cannot_vote_twice` | ✅ |
| 4.2 | Vote weight counted only if `position.lock_end >= proposal.voting_ends_at` | `a_flash_staked_position_cannot_vote` (runtime), `lock_gate_rejects_freshly_opened_positions` | ✅ |
| 4.3 | `for + against + abstain <= total_weight_snapshot` | `the_full_lifecycle_visits_every_state` | ◐ |
| 4.4 | No proposal executes before `eta` | `execution_before_the_timelock_elapses_is_refused` | ✅ |
| 4.5 | No proposal executes twice | `a_proposal_cannot_execute_twice` | ✅ |
| 4.6 | State transitions follow the documented lifecycle; no state is skipped | `the_full_lifecycle_visits_every_state` (runtime), `require_state_gates_transitions` | ✅ |
| 4.7 | The guardian can only cancel — never pass, queue, or execute | `the_guardian_cannot_do_anything_but_cancel`, `the_guardian_veto_prevents_execution` | ✅ |
| 4.11 | Execution parameters come from `proposal.action`, not the caller | `executing_a_different_destination_than_the_proposal_named_is_refused` | ✅ |
| 4.12 | An action variant cannot be executed through the wrong handler | `a_signal_proposal_cannot_be_executed_as_a_treasury_transfer` | ✅ |
| 4.8 | Quorum and approval lose nothing to rounding | `quorum_is_not_lost_to_rounding`, `supermajority_threshold` | ✅ |
| 4.9 | Abstentions count toward quorum but never toward approval | `abstentions_do_not_help_approval` | ✅ |
| 4.10 | A queued proposal expires and cannot execute afterwards | `queued_proposals_expire` | ◐ |

4.7 is marked ◐ because the *veto window* is tested but the "guardian has no other power"
half is currently established by inspection — `realm.guardian` is read in exactly one
instruction. A runtime test attempting every other instruction as guardian would make it ✅.

## 5. Authority

| # | Invariant | Test | Status |
|---|-----------|------|--------|
| 5.1 | Treasury funds move only under the governance executor's signature | `treasury_rejects_a_spend_that_is_not_from_governance`, `a_passed_proposal_moves_treasury_funds` | ✅ |
| 5.2 | HLX is minted only by a registered minter, within its epoch cap | `accrues_within_cap`, `rejects_over_cap_without_mutating` | ◐ |
| 5.3 | No non-PDA address holds mint authority after `initialize_token` | `token_manager::mint_authority_is_pda` | ⬜ P2 |
| 5.4 | Admin transfer requires both `propose` and `accept` | `token_manager::two_step_admin` | ⬜ P2 |
| 5.5 | Every PDA is derived with a stored, verified bump | `*::canonical_bumps` | ⬜ P2 |
| 5.6 | An epoch cap grants no accumulated allowance for idle epochs | `skipping_epochs_does_not_accumulate_allowance` | ✅ |
| 5.7 | A treasury spend cap likewise does not accumulate | `idle_epochs_do_not_accumulate_budget` | ✅ |
| 5.8 | Initialisers cannot install an unintended authority | *(none — see [F-1](./SECURITY-ASSESSMENT.md#f-1--initialisers-are-front-runnable))* | ⬜ P3 |

## 6. Liveness

| # | Invariant | Test | Status |
|---|-----------|------|--------|
| 6.1 | `claim` succeeds regardless of lock state | `staking::claim_ignores_lock` | ⬜ P2 |
| 6.2 | `unstake` after `lock_end` always succeeds if the vault is solvent | `staking::unstake_after_lock_always_works` | ⬜ P2 |
| 6.3 | No instruction's compute cost grows with staker or voter count | `bench::compute_is_constant` | ⬜ P6 |
| 6.4 | Pausing cannot trap principal — `unstake` and `claim` stay live | `staking::pause_does_not_trap_funds` | ⬜ P2 |

6.4 is a deliberate limit on the pause switch. A pause that stops withdrawals is
indistinguishable from a rug from the user's side; the pause here blocks *new* deposits
and rate changes only.

6.3 currently follows from the absence of any loop over user-growable sets, which is a
sound argument but not a measurement — compute has non-obvious contributors, account
deserialisation especially.

## 7. Vesting

| # | Invariant | Test | Status |
|---|-----------|------|--------|
| 7.1 | Nothing is claimable before the cliff | `nothing_vests_before_the_cliff` | ✅ |
| 7.2 | The cliff releases everything accrued since `start_ts` | `the_cliff_releases_everything_accrued_since_start` | ✅ |
| 7.3 | Exactly `total_amount` vests by `end_ts`, and never more | `the_full_amount_vests_at_the_end_and_never_more` | ✅ |
| 7.4 | Truncation favours the treasury, but the endpoint still pays in full | `vesting_truncates_in_the_treasurys_favour` | ✅ |
| 7.5 | A revoke freezes accrual and never claws back vested tokens | `revoke_freezes_vesting_without_clawing_back` | ✅ |
| 7.6 | Already-claimed tokens survive a revoke | `already_claimed_tokens_survive_a_revoke` | ✅ |
| 7.7 | The unvested remainder returns to the treasury's spendable balance | `unvested_remainder_returns_to_the_treasury` | ✅ |

---

## Running the suite

```bash
anchor build 2>&1 | tee build.log && grep -i "stack offset" build.log  # must be empty
cargo test --workspace          # 71 tests: unit + doc + integration
cargo clippy --workspace --all-targets -- -D warnings
```

Use `cargo test --workspace`, not `--lib`. The `--lib` filter skips doctests, and the
program READMEs are compiled via `#![doc = include_str!(..)]` — a broken code fence there
is a real failure that `--lib` hides. Integration tests need `anchor build` to have run
first, since they load the `.so` binaries.

Governance and treasury runtime flows, and the fuzz suite, do not exist yet. See
[TESTING.md](./TESTING.md) and [ROADMAP.md](./ROADMAP.md).
