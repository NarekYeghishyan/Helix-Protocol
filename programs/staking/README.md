# staking

Lock-tiered staking with O(1) reward distribution. This is the program the rest of
the system leans on: it holds the principal, it pays the rewards, and it produces the
vote weight that `helix-governance` consumes.

## Reward accounting

Rewards accrue at `reward_rate` tokens per second and are split in proportion to
**weighted** stake. Distribution uses a `reward_per_token` accumulator in u128 fixed
point:

```text
update_rewards(now):
    if total_weighted > 0:
        emitted           = (min(now, period_end) - last_update) * reward_rate
        reward_per_token += emitted * PRECISION / total_weighted
    last_update = now

earned(position):
    position.pending + weighted * (reward_per_token - position.paid) / PRECISION
```

Cost is O(1) per user, and no instruction iterates over the staker set. The obvious
alternative — looping over stakers to credit each — passes a ten-staker test and then
permanently bricks the pool once distribution exceeds the compute budget.

Both divisions truncate, so every rounding remainder stays in the vault. The direction
is deliberate and tested: an error that favours the user is a slow drain of the reward
vault, paid for by whoever claims last.

## Lock tiers

| Tier | Lock | Weight |
|------|------|--------|
| Flexible | none | 1.00× |
| Bronze | 30 days | 1.25× |
| Silver | 90 days | 1.50× |
| Gold | 180 days | 2.00× |

Weight drives reward share *and* vote power. A user may hold several positions in
different tiers; each is its own account, seeded
`["position", pool, owner, position_id]` with `position_id` a `u64` pinned to a
monotonic counter — fixed-length seeds, nothing caller-controlled to craft a collision
from.

`pool.position_count` means **positions ever opened**, and nothing else is safe. It is a
PDA seed, and `helix-governance` snapshots it at activation as the electorate boundary, so
an id that could be recycled would let a position created after a proposal opened vote on
it. `close_position` therefore reclaims rent without touching the counter — see
[F-7](../../docs/SECURITY-ASSESSMENT.md#f-7--position-accounts-never-closed) for what the
obvious implementation breaks.

## Token-2022 transfer fees

Every deposit credits the **observed vault balance delta**, never the `amount`
argument:

```text
let before = vault.amount;
transfer_checked(..., amount, decimals)?;
vault.reload()?;
let credited = vault.amount - before;      // never `amount`
```

With a transfer-fee extension active the vault receives `amount - fee`. Crediting
`amount` breaks solvency immediately and lets repeated deposit/withdraw cycles drain
the pool. This is invisible on a plain SPL mint, which is why it is easy to miss.

## Deliberate limits

**Early exit is refused, not penalised.** A penalty creates a second reward-bearing
balance to account for everywhere; refusing keeps solvency a single subtraction.

**Pause blocks deposits only.** `unstake` and `claim` stay live. A pause that traps
principal is indistinguishable from a freeze — see `INVARIANTS.md` §6.4.

**`set_reward_rate` refuses what the vault cannot fund.** Outstanding accrued rewards
plus everything the new rate commits to must both fit in the vault. Otherwise the pool
is insolvent from that moment, and the failure surfaces much later as a confusing
transfer error for whoever claims last.

## Instructions

| Instruction | Authority | Notes |
|-------------|-----------|-------|
| `initialize_pool` | anyone (once per mint pair) | Emissions start at zero |
| `stake` | staker | Credits the vault delta; blocked by pause |
| `unstake` | position owner | Requires `now >= lock_end`; **not** blocked by pause |
| `claim` | position owner | Always available |
| `close_position` | position owner | Reclaims rent; refused unless principal, weight and unclaimed rewards are all zero |
| `fund_rewards` | anyone | Topping up can only help stakers |
| `set_reward_rate` | pool authority | Solvency-checked |
| `set_paused` | pool authority | Deposits only |
| `propose_authority` / `accept_authority` | authority, then successor | Two-step |

## Tests

`cargo test -p helix-staking` covers the math directly — 19 unit tests including
accumulator monotonicity, idempotence within a timestamp, emissions halting at period
end, the clock never running backwards, a position earning nothing for time before it
existed, open-and-close-in-one-slot earning zero, and a differential check that the
sum payable to all stakers never exceeds what the pool emitted.
