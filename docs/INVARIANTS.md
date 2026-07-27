# Invariants

Properties that must hold after **every** instruction, in every state. Each one names
the test that asserts it. If you change a program and one of these tests goes red, the
change is wrong until proven otherwise.

Notation: `Σ` sums over all accounts of that type belonging to the pool/realm.

---

## 1. Solvency

| # | Invariant | Test |
|---|-----------|------|
| 1.1 | `Σ position.amount == stake_vault.amount` | `staking::solvency_stake_vault` |
| 1.2 | `Σ (position.pending + earned(position)) <= reward_vault.amount` | `staking::solvency_reward_vault` |
| 1.3 | `pool.total_staked == Σ position.amount` | `staking::total_staked_matches` |
| 1.4 | `pool.total_weighted == Σ position.weighted_amount` | `staking::total_weighted_matches` |
| 1.5 | `Σ stream.claimed <= Σ stream.total_amount` | `treasury::vesting_never_overclaims` |
| 1.6 | `Σ (stream.total_amount - stream.claimed) <= treasury_vault.amount` | `treasury::streams_are_funded` |

1.2 is the one that matters. A reward pool that can promise more than it holds is
insolvent the moment the last user tries to claim, and the failure surfaces as a
confusing transfer error for whoever claims last — not as an alert for the operator.
`fund_rewards` is the only way the right-hand side grows, and `set_reward_rate`
rejects a rate that cannot be sustained to `reward_period_end`.

## 2. Token-2022 transfer fees

| # | Invariant | Test |
|---|-----------|------|
| 2.1 | Credited stake == observed vault balance delta, never the `amount` argument | `staking::fee_mint_credits_delta` |
| 2.2 | Depositing `n` into a fee-bearing mint credits `< n`, and 1.1 still holds | `staking::fee_mint_preserves_solvency` |

If the mint carries a transfer-fee extension, `transfer_checked` moves `amount` but
the vault receives `amount - fee`. Crediting the user `amount` breaks 1.1 immediately
and lets the pool be drained by repeated deposit/withdraw cycles. Every deposit reads
the vault balance before and after and credits the difference.

## 3. Reward accounting

| # | Invariant | Test |
|---|-----------|------|
| 3.1 | `reward_per_token` is monotonically non-decreasing | `staking::rpt_monotonic` |
| 3.2 | `Σ rewards_paid <= reward_rate * (last_update_ts - start_ts)` | `staking::never_overpays_emission` |
| 3.3 | Rounding always favours the pool: `Σ earned <= exact_entitlement` | `staking::rounding_favours_pool` |
| 3.4 | A position opened and closed within one slot earns 0 | `staking::no_same_slot_yield` |
| 3.5 | `update_pool` is idempotent within a slot | `staking::update_pool_idempotent` |

3.3 is checked with a differential test: an exact rational computation in the test
harness versus the on-chain fixed-point result, asserting the on-chain value is never
larger.

## 4. Governance integrity

| # | Invariant | Test |
|---|-----------|------|
| 4.1 | One `VoteRecord` per `(proposal, position)` — double voting is impossible | `governance::no_double_vote` |
| 4.2 | Vote weight is only counted if `position.lock_end >= proposal.voting_ends_at` | `governance::flash_stake_cannot_vote` |
| 4.3 | `proposal.for + against + abstain <= total_weighted` at snapshot | `governance::votes_bounded_by_supply` |
| 4.4 | No proposal executes before `eta` | `governance::timelock_enforced` |
| 4.5 | No proposal executes twice | `governance::no_double_execute` |
| 4.6 | State transitions follow the documented lifecycle; no state is skipped | `governance::lifecycle_transitions` |
| 4.7 | The guardian can only cancel — never pass, queue, or execute | `governance::guardian_is_veto_only` |

## 5. Authority

| # | Invariant | Test |
|---|-----------|------|
| 5.1 | Treasury funds move only under the governance execution PDA's signature | `treasury::only_governance_spends` |
| 5.2 | HLX is minted only by a registered minter, within its epoch cap | `token_manager::minter_registry_enforced` |
| 5.3 | No mint authority is held by any non-PDA address after `initialize_token` | `token_manager::mint_authority_is_pda` |
| 5.4 | Admin transfer requires both `propose` and `accept` | `token_manager::two_step_admin` |
| 5.5 | Every PDA is derived with a stored, verified bump | `*::canonical_bumps` |

## 6. Liveness

| # | Invariant | Test |
|---|-----------|------|
| 6.1 | `claim` succeeds regardless of lock state | `staking::claim_ignores_lock` |
| 6.2 | `unstake` after `lock_end` always succeeds if the vault is solvent | `staking::unstake_after_lock_always_works` |
| 6.3 | No instruction's compute cost grows with the number of stakers or voters | `bench::compute_is_constant` |
| 6.4 | Pausing cannot trap user principal — `unstake` and `claim` stay live | `staking::pause_does_not_trap_funds` |

6.4 is a deliberate limit on the pause switch. A pause that stops withdrawals is
indistinguishable from a rug from the user's side; Helix's pause blocks *new* deposits
and rate changes only.

---

## Running the invariant suite

```bash
anchor test                      # full suite, Surfpool validator
cargo test -p helix-staking      # fast Rust unit + LiteSVM tests
cargo test --test invariants     # property tests only
trident fuzz run                 # stateful fuzzing (see docs/FUZZING.md)
```
