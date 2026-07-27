# token-manager

Owns the HLX mint.

The mint and freeze authorities are held by a PDA (`["mint_authority", config]`),
so no keypair anywhere can mint HLX. New supply has exactly one path:

```text
mint_tokens
  ├── requires a registered Minter account          ┐
  ├── requires that minter's own signature          ├─ all three, or the tx fails
  └── requires headroom under its per-epoch cap     ┘
```

In the deployed system the only registered minter is the staking program's reward
PDA, which means "who can create HLX?" has the answer "the staking program, at a
rate the DAO voted for, and never more than the cap per day".

## Accounts

| Account | PDA seeds | Purpose |
|---------|-----------|---------|
| `TokenConfig` | `["config", mint]` | Admin, mint, pause flag, lifetime supply counters |
| `Minter` | `["minter", config, authority]` | One registry entry: cap, epoch window, issuance history |

## Instructions

| Instruction | Authority | Notes |
|-------------|-----------|-------|
| `initialize_token` | anyone (once per mint) | Creates the Token-2022 mint, moves authority to the PDA |
| `register_minter` | admin | Bounded by `MAX_MINTERS` |
| `update_minter` | admin | Adjust cap / enable / disable |
| `revoke_minter` | admin | Disables; keeps the account so issuance history stays auditable |
| `mint_tokens` | a registered minter | Charges the epoch budget *before* the CPI |
| `burn_tokens` | token owner | Not blocked by pause — see below |
| `propose_admin` / `accept_admin` | admin, then successor | Two-step |
| `cancel_admin_transfer` | admin | |
| `set_paused` | admin | Blocks issuance only |

## Two design choices worth noting

**Admin transfer is two-step.** `propose_admin` records a successor; `accept_admin`
requires that successor's signature. A one-step transfer to a mistyped address
permanently orphans the role, and the signature requirement makes an unusable address
impossible to install.

**Pause does not block burning.** Pausing stops new issuance. Burning only ever reduces
the caller's own balance and total supply, so blocking it would gain no safety and
would turn the pause switch into something holders cannot distinguish from a freeze.
The same reasoning constrains the staking pause — see `INVARIANTS.md` §6.4.

## Tests

`cargo test -p helix-token-manager` covers the epoch-cap state machine directly:
accrual within cap, rejection without mutation on breach, window rollover, and the
case that matters most — that skipping ten idle epochs does not grant ten epochs'
worth of allowance.
