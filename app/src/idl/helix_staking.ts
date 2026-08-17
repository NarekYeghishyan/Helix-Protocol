/**
 * Generated from `target/idl/helix_staking.json` by `node scripts/sync-idl.mjs`.
 *
 * Do not edit. `tests/integration/tests/idl_sync.rs` fails if this file is not
 * byte-for-byte what `anchor build` currently produces.
 */

import type { Idl } from "../lib/idl.ts";

const idl: Idl = {
  "address": "9RuZJZpgCwbiF9JRAsyR8cqDhFSaFYus1mzobKzEZzP3",
  "metadata": {
    "name": "helix_staking",
    "version": "0.1.0",
    "spec": "0.1.0",
    "description": "Lock-tiered staking with O(1) reward distribution and lock-gated vote weight.",
    "repository": "https://github.com/NarekYeghishyan/Helix-Protocol"
  },
  "instructions": [
    {
      "name": "accept_authority",
      "docs": [
        "Step two. Callable only by the proposed successor."
      ],
      "discriminator": [
        107,
        86,
        198,
        91,
        33,
        12,
        107,
        160
      ],
      "accounts": [
        {
          "name": "pool",
          "writable": true,
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  112,
                  111,
                  111,
                  108
                ]
              },
              {
                "kind": "account",
                "path": "pool.stake_mint",
                "account": "Pool"
              },
              {
                "kind": "account",
                "path": "pool.reward_mint",
                "account": "Pool"
              }
            ]
          }
        },
        {
          "name": "new_authority",
          "docs": [
            "The proposed successor, proving key custody by signing."
          ],
          "signer": true
        }
      ],
      "args": []
    },
    {
      "name": "claim",
      "docs": [
        "Withdraws accrued rewards. Available regardless of lock or pause state."
      ],
      "discriminator": [
        62,
        198,
        214,
        193,
        213,
        159,
        108,
        210
      ],
      "accounts": [
        {
          "name": "pool",
          "writable": true,
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  112,
                  111,
                  111,
                  108
                ]
              },
              {
                "kind": "account",
                "path": "pool.stake_mint",
                "account": "Pool"
              },
              {
                "kind": "account",
                "path": "pool.reward_mint",
                "account": "Pool"
              }
            ]
          },
          "relations": [
            "position"
          ]
        },
        {
          "name": "owner",
          "signer": true,
          "relations": [
            "position"
          ]
        },
        {
          "name": "position",
          "writable": true,
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  112,
                  111,
                  115,
                  105,
                  116,
                  105,
                  111,
                  110
                ]
              },
              {
                "kind": "account",
                "path": "pool"
              },
              {
                "kind": "account",
                "path": "owner"
              },
              {
                "kind": "account",
                "path": "position.position_id",
                "account": "Position"
              }
            ]
          }
        },
        {
          "name": "reward_mint",
          "relations": [
            "pool"
          ]
        },
        {
          "name": "reward_vault",
          "writable": true,
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  114,
                  101,
                  119,
                  97,
                  114,
                  100,
                  95,
                  118,
                  97,
                  117,
                  108,
                  116
                ]
              },
              {
                "kind": "account",
                "path": "pool"
              }
            ]
          },
          "relations": [
            "pool"
          ]
        },
        {
          "name": "owner_reward_account",
          "writable": true
        },
        {
          "name": "vault_authority",
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  118,
                  97,
                  117,
                  108,
                  116,
                  95,
                  97,
                  117,
                  116,
                  104,
                  111,
                  114,
                  105,
                  116,
                  121
                ]
              },
              {
                "kind": "account",
                "path": "pool"
              }
            ]
          }
        },
        {
          "name": "token_program"
        }
      ],
      "args": []
    },
    {
      "name": "close_position",
      "docs": [
        "Reclaims the rent of a position that holds nothing. Refused while any",
        "principal, weight or unclaimed reward remains."
      ],
      "discriminator": [
        123,
        134,
        81,
        0,
        49,
        68,
        98,
        98
      ],
      "accounts": [
        {
          "name": "pool",
          "docs": [
            "Read-only, and deliberately so: a closable position holds no principal,",
            "no weight and no unpaid rewards, so there is nothing about it left in",
            "the pool's books to adjust. If closing ever needed to write here, the",
            "guard below would be wrong."
          ],
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  112,
                  111,
                  111,
                  108
                ]
              },
              {
                "kind": "account",
                "path": "pool.stake_mint",
                "account": "Pool"
              },
              {
                "kind": "account",
                "path": "pool.reward_mint",
                "account": "Pool"
              }
            ]
          },
          "relations": [
            "position"
          ]
        },
        {
          "name": "owner",
          "docs": [
            "Receives the reclaimed rent, having paid it in `stake`."
          ],
          "writable": true,
          "signer": true,
          "relations": [
            "position"
          ]
        },
        {
          "name": "position",
          "writable": true,
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  112,
                  111,
                  115,
                  105,
                  116,
                  105,
                  111,
                  110
                ]
              },
              {
                "kind": "account",
                "path": "pool"
              },
              {
                "kind": "account",
                "path": "owner"
              },
              {
                "kind": "account",
                "path": "position.position_id",
                "account": "Position"
              }
            ]
          }
        }
      ],
      "args": []
    },
    {
      "name": "fund_rewards",
      "docs": [
        "Tops up the reward vault. Permissionless."
      ],
      "discriminator": [
        114,
        64,
        163,
        112,
        175,
        167,
        19,
        121
      ],
      "accounts": [
        {
          "name": "pool",
          "writable": true,
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  112,
                  111,
                  111,
                  108
                ]
              },
              {
                "kind": "account",
                "path": "pool.stake_mint",
                "account": "Pool"
              },
              {
                "kind": "account",
                "path": "pool.reward_mint",
                "account": "Pool"
              }
            ]
          }
        },
        {
          "name": "funder",
          "docs": [
            "Anyone may top up the reward vault. Funding the pool can only benefit",
            "stakers, so there is no reason to restrict it to the authority."
          ],
          "writable": true,
          "signer": true
        },
        {
          "name": "reward_mint",
          "relations": [
            "pool"
          ]
        },
        {
          "name": "funder_token_account",
          "writable": true
        },
        {
          "name": "reward_vault",
          "writable": true,
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  114,
                  101,
                  119,
                  97,
                  114,
                  100,
                  95,
                  118,
                  97,
                  117,
                  108,
                  116
                ]
              },
              {
                "kind": "account",
                "path": "pool"
              }
            ]
          },
          "relations": [
            "pool"
          ]
        },
        {
          "name": "token_program"
        }
      ],
      "args": [
        {
          "name": "amount",
          "type": "u64"
        }
      ]
    },
    {
      "name": "initialize_pool",
      "docs": [
        "Creates a pool for a (stake mint, reward mint) pair, with both vaults",
        "owned by a PDA. Emissions start at zero."
      ],
      "discriminator": [
        95,
        180,
        10,
        172,
        84,
        174,
        232,
        40
      ],
      "accounts": [
        {
          "name": "payer",
          "writable": true,
          "signer": true
        },
        {
          "name": "authority",
          "docs": [
            "Sets the reward rate and funds rewards. Deliberately not a signer here —",
            "pool creation grants it no power it could not be given later."
          ]
        },
        {
          "name": "pool",
          "writable": true,
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  112,
                  111,
                  111,
                  108
                ]
              },
              {
                "kind": "account",
                "path": "stake_mint"
              },
              {
                "kind": "account",
                "path": "reward_mint"
              }
            ]
          }
        },
        {
          "name": "vault_authority",
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  118,
                  97,
                  117,
                  108,
                  116,
                  95,
                  97,
                  117,
                  116,
                  104,
                  111,
                  114,
                  105,
                  116,
                  121
                ]
              },
              {
                "kind": "account",
                "path": "pool"
              }
            ]
          }
        },
        {
          "name": "stake_mint"
        },
        {
          "name": "reward_mint"
        },
        {
          "name": "stake_vault",
          "writable": true,
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  115,
                  116,
                  97,
                  107,
                  101,
                  95,
                  118,
                  97,
                  117,
                  108,
                  116
                ]
              },
              {
                "kind": "account",
                "path": "pool"
              }
            ]
          }
        },
        {
          "name": "reward_vault",
          "writable": true,
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  114,
                  101,
                  119,
                  97,
                  114,
                  100,
                  95,
                  118,
                  97,
                  117,
                  108,
                  116
                ]
              },
              {
                "kind": "account",
                "path": "pool"
              }
            ]
          }
        },
        {
          "name": "token_program"
        },
        {
          "name": "system_program",
          "address": "11111111111111111111111111111111"
        }
      ],
      "args": []
    },
    {
      "name": "propose_authority",
      "docs": [
        "Step one of a two-step authority handover."
      ],
      "discriminator": [
        20,
        148,
        236,
        198,
        76,
        119,
        99,
        142
      ],
      "accounts": [
        {
          "name": "pool",
          "writable": true,
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  112,
                  111,
                  111,
                  108
                ]
              },
              {
                "kind": "account",
                "path": "pool.stake_mint",
                "account": "Pool"
              },
              {
                "kind": "account",
                "path": "pool.reward_mint",
                "account": "Pool"
              }
            ]
          }
        },
        {
          "name": "authority",
          "signer": true,
          "relations": [
            "pool"
          ]
        }
      ],
      "args": [
        {
          "name": "new_authority",
          "type": "pubkey"
        }
      ]
    },
    {
      "name": "set_paused",
      "docs": [
        "Blocks new deposits. Unstaking and claiming stay live."
      ],
      "discriminator": [
        91,
        60,
        125,
        192,
        176,
        225,
        166,
        218
      ],
      "accounts": [
        {
          "name": "pool",
          "writable": true,
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  112,
                  111,
                  111,
                  108
                ]
              },
              {
                "kind": "account",
                "path": "pool.stake_mint",
                "account": "Pool"
              },
              {
                "kind": "account",
                "path": "pool.reward_mint",
                "account": "Pool"
              }
            ]
          }
        },
        {
          "name": "authority",
          "signer": true,
          "relations": [
            "pool"
          ]
        }
      ],
      "args": [
        {
          "name": "paused",
          "type": "bool"
        }
      ]
    },
    {
      "name": "set_reward_rate",
      "docs": [
        "Sets emission rate and period end. Refuses a rate the vault cannot fund."
      ],
      "discriminator": [
        253,
        201,
        190,
        20,
        48,
        38,
        120,
        34
      ],
      "accounts": [
        {
          "name": "pool",
          "writable": true,
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  112,
                  111,
                  111,
                  108
                ]
              },
              {
                "kind": "account",
                "path": "pool.stake_mint",
                "account": "Pool"
              },
              {
                "kind": "account",
                "path": "pool.reward_mint",
                "account": "Pool"
              }
            ]
          }
        },
        {
          "name": "authority",
          "signer": true,
          "relations": [
            "pool"
          ]
        },
        {
          "name": "reward_vault",
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  114,
                  101,
                  119,
                  97,
                  114,
                  100,
                  95,
                  118,
                  97,
                  117,
                  108,
                  116
                ]
              },
              {
                "kind": "account",
                "path": "pool"
              }
            ]
          },
          "relations": [
            "pool"
          ]
        }
      ],
      "args": [
        {
          "name": "new_rate",
          "type": "u64"
        },
        {
          "name": "reward_period_end",
          "type": "i64"
        }
      ]
    },
    {
      "name": "stake",
      "docs": [
        "Opens a position of `amount` under `tier`. `position_id` must equal the",
        "pool's current `position_count`."
      ],
      "discriminator": [
        206,
        176,
        202,
        18,
        200,
        209,
        179,
        108
      ],
      "accounts": [
        {
          "name": "pool",
          "writable": true,
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  112,
                  111,
                  111,
                  108
                ]
              },
              {
                "kind": "account",
                "path": "pool.stake_mint",
                "account": "Pool"
              },
              {
                "kind": "account",
                "path": "pool.reward_mint",
                "account": "Pool"
              }
            ]
          }
        },
        {
          "name": "owner",
          "writable": true,
          "signer": true
        },
        {
          "name": "position",
          "writable": true,
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  112,
                  111,
                  115,
                  105,
                  116,
                  105,
                  111,
                  110
                ]
              },
              {
                "kind": "account",
                "path": "pool"
              },
              {
                "kind": "account",
                "path": "owner"
              },
              {
                "kind": "arg",
                "path": "position_id"
              }
            ]
          }
        },
        {
          "name": "stake_mint",
          "relations": [
            "pool"
          ]
        },
        {
          "name": "owner_token_account",
          "writable": true
        },
        {
          "name": "stake_vault",
          "writable": true,
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  115,
                  116,
                  97,
                  107,
                  101,
                  95,
                  118,
                  97,
                  117,
                  108,
                  116
                ]
              },
              {
                "kind": "account",
                "path": "pool"
              }
            ]
          },
          "relations": [
            "pool"
          ]
        },
        {
          "name": "token_program"
        },
        {
          "name": "system_program",
          "address": "11111111111111111111111111111111"
        }
      ],
      "args": [
        {
          "name": "position_id",
          "type": "u64"
        },
        {
          "name": "amount",
          "type": "u64"
        },
        {
          "name": "tier",
          "type": {
            "defined": {
              "name": "LockTier"
            }
          }
        }
      ]
    },
    {
      "name": "unstake",
      "docs": [
        "Withdraws principal from an unlocked position. Not blocked by pause."
      ],
      "discriminator": [
        90,
        95,
        107,
        42,
        205,
        124,
        50,
        225
      ],
      "accounts": [
        {
          "name": "pool",
          "writable": true,
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  112,
                  111,
                  111,
                  108
                ]
              },
              {
                "kind": "account",
                "path": "pool.stake_mint",
                "account": "Pool"
              },
              {
                "kind": "account",
                "path": "pool.reward_mint",
                "account": "Pool"
              }
            ]
          },
          "relations": [
            "position"
          ]
        },
        {
          "name": "owner",
          "writable": true,
          "signer": true,
          "relations": [
            "position"
          ]
        },
        {
          "name": "position",
          "writable": true,
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  112,
                  111,
                  115,
                  105,
                  116,
                  105,
                  111,
                  110
                ]
              },
              {
                "kind": "account",
                "path": "pool"
              },
              {
                "kind": "account",
                "path": "owner"
              },
              {
                "kind": "account",
                "path": "position.position_id",
                "account": "Position"
              }
            ]
          }
        },
        {
          "name": "stake_mint",
          "relations": [
            "pool"
          ]
        },
        {
          "name": "stake_vault",
          "writable": true,
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  115,
                  116,
                  97,
                  107,
                  101,
                  95,
                  118,
                  97,
                  117,
                  108,
                  116
                ]
              },
              {
                "kind": "account",
                "path": "pool"
              }
            ]
          },
          "relations": [
            "pool"
          ]
        },
        {
          "name": "owner_token_account",
          "writable": true
        },
        {
          "name": "vault_authority",
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  118,
                  97,
                  117,
                  108,
                  116,
                  95,
                  97,
                  117,
                  116,
                  104,
                  111,
                  114,
                  105,
                  116,
                  121
                ]
              },
              {
                "kind": "account",
                "path": "pool"
              }
            ]
          }
        },
        {
          "name": "token_program"
        }
      ],
      "args": [
        {
          "name": "amount",
          "type": "u64"
        }
      ]
    }
  ],
  "accounts": [
    {
      "name": "Pool",
      "discriminator": [
        241,
        154,
        109,
        4,
        17,
        177,
        109,
        188
      ]
    },
    {
      "name": "Position",
      "discriminator": [
        170,
        188,
        143,
        228,
        122,
        64,
        247,
        208
      ]
    }
  ],
  "events": [
    {
      "name": "AuthorityTransferAccepted",
      "discriminator": [
        149,
        165,
        140,
        221,
        104,
        203,
        239,
        121
      ]
    },
    {
      "name": "AuthorityTransferProposed",
      "discriminator": [
        103,
        244,
        27,
        116,
        177,
        4,
        100,
        119
      ]
    },
    {
      "name": "PoolInitialized",
      "discriminator": [
        100,
        118,
        173,
        87,
        12,
        198,
        254,
        229
      ]
    },
    {
      "name": "PoolPauseToggled",
      "discriminator": [
        190,
        233,
        13,
        162,
        239,
        176,
        159,
        109
      ]
    },
    {
      "name": "PositionClosed",
      "discriminator": [
        157,
        163,
        227,
        228,
        13,
        97,
        138,
        121
      ]
    },
    {
      "name": "RewardRateChanged",
      "discriminator": [
        205,
        131,
        65,
        52,
        199,
        73,
        225,
        50
      ]
    },
    {
      "name": "RewardsClaimed",
      "discriminator": [
        75,
        98,
        88,
        18,
        219,
        112,
        88,
        121
      ]
    },
    {
      "name": "RewardsFunded",
      "discriminator": [
        84,
        233,
        245,
        203,
        228,
        147,
        165,
        92
      ]
    },
    {
      "name": "Staked",
      "discriminator": [
        11,
        146,
        45,
        205,
        230,
        58,
        213,
        240
      ]
    },
    {
      "name": "Unstaked",
      "discriminator": [
        27,
        179,
        156,
        215,
        47,
        71,
        195,
        7
      ]
    }
  ],
  "errors": [
    {
      "code": 6000,
      "name": "NotAuthority",
      "msg": "Caller is not the pool authority"
    },
    {
      "code": 6001,
      "name": "NoPendingAuthority",
      "msg": "No authority transfer is pending"
    },
    {
      "code": 6002,
      "name": "NotPendingAuthority",
      "msg": "Caller is not the pending authority"
    },
    {
      "code": 6003,
      "name": "DepositsPaused",
      "msg": "New deposits are paused"
    },
    {
      "code": 6004,
      "name": "BelowMinimumStake",
      "msg": "Stake amount is below the minimum"
    },
    {
      "code": 6005,
      "name": "ZeroAmount",
      "msg": "Amount must be greater than zero"
    },
    {
      "code": 6006,
      "name": "PositionLocked",
      "msg": "Position is still locked"
    },
    {
      "code": 6007,
      "name": "InsufficientStake",
      "msg": "Requested amount exceeds the position balance"
    },
    {
      "code": 6008,
      "name": "RewardRateTooHigh",
      "msg": "Reward rate exceeds the permitted maximum"
    },
    {
      "code": 6009,
      "name": "InvalidRewardPeriod",
      "msg": "Reward period end must be in the future"
    },
    {
      "code": 6010,
      "name": "InsufficientRewardFunding",
      "msg": "Reward vault cannot fund this rate for the full period"
    },
    {
      "code": 6011,
      "name": "NothingToClaim",
      "msg": "Nothing to claim"
    },
    {
      "code": 6012,
      "name": "ZeroAfterFees",
      "msg": "Deposit credited zero after transfer fees"
    },
    {
      "code": 6013,
      "name": "VaultBalanceMismatch",
      "msg": "Vault balance moved unexpectedly during the transfer"
    },
    {
      "code": 6014,
      "name": "PositionPoolMismatch",
      "msg": "Position does not belong to this pool"
    },
    {
      "code": 6015,
      "name": "MathOverflow",
      "msg": "Arithmetic overflow"
    },
    {
      "code": 6016,
      "name": "PositionNotEmpty",
      "msg": "Position still holds principal or vote weight"
    },
    {
      "code": 6017,
      "name": "UnclaimedRewards",
      "msg": "Position has unclaimed rewards"
    },
    {
      "code": 6018,
      "name": "UnexpectedPositionId",
      "msg": "position_id must equal the pool's current position_count"
    }
  ],
  "types": [
    {
      "name": "AuthorityTransferAccepted",
      "type": {
        "kind": "struct",
        "fields": [
          {
            "name": "pool",
            "type": "pubkey"
          },
          {
            "name": "previous_authority",
            "type": "pubkey"
          },
          {
            "name": "new_authority",
            "type": "pubkey"
          },
          {
            "name": "timestamp",
            "type": "i64"
          }
        ]
      }
    },
    {
      "name": "AuthorityTransferProposed",
      "type": {
        "kind": "struct",
        "fields": [
          {
            "name": "pool",
            "type": "pubkey"
          },
          {
            "name": "current_authority",
            "type": "pubkey"
          },
          {
            "name": "pending_authority",
            "type": "pubkey"
          },
          {
            "name": "timestamp",
            "type": "i64"
          }
        ]
      }
    },
    {
      "name": "LockTier",
      "docs": [
        "Lock commitment, which sets both reward share and governance weight.",
        "",
        "Weight is applied to the staked amount to give a *weighted* amount; all",
        "reward maths and all vote tallies operate on the weighted figure. Tying",
        "influence to lock duration is what makes the governance design in",
        "`helix-governance` work: vote weight cannot be rented for a single block."
      ],
      "type": {
        "kind": "enum",
        "variants": [
          {
            "name": "Flexible"
          },
          {
            "name": "Bronze"
          },
          {
            "name": "Silver"
          },
          {
            "name": "Gold"
          }
        ]
      }
    },
    {
      "name": "Pool",
      "docs": [
        "A staking pool. One per (stake mint, reward mint) pair."
      ],
      "type": {
        "kind": "struct",
        "fields": [
          {
            "name": "authority",
            "docs": [
              "Sets the reward rate and funds the reward vault. Cannot touch principal."
            ],
            "type": "pubkey"
          },
          {
            "name": "pending_authority",
            "docs": [
              "Pending half of the two-step authority handover."
            ],
            "type": {
              "option": "pubkey"
            }
          },
          {
            "name": "stake_mint",
            "type": "pubkey"
          },
          {
            "name": "reward_mint",
            "type": "pubkey"
          },
          {
            "name": "stake_vault",
            "type": "pubkey"
          },
          {
            "name": "reward_vault",
            "type": "pubkey"
          },
          {
            "name": "total_staked",
            "docs": [
              "Principal actually held, in stake-token base units. Must always equal the",
              "stake vault balance (`INVARIANTS.md` §1.1)."
            ],
            "type": "u64"
          },
          {
            "name": "total_weighted",
            "docs": [
              "Sum of every position's weighted amount. The denominator of reward share",
              "and of governance quorum."
            ],
            "type": "u64"
          },
          {
            "name": "reward_rate",
            "docs": [
              "Emission rate in reward-token base units per second."
            ],
            "type": "u64"
          },
          {
            "name": "reward_period_end",
            "docs": [
              "Emissions stop at this timestamp. Beyond it the accumulator stops",
              "advancing, so an unfunded pool silently stops paying instead of",
              "promising rewards it cannot cover."
            ],
            "type": "i64"
          },
          {
            "name": "reward_per_token",
            "docs": [
              "Rewards per unit of weighted stake since inception, scaled by",
              "[`PRECISION`]. Monotonically non-decreasing (`INVARIANTS.md` §3.1)."
            ],
            "type": "u128"
          },
          {
            "name": "last_update_ts",
            "docs": [
              "Timestamp the accumulator was last advanced to."
            ],
            "type": "i64"
          },
          {
            "name": "total_rewards_funded",
            "docs": [
              "Total ever deposited into the reward vault. Analytics only — see",
              "[`Self::unpaid_liability`] for why this must not be used as a liability."
            ],
            "type": "u64"
          },
          {
            "name": "total_rewards_accrued",
            "docs": [
              "Total that has become claimable by positions, accumulated as the",
              "accumulator advances. This is the real liability."
            ],
            "type": "u64"
          },
          {
            "name": "total_rewards_paid",
            "type": "u64"
          },
          {
            "name": "position_count",
            "docs": [
              "Monotonic counter used to seed position PDAs."
            ],
            "type": "u64"
          },
          {
            "name": "paused",
            "docs": [
              "Blocks new deposits only. Unstaking and claiming stay live — see",
              "`INVARIANTS.md` §6.4."
            ],
            "type": "bool"
          },
          {
            "name": "bump",
            "type": "u8"
          },
          {
            "name": "vault_authority_bump",
            "type": "u8"
          }
        ]
      }
    },
    {
      "name": "PoolInitialized",
      "type": {
        "kind": "struct",
        "fields": [
          {
            "name": "pool",
            "type": "pubkey"
          },
          {
            "name": "authority",
            "type": "pubkey"
          },
          {
            "name": "stake_mint",
            "type": "pubkey"
          },
          {
            "name": "reward_mint",
            "type": "pubkey"
          },
          {
            "name": "timestamp",
            "type": "i64"
          }
        ]
      }
    },
    {
      "name": "PoolPauseToggled",
      "type": {
        "kind": "struct",
        "fields": [
          {
            "name": "pool",
            "type": "pubkey"
          },
          {
            "name": "paused",
            "type": "bool"
          },
          {
            "name": "timestamp",
            "type": "i64"
          }
        ]
      }
    },
    {
      "name": "Position",
      "docs": [
        "One staked position. A user may hold several, in different tiers."
      ],
      "type": {
        "kind": "struct",
        "fields": [
          {
            "name": "pool",
            "type": "pubkey"
          },
          {
            "name": "owner",
            "type": "pubkey"
          },
          {
            "name": "position_id",
            "docs": [
              "Index within the pool, used in the PDA seeds. A `u64` in little-endian",
              "bytes rather than a caller-supplied byte string, so seeds are",
              "fixed-length and cannot be crafted to collide."
            ],
            "type": "u64"
          },
          {
            "name": "amount",
            "docs": [
              "Principal, in stake-token base units. For a fee-bearing Token-2022 mint",
              "this is the amount the vault *received*, not the amount sent."
            ],
            "type": "u64"
          },
          {
            "name": "weighted_amount",
            "docs": [
              "`amount` scaled by the tier multiplier."
            ],
            "type": "u64"
          },
          {
            "name": "tier",
            "type": {
              "defined": {
                "name": "LockTier"
              }
            }
          },
          {
            "name": "lock_end",
            "docs": [
              "Principal is withdrawable from this timestamp. Also the gate on voting:",
              "`helix-governance` accepts this position's weight only for proposals",
              "that close at or before `lock_end`."
            ],
            "type": "i64"
          },
          {
            "name": "reward_per_token_paid",
            "docs": [
              "The pool accumulator as of this position's last settlement."
            ],
            "type": "u128"
          },
          {
            "name": "pending_rewards",
            "docs": [
              "Rewards accrued but not yet withdrawn."
            ],
            "type": "u64"
          },
          {
            "name": "created_at",
            "type": "i64"
          },
          {
            "name": "bump",
            "type": "u8"
          }
        ]
      }
    },
    {
      "name": "PositionClosed",
      "docs": [
        "A fully exited position's account was deallocated and its rent returned.",
        "",
        "Carries `position_id` because the account it describes no longer exists —",
        "a consumer that wanted the id would otherwise have to have retained the",
        "`Staked` event that opened it. This is the same rule `Unstaked` produced:",
        "an event that cannot be folded into state without going elsewhere for a",
        "field is an incomplete event.",
        "",
        "Emphatically **not** a decrement of `pool.position_count`, which counts",
        "positions ever opened — see [`crate::instructions::close_position`]."
      ],
      "type": {
        "kind": "struct",
        "fields": [
          {
            "name": "pool",
            "type": "pubkey"
          },
          {
            "name": "position",
            "type": "pubkey"
          },
          {
            "name": "owner",
            "type": "pubkey"
          },
          {
            "name": "position_id",
            "type": "u64"
          },
          {
            "name": "timestamp",
            "type": "i64"
          }
        ]
      }
    },
    {
      "name": "RewardRateChanged",
      "type": {
        "kind": "struct",
        "fields": [
          {
            "name": "pool",
            "type": "pubkey"
          },
          {
            "name": "old_rate",
            "type": "u64"
          },
          {
            "name": "new_rate",
            "type": "u64"
          },
          {
            "name": "reward_period_end",
            "type": "i64"
          },
          {
            "name": "timestamp",
            "type": "i64"
          }
        ]
      }
    },
    {
      "name": "RewardsClaimed",
      "type": {
        "kind": "struct",
        "fields": [
          {
            "name": "pool",
            "type": "pubkey"
          },
          {
            "name": "position",
            "type": "pubkey"
          },
          {
            "name": "owner",
            "type": "pubkey"
          },
          {
            "name": "amount",
            "type": "u64"
          },
          {
            "name": "timestamp",
            "type": "i64"
          }
        ]
      }
    },
    {
      "name": "RewardsFunded",
      "type": {
        "kind": "struct",
        "fields": [
          {
            "name": "pool",
            "type": "pubkey"
          },
          {
            "name": "funder",
            "type": "pubkey"
          },
          {
            "name": "amount_credited",
            "type": "u64"
          },
          {
            "name": "total_funded",
            "type": "u64"
          },
          {
            "name": "timestamp",
            "type": "i64"
          }
        ]
      }
    },
    {
      "name": "Staked",
      "type": {
        "kind": "struct",
        "fields": [
          {
            "name": "pool",
            "type": "pubkey"
          },
          {
            "name": "position",
            "type": "pubkey"
          },
          {
            "name": "owner",
            "type": "pubkey"
          },
          {
            "name": "position_id",
            "type": "u64"
          },
          {
            "name": "amount_sent",
            "docs": [
              "What the caller sent."
            ],
            "type": "u64"
          },
          {
            "name": "amount_credited",
            "docs": [
              "What the vault actually received. These differ when the stake mint",
              "carries a Token-2022 transfer fee, and the credited figure is the one",
              "the position is built from."
            ],
            "type": "u64"
          },
          {
            "name": "weighted_amount",
            "type": "u64"
          },
          {
            "name": "tier",
            "type": {
              "defined": {
                "name": "LockTier"
              }
            }
          },
          {
            "name": "lock_end",
            "type": "i64"
          },
          {
            "name": "timestamp",
            "type": "i64"
          }
        ]
      }
    },
    {
      "name": "Unstaked",
      "type": {
        "kind": "struct",
        "fields": [
          {
            "name": "pool",
            "type": "pubkey"
          },
          {
            "name": "position",
            "type": "pubkey"
          },
          {
            "name": "owner",
            "type": "pubkey"
          },
          {
            "name": "amount",
            "type": "u64"
          },
          {
            "name": "remaining",
            "docs": [
              "Principal left in the position. Zero means it is fully exited, not that",
              "the account is gone — reclaiming the rent is a separate, optional step",
              "that emits [`PositionClosed`]."
            ],
            "type": "u64"
          },
          {
            "name": "weighted_amount",
            "docs": [
              "Vote weight left in the position, so a consumer never has to recompute",
              "it from `remaining` and the tier.",
              "",
              "Added when the indexer was built. Without it, reconstructing",
              "`pool.total_weighted` from the event stream means re-running",
              "`LockTier::apply_weight` off chain — a second implementation of the",
              "weight table that agrees with the program until the day the table",
              "changes, and then disagrees silently. An event that cannot be folded into",
              "state without recomputation is an incomplete event."
            ],
            "type": "u64"
          },
          {
            "name": "timestamp",
            "type": "i64"
          }
        ]
      }
    }
  ]
};

export default idl;
