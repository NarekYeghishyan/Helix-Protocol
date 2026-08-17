/**
 * Generated from `target/idl/helix_governance.json` by `node scripts/sync-idl.mjs`.
 *
 * Do not edit. `tests/integration/tests/idl_sync.rs` fails if this file is not
 * byte-for-byte what `anchor build` currently produces.
 */

import type { Idl } from "../lib/idl.ts";

const idl: Idl = {
  "address": "nSZnzJR8uUuZu8t1SqmLU2ExCvXNYABuVHwrDQJqSf5",
  "metadata": {
    "name": "helix_governance",
    "version": "0.1.0",
    "spec": "0.1.0",
    "description": "Proposal lifecycle, lock-gated voting, quorum and timelock.",
    "repository": "https://github.com/NarekYeghishyan/Helix-Protocol"
  },
  "instructions": [
    {
      "name": "activate_proposal",
      "docs": [
        "Opens voting and fixes the quorum denominator. Permissionless."
      ],
      "discriminator": [
        90,
        186,
        203,
        234,
        70,
        185,
        191,
        21
      ],
      "accounts": [
        {
          "name": "realm",
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  114,
                  101,
                  97,
                  108,
                  109
                ]
              },
              {
                "kind": "account",
                "path": "realm.staking_pool",
                "account": "Realm"
              }
            ]
          },
          "relations": [
            "proposal"
          ]
        },
        {
          "name": "proposal",
          "writable": true,
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  112,
                  114,
                  111,
                  112,
                  111,
                  115,
                  97,
                  108
                ]
              },
              {
                "kind": "account",
                "path": "realm"
              },
              {
                "kind": "account",
                "path": "proposal.id",
                "account": "Proposal"
              }
            ]
          }
        },
        {
          "name": "staking_pool",
          "docs": [
            "Read to snapshot `total_weighted` as the quorum denominator."
          ],
          "relations": [
            "realm"
          ]
        }
      ],
      "args": []
    },
    {
      "name": "cancel_proposal",
      "docs": [
        "Guardian veto. The guardian's only power."
      ],
      "discriminator": [
        106,
        74,
        128,
        146,
        19,
        65,
        39,
        23
      ],
      "accounts": [
        {
          "name": "realm",
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  114,
                  101,
                  97,
                  108,
                  109
                ]
              },
              {
                "kind": "account",
                "path": "realm.staking_pool",
                "account": "Realm"
              }
            ]
          },
          "relations": [
            "proposal"
          ]
        },
        {
          "name": "guardian",
          "signer": true,
          "relations": [
            "realm"
          ]
        },
        {
          "name": "proposal",
          "writable": true,
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  112,
                  114,
                  111,
                  112,
                  111,
                  115,
                  97,
                  108
                ]
              },
              {
                "kind": "account",
                "path": "realm"
              },
              {
                "kind": "account",
                "path": "proposal.id",
                "account": "Proposal"
              }
            ]
          }
        }
      ],
      "args": []
    },
    {
      "name": "cast_vote",
      "docs": [
        "Casts a position's weight. Requires `lock_end >= voting_ends_at`."
      ],
      "discriminator": [
        20,
        212,
        15,
        189,
        69,
        180,
        69,
        151
      ],
      "accounts": [
        {
          "name": "realm",
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  114,
                  101,
                  97,
                  108,
                  109
                ]
              },
              {
                "kind": "account",
                "path": "realm.staking_pool",
                "account": "Realm"
              }
            ]
          },
          "relations": [
            "proposal"
          ]
        },
        {
          "name": "proposal",
          "writable": true,
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  112,
                  114,
                  111,
                  112,
                  111,
                  115,
                  97,
                  108
                ]
              },
              {
                "kind": "account",
                "path": "realm"
              },
              {
                "kind": "account",
                "path": "proposal.id",
                "account": "Proposal"
              }
            ]
          }
        },
        {
          "name": "voter",
          "writable": true,
          "signer": true
        },
        {
          "name": "position",
          "docs": [
            "The staking position whose weight is being cast.",
            "",
            "Typed as `Account<Position>`, so Anchor verifies the account is owned by",
            "the staking program before this handler sees it — a look-alike account",
            "with a forged `weighted_amount` cannot be substituted."
          ]
        },
        {
          "name": "vote_record",
          "docs": [
            "One record per (proposal, position). `init` makes a second vote from the",
            "same position fail at account creation, before any handler logic runs —",
            "double voting is structurally impossible rather than checked for."
          ],
          "writable": true,
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  118,
                  111,
                  116,
                  101
                ]
              },
              {
                "kind": "account",
                "path": "proposal"
              },
              {
                "kind": "account",
                "path": "position"
              }
            ]
          }
        },
        {
          "name": "system_program",
          "address": "11111111111111111111111111111111"
        }
      ],
      "args": [
        {
          "name": "choice",
          "type": {
            "defined": {
              "name": "VoteChoice"
            }
          }
        }
      ]
    },
    {
      "name": "create_proposal",
      "docs": [
        "Creates a proposal in `Draft`. Requires `min_weight_to_propose`."
      ],
      "discriminator": [
        132,
        116,
        68,
        174,
        216,
        160,
        198,
        22
      ],
      "accounts": [
        {
          "name": "realm",
          "writable": true,
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  114,
                  101,
                  97,
                  108,
                  109
                ]
              },
              {
                "kind": "account",
                "path": "realm.staking_pool",
                "account": "Realm"
              }
            ]
          }
        },
        {
          "name": "proposer",
          "writable": true,
          "signer": true
        },
        {
          "name": "proposer_position",
          "docs": [
            "A position held by the proposer, used only to prove they meet",
            "`min_weight_to_propose`. It is not consumed and does not vote."
          ]
        },
        {
          "name": "owner",
          "docs": [
            "the signer, which is enforced in the handler."
          ],
          "relations": [
            "proposer_position"
          ]
        },
        {
          "name": "proposal",
          "writable": true,
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  112,
                  114,
                  111,
                  112,
                  111,
                  115,
                  97,
                  108
                ]
              },
              {
                "kind": "account",
                "path": "realm"
              },
              {
                "kind": "arg",
                "path": "proposal_id"
              }
            ]
          }
        },
        {
          "name": "system_program",
          "address": "11111111111111111111111111111111"
        }
      ],
      "args": [
        {
          "name": "proposal_id",
          "type": "u64"
        },
        {
          "name": "action",
          "type": {
            "defined": {
              "name": "ProposalAction"
            }
          }
        },
        {
          "name": "title",
          "type": "string"
        },
        {
          "name": "descriptor_uri",
          "type": "string"
        }
      ]
    },
    {
      "name": "execute_accept_token_manager_admin",
      "docs": [
        "Completes the admin handover, making this realm the mint's admin."
      ],
      "discriminator": [
        247,
        149,
        91,
        66,
        132,
        40,
        46,
        183
      ],
      "accounts": [
        {
          "name": "realm",
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  114,
                  101,
                  97,
                  108,
                  109
                ]
              },
              {
                "kind": "account",
                "path": "realm.staking_pool",
                "account": "Realm"
              }
            ]
          },
          "relations": [
            "proposal"
          ]
        },
        {
          "name": "proposal",
          "writable": true,
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  112,
                  114,
                  111,
                  112,
                  111,
                  115,
                  97,
                  108
                ]
              },
              {
                "kind": "account",
                "path": "realm"
              },
              {
                "kind": "account",
                "path": "proposal.id",
                "account": "Proposal"
              }
            ]
          }
        },
        {
          "name": "executor",
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  101,
                  120,
                  101,
                  99,
                  117,
                  116,
                  111,
                  114
                ]
              },
              {
                "kind": "account",
                "path": "realm"
              }
            ]
          }
        },
        {
          "name": "token_config",
          "writable": true
        },
        {
          "name": "token_manager_program",
          "address": "5RU35Eni3MxkuSc9Zv5xm8LLd2QX85XdbYjRUaLkFRFr"
        }
      ],
      "args": []
    },
    {
      "name": "execute_create_vesting_stream",
      "docs": [
        "Creates the vesting stream a passed proposal called for. `stream_id` must",
        "equal the treasury's current `stream_count`, which the treasury verifies."
      ],
      "discriminator": [
        6,
        211,
        130,
        97,
        119,
        209,
        98,
        56
      ],
      "accounts": [
        {
          "name": "realm",
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  114,
                  101,
                  97,
                  108,
                  109
                ]
              },
              {
                "kind": "account",
                "path": "realm.staking_pool",
                "account": "Realm"
              }
            ]
          },
          "relations": [
            "proposal"
          ]
        },
        {
          "name": "proposal",
          "writable": true,
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  112,
                  114,
                  111,
                  112,
                  111,
                  115,
                  97,
                  108
                ]
              },
              {
                "kind": "account",
                "path": "realm"
              },
              {
                "kind": "account",
                "path": "proposal.id",
                "account": "Proposal"
              }
            ]
          }
        },
        {
          "name": "executor",
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  101,
                  120,
                  101,
                  99,
                  117,
                  116,
                  111,
                  114
                ]
              },
              {
                "kind": "account",
                "path": "realm"
              }
            ]
          }
        },
        {
          "name": "payer",
          "docs": [
            "Pays rent for the new stream account."
          ],
          "writable": true,
          "signer": true
        },
        {
          "name": "treasury",
          "writable": true
        },
        {
          "name": "beneficiary",
          "docs": [
            "checked against the proposal's action below, so it cannot be substituted."
          ]
        },
        {
          "name": "stream",
          "docs": [
            "PDA from `stream_id` under its own seeds constraint."
          ],
          "writable": true
        },
        {
          "name": "treasury_vault"
        },
        {
          "name": "system_program",
          "address": "11111111111111111111111111111111"
        },
        {
          "name": "treasury_program",
          "address": "B9HenpXUQzzGdT7mv93MQM8f6ytdPRKhJCbdx1CcBvdh"
        }
      ],
      "args": [
        {
          "name": "stream_id",
          "type": "u64"
        }
      ]
    },
    {
      "name": "execute_propose_token_admin",
      "docs": [
        "Begins handing the admin role onward; the successor must still accept."
      ],
      "discriminator": [
        216,
        59,
        237,
        172,
        104,
        38,
        212,
        247
      ],
      "accounts": [
        {
          "name": "realm",
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  114,
                  101,
                  97,
                  108,
                  109
                ]
              },
              {
                "kind": "account",
                "path": "realm.staking_pool",
                "account": "Realm"
              }
            ]
          },
          "relations": [
            "proposal"
          ]
        },
        {
          "name": "proposal",
          "writable": true,
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  112,
                  114,
                  111,
                  112,
                  111,
                  115,
                  97,
                  108
                ]
              },
              {
                "kind": "account",
                "path": "realm"
              },
              {
                "kind": "account",
                "path": "proposal.id",
                "account": "Proposal"
              }
            ]
          }
        },
        {
          "name": "executor",
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  101,
                  120,
                  101,
                  99,
                  117,
                  116,
                  111,
                  114
                ]
              },
              {
                "kind": "account",
                "path": "realm"
              }
            ]
          }
        },
        {
          "name": "token_config",
          "writable": true
        },
        {
          "name": "token_manager_program",
          "address": "5RU35Eni3MxkuSc9Zv5xm8LLd2QX85XdbYjRUaLkFRFr"
        }
      ],
      "args": []
    },
    {
      "name": "execute_register_minter",
      "docs": [
        "Registers a minter with a per-epoch issuance cap."
      ],
      "discriminator": [
        18,
        154,
        218,
        109,
        95,
        165,
        18,
        24
      ],
      "accounts": [
        {
          "name": "realm",
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  114,
                  101,
                  97,
                  108,
                  109
                ]
              },
              {
                "kind": "account",
                "path": "realm.staking_pool",
                "account": "Realm"
              }
            ]
          },
          "relations": [
            "proposal"
          ]
        },
        {
          "name": "proposal",
          "writable": true,
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  112,
                  114,
                  111,
                  112,
                  111,
                  115,
                  97,
                  108
                ]
              },
              {
                "kind": "account",
                "path": "realm"
              },
              {
                "kind": "account",
                "path": "proposal.id",
                "account": "Proposal"
              }
            ]
          }
        },
        {
          "name": "executor",
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  101,
                  120,
                  101,
                  99,
                  117,
                  116,
                  111,
                  114
                ]
              },
              {
                "kind": "account",
                "path": "realm"
              }
            ]
          }
        },
        {
          "name": "payer",
          "docs": [
            "Pays rent for the new minter account."
          ],
          "writable": true,
          "signer": true
        },
        {
          "name": "token_config",
          "writable": true
        },
        {
          "name": "minter",
          "docs": [
            "under its own seeds constraint."
          ],
          "writable": true
        },
        {
          "name": "system_program",
          "address": "11111111111111111111111111111111"
        },
        {
          "name": "token_manager_program",
          "address": "5RU35Eni3MxkuSc9Zv5xm8LLd2QX85XdbYjRUaLkFRFr"
        }
      ],
      "args": []
    },
    {
      "name": "execute_revoke_minter",
      "docs": [
        "Permanently disables a minter, retaining its issuance history."
      ],
      "discriminator": [
        72,
        52,
        174,
        11,
        221,
        227,
        47,
        143
      ],
      "accounts": [
        {
          "name": "realm",
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  114,
                  101,
                  97,
                  108,
                  109
                ]
              },
              {
                "kind": "account",
                "path": "realm.staking_pool",
                "account": "Realm"
              }
            ]
          },
          "relations": [
            "proposal"
          ]
        },
        {
          "name": "proposal",
          "writable": true,
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  112,
                  114,
                  111,
                  112,
                  111,
                  115,
                  97,
                  108
                ]
              },
              {
                "kind": "account",
                "path": "realm"
              },
              {
                "kind": "account",
                "path": "proposal.id",
                "account": "Proposal"
              }
            ]
          }
        },
        {
          "name": "executor",
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  101,
                  120,
                  101,
                  99,
                  117,
                  116,
                  111,
                  114
                ]
              },
              {
                "kind": "account",
                "path": "realm"
              }
            ]
          }
        },
        {
          "name": "token_config",
          "writable": true
        },
        {
          "name": "minter",
          "writable": true
        },
        {
          "name": "token_manager_program",
          "address": "5RU35Eni3MxkuSc9Zv5xm8LLd2QX85XdbYjRUaLkFRFr"
        }
      ],
      "args": []
    },
    {
      "name": "execute_revoke_vesting_stream",
      "docs": [
        "Revokes a vesting stream. Already-vested tokens stay claimable."
      ],
      "discriminator": [
        214,
        195,
        86,
        83,
        224,
        27,
        114,
        138
      ],
      "accounts": [
        {
          "name": "realm",
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  114,
                  101,
                  97,
                  108,
                  109
                ]
              },
              {
                "kind": "account",
                "path": "realm.staking_pool",
                "account": "Realm"
              }
            ]
          },
          "relations": [
            "proposal"
          ]
        },
        {
          "name": "proposal",
          "writable": true,
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  112,
                  114,
                  111,
                  112,
                  111,
                  115,
                  97,
                  108
                ]
              },
              {
                "kind": "account",
                "path": "realm"
              },
              {
                "kind": "account",
                "path": "proposal.id",
                "account": "Proposal"
              }
            ]
          }
        },
        {
          "name": "executor",
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  101,
                  120,
                  101,
                  99,
                  117,
                  116,
                  111,
                  114
                ]
              },
              {
                "kind": "account",
                "path": "realm"
              }
            ]
          }
        },
        {
          "name": "treasury",
          "writable": true
        },
        {
          "name": "stream",
          "docs": [
            "Typed rather than unchecked, so the stream's own `stream_id` can be",
            "compared against the one the proposal named."
          ],
          "writable": true
        },
        {
          "name": "treasury_program",
          "address": "B9HenpXUQzzGdT7mv93MQM8f6ytdPRKhJCbdx1CcBvdh"
        }
      ],
      "args": []
    },
    {
      "name": "execute_set_governance_executor",
      "docs": [
        "Hands treasury spending rights to a different governance executor."
      ],
      "discriminator": [
        172,
        198,
        170,
        65,
        205,
        241,
        149,
        12
      ],
      "accounts": [
        {
          "name": "realm",
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  114,
                  101,
                  97,
                  108,
                  109
                ]
              },
              {
                "kind": "account",
                "path": "realm.staking_pool",
                "account": "Realm"
              }
            ]
          },
          "relations": [
            "proposal"
          ]
        },
        {
          "name": "proposal",
          "writable": true,
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  112,
                  114,
                  111,
                  112,
                  111,
                  115,
                  97,
                  108
                ]
              },
              {
                "kind": "account",
                "path": "realm"
              },
              {
                "kind": "account",
                "path": "proposal.id",
                "account": "Proposal"
              }
            ]
          }
        },
        {
          "name": "executor",
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  101,
                  120,
                  101,
                  99,
                  117,
                  116,
                  111,
                  114
                ]
              },
              {
                "kind": "account",
                "path": "realm"
              }
            ]
          }
        },
        {
          "name": "treasury",
          "writable": true
        },
        {
          "name": "treasury_program",
          "address": "B9HenpXUQzzGdT7mv93MQM8f6ytdPRKhJCbdx1CcBvdh"
        }
      ],
      "args": []
    },
    {
      "name": "execute_set_realm_authority",
      "docs": [
        "Moves `realm.authority`, in practice to the realm's own executor PDA.",
        "This is the migration that makes the realm answer only to itself."
      ],
      "discriminator": [
        39,
        83,
        240,
        197,
        21,
        128,
        20,
        206
      ],
      "accounts": [
        {
          "name": "realm",
          "writable": true,
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  114,
                  101,
                  97,
                  108,
                  109
                ]
              },
              {
                "kind": "account",
                "path": "realm.staking_pool",
                "account": "Realm"
              }
            ]
          },
          "relations": [
            "proposal"
          ]
        },
        {
          "name": "proposal",
          "writable": true,
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  112,
                  114,
                  111,
                  112,
                  111,
                  115,
                  97,
                  108
                ]
              },
              {
                "kind": "account",
                "path": "realm"
              },
              {
                "kind": "account",
                "path": "proposal.id",
                "account": "Proposal"
              }
            ]
          }
        },
        {
          "name": "executor",
          "docs": [
            "attributed to this realm's executor even though no CPI is made."
          ],
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  101,
                  120,
                  101,
                  99,
                  117,
                  116,
                  111,
                  114
                ]
              },
              {
                "kind": "account",
                "path": "realm"
              }
            ]
          }
        }
      ],
      "args": []
    },
    {
      "name": "execute_set_staking_reward_rate",
      "docs": [
        "Executes a staking emission change."
      ],
      "discriminator": [
        224,
        26,
        118,
        36,
        11,
        212,
        168,
        235
      ],
      "accounts": [
        {
          "name": "realm",
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  114,
                  101,
                  97,
                  108,
                  109
                ]
              },
              {
                "kind": "account",
                "path": "realm.staking_pool",
                "account": "Realm"
              }
            ]
          },
          "relations": [
            "proposal"
          ]
        },
        {
          "name": "proposal",
          "writable": true,
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  112,
                  114,
                  111,
                  112,
                  111,
                  115,
                  97,
                  108
                ]
              },
              {
                "kind": "account",
                "path": "realm"
              },
              {
                "kind": "account",
                "path": "proposal.id",
                "account": "Proposal"
              }
            ]
          }
        },
        {
          "name": "executor",
          "docs": [
            "the staking program checks when the CPI lands."
          ],
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  101,
                  120,
                  101,
                  99,
                  117,
                  116,
                  111,
                  114
                ]
              },
              {
                "kind": "account",
                "path": "realm"
              }
            ]
          }
        },
        {
          "name": "staking_pool",
          "writable": true,
          "relations": [
            "realm"
          ]
        },
        {
          "name": "reward_vault"
        },
        {
          "name": "staking_program",
          "address": "9RuZJZpgCwbiF9JRAsyR8cqDhFSaFYus1mzobKzEZzP3"
        }
      ],
      "args": []
    },
    {
      "name": "execute_set_token_paused",
      "docs": [
        "Halts or resumes HLX issuance. Never blocks burning."
      ],
      "discriminator": [
        183,
        75,
        71,
        177,
        197,
        218,
        212,
        247
      ],
      "accounts": [
        {
          "name": "realm",
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  114,
                  101,
                  97,
                  108,
                  109
                ]
              },
              {
                "kind": "account",
                "path": "realm.staking_pool",
                "account": "Realm"
              }
            ]
          },
          "relations": [
            "proposal"
          ]
        },
        {
          "name": "proposal",
          "writable": true,
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  112,
                  114,
                  111,
                  112,
                  111,
                  115,
                  97,
                  108
                ]
              },
              {
                "kind": "account",
                "path": "realm"
              },
              {
                "kind": "account",
                "path": "proposal.id",
                "account": "Proposal"
              }
            ]
          }
        },
        {
          "name": "executor",
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  101,
                  120,
                  101,
                  99,
                  117,
                  116,
                  111,
                  114
                ]
              },
              {
                "kind": "account",
                "path": "realm"
              }
            ]
          }
        },
        {
          "name": "token_config",
          "writable": true
        },
        {
          "name": "token_manager_program",
          "address": "5RU35Eni3MxkuSc9Zv5xm8LLd2QX85XdbYjRUaLkFRFr"
        }
      ],
      "args": []
    },
    {
      "name": "execute_set_treasury_spend_cap",
      "docs": [
        "Adjusts the treasury's per-epoch spend cap."
      ],
      "discriminator": [
        204,
        6,
        229,
        143,
        39,
        165,
        82,
        234
      ],
      "accounts": [
        {
          "name": "realm",
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  114,
                  101,
                  97,
                  108,
                  109
                ]
              },
              {
                "kind": "account",
                "path": "realm.staking_pool",
                "account": "Realm"
              }
            ]
          },
          "relations": [
            "proposal"
          ]
        },
        {
          "name": "proposal",
          "writable": true,
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  112,
                  114,
                  111,
                  112,
                  111,
                  115,
                  97,
                  108
                ]
              },
              {
                "kind": "account",
                "path": "realm"
              },
              {
                "kind": "account",
                "path": "proposal.id",
                "account": "Proposal"
              }
            ]
          }
        },
        {
          "name": "executor",
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  101,
                  120,
                  101,
                  99,
                  117,
                  116,
                  111,
                  114
                ]
              },
              {
                "kind": "account",
                "path": "realm"
              }
            ]
          }
        },
        {
          "name": "treasury",
          "writable": true
        },
        {
          "name": "treasury_program",
          "address": "B9HenpXUQzzGdT7mv93MQM8f6ytdPRKhJCbdx1CcBvdh"
        }
      ],
      "args": []
    },
    {
      "name": "execute_signal",
      "docs": [
        "Executes a signalling proposal."
      ],
      "discriminator": [
        57,
        236,
        108,
        57,
        15,
        111,
        91,
        118
      ],
      "accounts": [
        {
          "name": "realm",
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  114,
                  101,
                  97,
                  108,
                  109
                ]
              },
              {
                "kind": "account",
                "path": "realm.staking_pool",
                "account": "Realm"
              }
            ]
          },
          "relations": [
            "proposal"
          ]
        },
        {
          "name": "proposal",
          "writable": true,
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  112,
                  114,
                  111,
                  112,
                  111,
                  115,
                  97,
                  108
                ]
              },
              {
                "kind": "account",
                "path": "realm"
              },
              {
                "kind": "account",
                "path": "proposal.id",
                "account": "Proposal"
              }
            ]
          }
        }
      ],
      "args": []
    },
    {
      "name": "execute_treasury_transfer",
      "docs": [
        "Executes a treasury transfer, CPI-signing as the realm executor."
      ],
      "discriminator": [
        168,
        161,
        111,
        154,
        196,
        116,
        197,
        137
      ],
      "accounts": [
        {
          "name": "realm",
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  114,
                  101,
                  97,
                  108,
                  109
                ]
              },
              {
                "kind": "account",
                "path": "realm.staking_pool",
                "account": "Realm"
              }
            ]
          },
          "relations": [
            "proposal"
          ]
        },
        {
          "name": "proposal",
          "writable": true,
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  112,
                  114,
                  111,
                  112,
                  111,
                  115,
                  97,
                  108
                ]
              },
              {
                "kind": "account",
                "path": "realm"
              },
              {
                "kind": "account",
                "path": "proposal.id",
                "account": "Proposal"
              }
            ]
          }
        },
        {
          "name": "executor",
          "docs": [
            "constraint; it is never deserialised, only used as a signer. The treasury",
            "program accepts nothing else, so producing this signature here is exactly",
            "what \"governance approved the spend\" means."
          ],
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  101,
                  120,
                  101,
                  99,
                  117,
                  116,
                  111,
                  114
                ]
              },
              {
                "kind": "account",
                "path": "realm"
              }
            ]
          }
        },
        {
          "name": "treasury",
          "writable": true
        },
        {
          "name": "treasury_mint"
        },
        {
          "name": "treasury_vault",
          "writable": true
        },
        {
          "name": "destination",
          "writable": true
        },
        {
          "name": "treasury_vault_authority",
          "docs": [
            "program's own seeds constraint when the CPI lands."
          ]
        },
        {
          "name": "token_program"
        },
        {
          "name": "treasury_program",
          "address": "B9HenpXUQzzGdT7mv93MQM8f6ytdPRKhJCbdx1CcBvdh"
        }
      ],
      "args": []
    },
    {
      "name": "execute_update_minter",
      "docs": [
        "Adjusts a minter's cap, or enables/disables it."
      ],
      "discriminator": [
        154,
        222,
        114,
        64,
        73,
        123,
        84,
        6
      ],
      "accounts": [
        {
          "name": "realm",
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  114,
                  101,
                  97,
                  108,
                  109
                ]
              },
              {
                "kind": "account",
                "path": "realm.staking_pool",
                "account": "Realm"
              }
            ]
          },
          "relations": [
            "proposal"
          ]
        },
        {
          "name": "proposal",
          "writable": true,
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  112,
                  114,
                  111,
                  112,
                  111,
                  115,
                  97,
                  108
                ]
              },
              {
                "kind": "account",
                "path": "realm"
              },
              {
                "kind": "account",
                "path": "proposal.id",
                "account": "Proposal"
              }
            ]
          }
        },
        {
          "name": "executor",
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  101,
                  120,
                  101,
                  99,
                  117,
                  116,
                  111,
                  114
                ]
              },
              {
                "kind": "account",
                "path": "realm"
              }
            ]
          }
        },
        {
          "name": "token_config",
          "writable": true
        },
        {
          "name": "minter",
          "writable": true
        },
        {
          "name": "token_manager_program",
          "address": "5RU35Eni3MxkuSc9Zv5xm8LLd2QX85XdbYjRUaLkFRFr"
        }
      ],
      "args": []
    },
    {
      "name": "execute_update_realm_params",
      "docs": [
        "Retunes the realm's own parameters — quorum, approval, periods,",
        "proposal threshold — through a passed, timelocked proposal."
      ],
      "discriminator": [
        90,
        145,
        95,
        141,
        29,
        237,
        246,
        134
      ],
      "accounts": [
        {
          "name": "realm",
          "writable": true,
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  114,
                  101,
                  97,
                  108,
                  109
                ]
              },
              {
                "kind": "account",
                "path": "realm.staking_pool",
                "account": "Realm"
              }
            ]
          },
          "relations": [
            "proposal"
          ]
        },
        {
          "name": "proposal",
          "writable": true,
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  112,
                  114,
                  111,
                  112,
                  111,
                  115,
                  97,
                  108
                ]
              },
              {
                "kind": "account",
                "path": "realm"
              },
              {
                "kind": "account",
                "path": "proposal.id",
                "account": "Proposal"
              }
            ]
          }
        },
        {
          "name": "executor",
          "docs": [
            "attributed to this realm's executor even though no CPI is made."
          ],
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  101,
                  120,
                  101,
                  99,
                  117,
                  116,
                  111,
                  114
                ]
              },
              {
                "kind": "account",
                "path": "realm"
              }
            ]
          }
        }
      ],
      "args": []
    },
    {
      "name": "finalize_proposal",
      "docs": [
        "Resolves a closed vote. Permissionless."
      ],
      "discriminator": [
        23,
        68,
        51,
        167,
        109,
        173,
        187,
        164
      ],
      "accounts": [
        {
          "name": "realm",
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  114,
                  101,
                  97,
                  108,
                  109
                ]
              },
              {
                "kind": "account",
                "path": "realm.staking_pool",
                "account": "Realm"
              }
            ]
          },
          "relations": [
            "proposal"
          ]
        },
        {
          "name": "proposal",
          "writable": true,
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  112,
                  114,
                  111,
                  112,
                  111,
                  115,
                  97,
                  108
                ]
              },
              {
                "kind": "account",
                "path": "realm"
              },
              {
                "kind": "account",
                "path": "proposal.id",
                "account": "Proposal"
              }
            ]
          }
        }
      ],
      "args": []
    },
    {
      "name": "initialize_realm",
      "docs": [
        "Creates a realm governing one staking pool."
      ],
      "discriminator": [
        252,
        61,
        206,
        50,
        109,
        163,
        242,
        27
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
            "executor PDA, so parameter changes go through a vote like anything else."
          ]
        },
        {
          "name": "guardian"
        },
        {
          "name": "staking_pool",
          "docs": [
            "The pool whose positions confer vote weight. Typed, so an account that is",
            "not actually a staking pool cannot be passed."
          ]
        },
        {
          "name": "realm",
          "writable": true,
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  114,
                  101,
                  97,
                  108,
                  109
                ]
              },
              {
                "kind": "account",
                "path": "staking_pool"
              }
            ]
          }
        },
        {
          "name": "executor",
          "docs": [
            "fixed by seeds. Possession of this PDA is the right to spend the treasury,",
            "and only `execute_*` can produce it."
          ],
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  101,
                  120,
                  101,
                  99,
                  117,
                  116,
                  111,
                  114
                ]
              },
              {
                "kind": "account",
                "path": "realm"
              }
            ]
          }
        },
        {
          "name": "system_program",
          "address": "11111111111111111111111111111111"
        }
      ],
      "args": [
        {
          "name": "params",
          "type": {
            "defined": {
              "name": "RealmParams"
            }
          }
        }
      ]
    },
    {
      "name": "queue_proposal",
      "docs": [
        "Moves a passed proposal into the timelock."
      ],
      "discriminator": [
        168,
        219,
        139,
        211,
        205,
        152,
        125,
        110
      ],
      "accounts": [
        {
          "name": "realm",
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  114,
                  101,
                  97,
                  108,
                  109
                ]
              },
              {
                "kind": "account",
                "path": "realm.staking_pool",
                "account": "Realm"
              }
            ]
          },
          "relations": [
            "proposal"
          ]
        },
        {
          "name": "proposal",
          "writable": true,
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  112,
                  114,
                  111,
                  112,
                  111,
                  115,
                  97,
                  108
                ]
              },
              {
                "kind": "account",
                "path": "realm"
              },
              {
                "kind": "account",
                "path": "proposal.id",
                "account": "Proposal"
              }
            ]
          }
        }
      ],
      "args": []
    },
    {
      "name": "update_realm_params",
      "docs": [
        "Changes governance parameters. Does not affect already-queued proposals."
      ],
      "discriminator": [
        115,
        76,
        36,
        219,
        253,
        67,
        197,
        23
      ],
      "accounts": [
        {
          "name": "realm",
          "writable": true,
          "pda": {
            "seeds": [
              {
                "kind": "const",
                "value": [
                  114,
                  101,
                  97,
                  108,
                  109
                ]
              },
              {
                "kind": "account",
                "path": "realm.staking_pool",
                "account": "Realm"
              }
            ]
          }
        },
        {
          "name": "authority",
          "signer": true,
          "relations": [
            "realm"
          ]
        }
      ],
      "args": [
        {
          "name": "params",
          "type": {
            "defined": {
              "name": "RealmParams"
            }
          }
        }
      ]
    }
  ],
  "accounts": [
    {
      "name": "Proposal",
      "discriminator": [
        26,
        94,
        189,
        187,
        116,
        136,
        53,
        33
      ]
    },
    {
      "name": "Realm",
      "discriminator": [
        197,
        236,
        164,
        155,
        120,
        56,
        152,
        56
      ]
    },
    {
      "name": "VoteRecord",
      "discriminator": [
        112,
        9,
        123,
        165,
        234,
        9,
        157,
        167
      ]
    }
  ],
  "events": [
    {
      "name": "ProposalActivated",
      "discriminator": [
        165,
        205,
        106,
        34,
        20,
        77,
        79,
        219
      ]
    },
    {
      "name": "ProposalCancelled",
      "discriminator": [
        253,
        59,
        104,
        46,
        129,
        78,
        9,
        14
      ]
    },
    {
      "name": "ProposalCreated",
      "discriminator": [
        186,
        8,
        160,
        108,
        81,
        13,
        51,
        206
      ]
    },
    {
      "name": "ProposalExecuted",
      "discriminator": [
        92,
        213,
        189,
        201,
        101,
        83,
        111,
        83
      ]
    },
    {
      "name": "ProposalFinalized",
      "discriminator": [
        159,
        104,
        210,
        220,
        86,
        209,
        61,
        51
      ]
    },
    {
      "name": "ProposalQueued",
      "discriminator": [
        127,
        31,
        107,
        17,
        2,
        119,
        72,
        39
      ]
    },
    {
      "name": "RealmAuthorityChanged",
      "discriminator": [
        86,
        89,
        168,
        93,
        125,
        45,
        136,
        228
      ]
    },
    {
      "name": "RealmInitialized",
      "discriminator": [
        58,
        2,
        148,
        228,
        11,
        49,
        40,
        214
      ]
    },
    {
      "name": "RealmParamsUpdated",
      "discriminator": [
        73,
        214,
        44,
        235,
        83,
        134,
        163,
        233
      ]
    },
    {
      "name": "VoteCast",
      "discriminator": [
        39,
        53,
        195,
        104,
        188,
        17,
        225,
        213
      ]
    }
  ],
  "errors": [
    {
      "code": 6000,
      "name": "NotAuthority",
      "msg": "Caller is not the realm authority"
    },
    {
      "code": 6001,
      "name": "NotGuardian",
      "msg": "Caller is not the realm guardian"
    },
    {
      "code": 6002,
      "name": "InvalidVotingPeriod",
      "msg": "Voting period is outside the permitted range"
    },
    {
      "code": 6003,
      "name": "InvalidTimelockDelay",
      "msg": "Timelock delay is outside the permitted range"
    },
    {
      "code": 6004,
      "name": "InvalidQuorum",
      "msg": "Quorum must be between 1 and 10000 basis points"
    },
    {
      "code": 6005,
      "name": "InvalidApprovalThreshold",
      "msg": "Approval threshold must be a simple majority or greater"
    },
    {
      "code": 6006,
      "name": "TextTooLong",
      "msg": "Title or URI exceeds its maximum length"
    },
    {
      "code": 6007,
      "name": "InvalidProposalState",
      "msg": "Proposal is not in the required state for this action"
    },
    {
      "code": 6008,
      "name": "VotingNotStarted",
      "msg": "Voting has not started"
    },
    {
      "code": 6009,
      "name": "VotingEnded",
      "msg": "Voting has ended"
    },
    {
      "code": 6010,
      "name": "VotingStillOpen",
      "msg": "Voting is still open"
    },
    {
      "code": 6011,
      "name": "NotPositionOwner",
      "msg": "Position does not belong to the voter"
    },
    {
      "code": 6012,
      "name": "PoolMismatch",
      "msg": "Position belongs to a different staking pool than this realm"
    },
    {
      "code": 6013,
      "name": "InsufficientLockDuration",
      "msg": "Position lock expires before voting closes, so it carries no weight"
    },
    {
      "code": 6014,
      "name": "ZeroWeight",
      "msg": "Position carries zero weight"
    },
    {
      "code": 6015,
      "name": "PositionNotInSnapshot",
      "msg": "Position was opened after the proposal's weight snapshot was taken"
    },
    {
      "code": 6016,
      "name": "BelowProposalThreshold",
      "msg": "Proposer does not meet the minimum weight to create a proposal"
    },
    {
      "code": 6017,
      "name": "TimelockNotElapsed",
      "msg": "Timelock has not elapsed"
    },
    {
      "code": 6018,
      "name": "ProposalExpired",
      "msg": "Proposal has expired and can no longer be executed"
    },
    {
      "code": 6019,
      "name": "MissingSnapshot",
      "msg": "Proposal has no snapshot of total voting weight"
    },
    {
      "code": 6020,
      "name": "ActionAccountMismatch",
      "msg": "Accounts supplied do not match the proposal's action"
    },
    {
      "code": 6021,
      "name": "MathOverflow",
      "msg": "Arithmetic overflow"
    },
    {
      "code": 6022,
      "name": "UnexpectedProposalId",
      "msg": "proposal_id must equal the realm's current proposal_count"
    }
  ],
  "types": [
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
      "name": "Minter",
      "docs": [
        "A registered issuer of HLX. PDA: `[\"minter\", config, authority]`.",
        "",
        "The registry is what makes the mint authority safe to hold in a PDA: the PDA",
        "will sign an issuance, but only on behalf of an authority recorded here and",
        "only within that authority's cap for the current epoch. In the deployed",
        "system the staking program's reward PDA is the only entry."
      ],
      "type": {
        "kind": "struct",
        "fields": [
          {
            "name": "config",
            "docs": [
              "The config this entry belongs to. Checked with `has_one` so a minter",
              "from one deployment cannot be replayed against another."
            ],
            "type": "pubkey"
          },
          {
            "name": "authority",
            "docs": [
              "The signer permitted to request issuance through this entry."
            ],
            "type": "pubkey"
          },
          {
            "name": "epoch_cap",
            "docs": [
              "Maximum that may be minted through this entry per epoch."
            ],
            "type": "u64"
          },
          {
            "name": "minted_this_epoch",
            "docs": [
              "Amount minted so far in [`Self::current_epoch`]."
            ],
            "type": "u64"
          },
          {
            "name": "current_epoch",
            "docs": [
              "Index of the epoch [`Self::minted_this_epoch`] refers to, derived from",
              "the chain clock as `unix_timestamp / epoch_duration`."
            ],
            "type": "u64"
          },
          {
            "name": "epoch_duration",
            "docs": [
              "Length of an issuance epoch in seconds."
            ],
            "type": "i64"
          },
          {
            "name": "total_minted",
            "docs": [
              "Lifetime issuance through this entry."
            ],
            "type": "u64"
          },
          {
            "name": "enabled",
            "docs": [
              "Cleared by `revoke_minter`. Revocation disables rather than closes, so",
              "the historical `total_minted` stays auditable on chain."
            ],
            "type": "bool"
          },
          {
            "name": "bump",
            "type": "u8"
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
      "name": "Proposal",
      "type": {
        "kind": "struct",
        "fields": [
          {
            "name": "realm",
            "type": "pubkey"
          },
          {
            "name": "proposer",
            "type": "pubkey"
          },
          {
            "name": "id",
            "type": "u64"
          },
          {
            "name": "state",
            "type": {
              "defined": {
                "name": "ProposalState"
              }
            }
          },
          {
            "name": "action",
            "type": {
              "defined": {
                "name": "ProposalAction"
              }
            }
          },
          {
            "name": "title",
            "type": "string"
          },
          {
            "name": "descriptor_uri",
            "docs": [
              "Off-chain discussion / full text. On-chain storage is expensive and",
              "rationale is not something a program needs to read."
            ],
            "type": "string"
          },
          {
            "name": "created_at",
            "type": "i64"
          },
          {
            "name": "voting_starts_at",
            "type": "i64"
          },
          {
            "name": "voting_ends_at",
            "type": "i64"
          },
          {
            "name": "eta",
            "docs": [
              "Earliest execution time. Zero until queued."
            ],
            "type": "i64"
          },
          {
            "name": "for_votes",
            "type": "u64"
          },
          {
            "name": "against_votes",
            "type": "u64"
          },
          {
            "name": "abstain_votes",
            "type": "u64"
          },
          {
            "name": "total_weight_snapshot",
            "docs": [
              "`pool.total_weighted` at activation — the quorum denominator.",
              "",
              "Fixed at activation rather than read live at finalisation, so a whale",
              "cannot defeat a proposal by staking more (inflating the denominator)",
              "after seeing how the vote is going."
            ],
            "type": "u64"
          },
          {
            "name": "position_count_snapshot",
            "docs": [
              "`pool.position_count` at activation — which positions the snapshot above",
              "was measured over.",
              "",
              "Position ids come from a pool-wide monotonic counter, so",
              "`position_id < position_count_snapshot` is exactly \"this position existed",
              "when the denominator was taken\". Storing the count rather than comparing",
              "`created_at` to `voting_starts_at` makes that exact rather than",
              "approximate: timestamps have one-second granularity, and a stake landing",
              "in the same second as activation would slip through a timestamp test.",
              "",
              "Without this, weight staked *after* activation adds to the numerator of",
              "the quorum test while the denominator stays fixed — see",
              "`a_position_opened_after_the_snapshot_cannot_vote`."
            ],
            "type": "u64"
          },
          {
            "name": "bump",
            "type": "u8"
          }
        ]
      }
    },
    {
      "name": "ProposalAction",
      "docs": [
        "What a passed proposal actually does.",
        "",
        "This is a closed enum rather than a blob of serialised instruction data.",
        "General-purpose governance (SPL Governance, OpenZeppelin Governor) lets a",
        "proposal carry arbitrary CPIs, which is more flexible and much harder to",
        "reason about: a voter has to decode raw instruction bytes to know what they",
        "are approving. Here the set of things governance *can* do is fixed at deploy",
        "time and visible in the IDL, so a voter reads the variant and knows the blast",
        "radius. Extending the set requires a program upgrade — which is itself",
        "governed. The trade-off is deliberate: less general, far more auditable."
      ],
      "type": {
        "kind": "enum",
        "variants": [
          {
            "name": "Signal"
          },
          {
            "name": "TreasuryTransfer",
            "fields": [
              {
                "name": "destination",
                "type": "pubkey"
              },
              {
                "name": "amount",
                "type": "u64"
              }
            ]
          },
          {
            "name": "SetStakingRewardRate",
            "fields": [
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
            "name": "CreateVestingStream",
            "fields": [
              {
                "name": "beneficiary",
                "type": "pubkey"
              },
              {
                "name": "total_amount",
                "type": "u64"
              },
              {
                "name": "start_ts",
                "type": "i64"
              },
              {
                "name": "cliff_ts",
                "type": "i64"
              },
              {
                "name": "end_ts",
                "type": "i64"
              }
            ]
          },
          {
            "name": "RevokeVestingStream",
            "fields": [
              {
                "name": "stream_id",
                "type": "u64"
              }
            ]
          },
          {
            "name": "SetTreasurySpendCap",
            "fields": [
              {
                "name": "new_cap",
                "type": "u64"
              },
              {
                "name": "epoch_duration",
                "type": "i64"
              }
            ]
          },
          {
            "name": "SetGovernanceExecutor",
            "fields": [
              {
                "name": "new_executor",
                "type": "pubkey"
              }
            ]
          },
          {
            "name": "AcceptTokenManagerAdmin"
          },
          {
            "name": "RegisterMinter",
            "fields": [
              {
                "name": "authority",
                "type": "pubkey"
              },
              {
                "name": "epoch_cap",
                "type": "u64"
              },
              {
                "name": "epoch_duration",
                "type": "i64"
              }
            ]
          },
          {
            "name": "UpdateMinter",
            "fields": [
              {
                "name": "epoch_cap",
                "type": "u64"
              },
              {
                "name": "enabled",
                "type": "bool"
              }
            ]
          },
          {
            "name": "RevokeMinter"
          },
          {
            "name": "SetTokenPaused",
            "fields": [
              {
                "name": "paused",
                "type": "bool"
              }
            ]
          },
          {
            "name": "ProposeTokenAdmin",
            "fields": [
              {
                "name": "new_admin",
                "type": "pubkey"
              }
            ]
          },
          {
            "name": "UpdateRealmParams",
            "fields": [
              {
                "name": "params",
                "type": {
                  "defined": {
                    "name": "RealmParams"
                  }
                }
              }
            ]
          },
          {
            "name": "SetRealmAuthority",
            "fields": [
              {
                "name": "new_authority",
                "type": "pubkey"
              }
            ]
          }
        ]
      }
    },
    {
      "name": "ProposalActivated",
      "type": {
        "kind": "struct",
        "fields": [
          {
            "name": "proposal",
            "type": "pubkey"
          },
          {
            "name": "voting_starts_at",
            "type": "i64"
          },
          {
            "name": "voting_ends_at",
            "type": "i64"
          },
          {
            "name": "total_weight_snapshot",
            "docs": [
              "The quorum denominator, fixed at this moment."
            ],
            "type": "u64"
          },
          {
            "name": "position_count_snapshot",
            "docs": [
              "How many positions that denominator covers. Carried so a consumer can",
              "tell whether a later vote belonged to the electorate without reading the",
              "proposal account back."
            ],
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
      "name": "ProposalCancelled",
      "type": {
        "kind": "struct",
        "fields": [
          {
            "name": "proposal",
            "type": "pubkey"
          },
          {
            "name": "guardian",
            "type": "pubkey"
          },
          {
            "name": "previous_state",
            "docs": [
              "State the proposal was vetoed from, so the record shows how far it got."
            ],
            "type": {
              "defined": {
                "name": "ProposalState"
              }
            }
          },
          {
            "name": "timestamp",
            "type": "i64"
          }
        ]
      }
    },
    {
      "name": "ProposalCreated",
      "type": {
        "kind": "struct",
        "fields": [
          {
            "name": "realm",
            "type": "pubkey"
          },
          {
            "name": "proposal",
            "type": "pubkey"
          },
          {
            "name": "proposer",
            "type": "pubkey"
          },
          {
            "name": "id",
            "type": "u64"
          },
          {
            "name": "action",
            "type": {
              "defined": {
                "name": "ProposalAction"
              }
            }
          },
          {
            "name": "title",
            "type": "string"
          },
          {
            "name": "timestamp",
            "type": "i64"
          }
        ]
      }
    },
    {
      "name": "ProposalExecuted",
      "type": {
        "kind": "struct",
        "fields": [
          {
            "name": "proposal",
            "type": "pubkey"
          },
          {
            "name": "action",
            "type": {
              "defined": {
                "name": "ProposalAction"
              }
            }
          },
          {
            "name": "timestamp",
            "type": "i64"
          }
        ]
      }
    },
    {
      "name": "ProposalFinalized",
      "type": {
        "kind": "struct",
        "fields": [
          {
            "name": "proposal",
            "type": "pubkey"
          },
          {
            "name": "outcome",
            "type": {
              "defined": {
                "name": "ProposalState"
              }
            }
          },
          {
            "name": "for_votes",
            "type": "u64"
          },
          {
            "name": "against_votes",
            "type": "u64"
          },
          {
            "name": "abstain_votes",
            "type": "u64"
          },
          {
            "name": "total_weight_snapshot",
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
      "name": "ProposalQueued",
      "type": {
        "kind": "struct",
        "fields": [
          {
            "name": "proposal",
            "type": "pubkey"
          },
          {
            "name": "eta",
            "docs": [
              "Earliest execution time."
            ],
            "type": "i64"
          },
          {
            "name": "expires_at",
            "docs": [
              "After this, the proposal expires unexecuted."
            ],
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
      "name": "ProposalState",
      "type": {
        "kind": "enum",
        "variants": [
          {
            "name": "Draft"
          },
          {
            "name": "Voting"
          },
          {
            "name": "Succeeded"
          },
          {
            "name": "Defeated"
          },
          {
            "name": "Queued"
          },
          {
            "name": "Executed"
          },
          {
            "name": "Cancelled"
          }
        ]
      }
    },
    {
      "name": "Realm",
      "docs": [
        "Governance configuration for one staking pool."
      ],
      "type": {
        "kind": "struct",
        "fields": [
          {
            "name": "authority",
            "docs": [
              "May change governance parameters. Intended end state is the realm's own",
              "executor PDA, so parameter changes go through the same vote as anything",
              "else."
            ],
            "type": "pubkey"
          },
          {
            "name": "guardian",
            "docs": [
              "May veto a proposal before execution, and may do nothing else. A guardian",
              "that could also *pass* proposals would not be a safety mechanism; it",
              "would be an admin key wearing a safety mechanism's name."
            ],
            "type": "pubkey"
          },
          {
            "name": "staking_pool",
            "docs": [
              "The staking pool whose positions confer vote weight. A position from any",
              "other pool is rejected."
            ],
            "type": "pubkey"
          },
          {
            "name": "quorum_bps",
            "docs": [
              "Fraction of snapshotted weight that must vote (any choice) for the",
              "result to count."
            ],
            "type": "u16"
          },
          {
            "name": "approval_bps",
            "docs": [
              "Fraction of decisive (for + against) weight that must be `For`."
            ],
            "type": "u16"
          },
          {
            "name": "voting_period",
            "type": "i64"
          },
          {
            "name": "timelock_delay",
            "type": "i64"
          },
          {
            "name": "min_weight_to_propose",
            "docs": [
              "Minimum weight a proposer must hold, to price out spam."
            ],
            "type": "u64"
          },
          {
            "name": "proposal_count",
            "docs": [
              "Monotonic counter seeding proposal PDAs."
            ],
            "type": "u64"
          },
          {
            "name": "bump",
            "type": "u8"
          },
          {
            "name": "executor_bump",
            "docs": [
              "Bump for the executor PDA, stored so only the canonical bump is used."
            ],
            "type": "u8"
          }
        ]
      }
    },
    {
      "name": "RealmAuthorityChanged",
      "type": {
        "kind": "struct",
        "fields": [
          {
            "name": "realm",
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
            "name": "self_governing",
            "docs": [
              "True once the realm's parameters answer only to the realm itself."
            ],
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
      "name": "RealmInitialized",
      "type": {
        "kind": "struct",
        "fields": [
          {
            "name": "realm",
            "type": "pubkey"
          },
          {
            "name": "authority",
            "type": "pubkey"
          },
          {
            "name": "guardian",
            "type": "pubkey"
          },
          {
            "name": "staking_pool",
            "type": "pubkey"
          },
          {
            "name": "quorum_bps",
            "type": "u16"
          },
          {
            "name": "approval_bps",
            "type": "u16"
          },
          {
            "name": "voting_period",
            "type": "i64"
          },
          {
            "name": "timelock_delay",
            "type": "i64"
          },
          {
            "name": "min_weight_to_propose",
            "docs": [
              "Carried for the same reason every other parameter is: a consumer that",
              "reconstructs the realm from events must arrive at the account, and this",
              "field is otherwise unlearnable until the first `RealmParamsUpdated`. An",
              "event that omits one field of the state it announces makes the whole",
              "reconstruction conditional on an update that may never happen."
            ],
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
      "name": "RealmParams",
      "docs": [
        "Parameters shared by realm creation and parameter updates.",
        "Derives `InitSpace` because `ProposalAction::UpdateRealmParams` carries one,",
        "and a proposal's action is stored in the account."
      ],
      "type": {
        "kind": "struct",
        "fields": [
          {
            "name": "quorum_bps",
            "type": "u16"
          },
          {
            "name": "approval_bps",
            "type": "u16"
          },
          {
            "name": "voting_period",
            "type": "i64"
          },
          {
            "name": "timelock_delay",
            "type": "i64"
          },
          {
            "name": "min_weight_to_propose",
            "type": "u64"
          }
        ]
      }
    },
    {
      "name": "RealmParamsUpdated",
      "type": {
        "kind": "struct",
        "fields": [
          {
            "name": "realm",
            "type": "pubkey"
          },
          {
            "name": "by_proposal",
            "docs": [
              "Whether the change came through a proposal or from the realm authority",
              "signing directly. Worth recording: before the authority is migrated these",
              "are two very different events with the same effect."
            ],
            "type": "bool"
          },
          {
            "name": "quorum_bps",
            "type": "u16"
          },
          {
            "name": "approval_bps",
            "type": "u16"
          },
          {
            "name": "voting_period",
            "type": "i64"
          },
          {
            "name": "timelock_delay",
            "type": "i64"
          },
          {
            "name": "min_weight_to_propose",
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
      "name": "TokenConfig",
      "docs": [
        "Singleton configuration for the HLX mint. PDA: `[\"config\", mint]`."
      ],
      "type": {
        "kind": "struct",
        "fields": [
          {
            "name": "admin",
            "docs": [
              "May register minters and pause deposits. Cannot mint, and cannot move",
              "funds anywhere."
            ],
            "type": "pubkey"
          },
          {
            "name": "pending_admin",
            "docs": [
              "Set by `propose_admin`, cleared by `accept_admin`. Admin transfer is",
              "deliberately two-step: a one-step transfer to a mistyped address is",
              "unrecoverable."
            ],
            "type": {
              "option": "pubkey"
            }
          },
          {
            "name": "mint",
            "docs": [
              "The HLX mint this config governs."
            ],
            "type": "pubkey"
          },
          {
            "name": "mint_authority_bump",
            "docs": [
              "Bump for `[\"mint_authority\", config]`, stored so every later derivation",
              "uses the canonical bump rather than searching for one."
            ],
            "type": "u8"
          },
          {
            "name": "bump",
            "docs": [
              "Bump for this account's own PDA."
            ],
            "type": "u8"
          },
          {
            "name": "paused",
            "docs": [
              "When true, `mint_to` is rejected. Burning stays available — a pause that",
              "blocks the exit path is indistinguishable from a freeze."
            ],
            "type": "bool"
          },
          {
            "name": "total_minted",
            "docs": [
              "Lifetime issuance and redemption, for analytics and for reconciling the",
              "indexer against chain state."
            ],
            "type": "u64"
          },
          {
            "name": "total_burned",
            "type": "u64"
          },
          {
            "name": "minter_count",
            "docs": [
              "Number of registered minters, bounded by [`crate::constants::MAX_MINTERS`]."
            ],
            "type": "u16"
          }
        ]
      }
    },
    {
      "name": "Treasury",
      "docs": [
        "A DAO-owned vault.",
        "",
        "The vault authority is a PDA, so no key can move funds. Spending requires a",
        "signature from [`Self::governance_executor`] — a PDA that only the governance",
        "program can produce, and only inside the execution of a proposal that passed",
        "quorum and cleared its timelock. That chain is the entire security model; see",
        "`docs/THREAT-MODEL.md`."
      ],
      "type": {
        "kind": "struct",
        "fields": [
          {
            "name": "governance_executor",
            "docs": [
              "The only signer permitted to spend. Set at initialisation and changeable",
              "only by the current executor — so migrating governance is itself an act",
              "of governance."
            ],
            "type": "pubkey"
          },
          {
            "name": "mint",
            "type": "pubkey"
          },
          {
            "name": "vault",
            "type": "pubkey"
          },
          {
            "name": "total_deposited",
            "type": "u64"
          },
          {
            "name": "total_spent",
            "type": "u64"
          },
          {
            "name": "committed_to_streams",
            "docs": [
              "Sum of the unclaimed remainder of every live vesting stream.",
              "",
              "Tracked so that `spend` cannot pay out tokens already promised to a",
              "beneficiary. Without this, a passed proposal could drain the vault and",
              "leave existing streams unfunded — the stream holder would discover it",
              "only when their claim failed (`INVARIANTS.md` §1.6)."
            ],
            "type": "u64"
          },
          {
            "name": "epoch_duration",
            "docs": [
              "Defence in depth against a malicious-but-passed proposal: even with a",
              "genuine majority, the treasury cannot be emptied in a single",
              "transaction. It buys time for the guardian veto and for holders to exit."
            ],
            "type": "i64"
          },
          {
            "name": "epoch_spend_cap",
            "type": "u64"
          },
          {
            "name": "spent_this_epoch",
            "type": "u64"
          },
          {
            "name": "current_epoch",
            "type": "u64"
          },
          {
            "name": "stream_count",
            "docs": [
              "Monotonic counter seeding stream PDAs."
            ],
            "type": "u64"
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
      "name": "VestingStream",
      "docs": [
        "A linear vesting stream with an optional cliff.",
        "",
        "Created only by governance, claimable only by the beneficiary, revocable only",
        "by governance — and a revoke never claws back what has already vested."
      ],
      "type": {
        "kind": "struct",
        "fields": [
          {
            "name": "treasury",
            "type": "pubkey"
          },
          {
            "name": "beneficiary",
            "type": "pubkey"
          },
          {
            "name": "stream_id",
            "type": "u64"
          },
          {
            "name": "total_amount",
            "type": "u64"
          },
          {
            "name": "claimed",
            "type": "u64"
          },
          {
            "name": "start_ts",
            "type": "i64"
          },
          {
            "name": "cliff_ts",
            "docs": [
              "Nothing is claimable before this. Vesting still *accrues* from",
              "`start_ts`, so the cliff releases everything accrued up to that point at",
              "once — the standard \"1 year cliff on a 4 year schedule\" shape."
            ],
            "type": "i64"
          },
          {
            "name": "end_ts",
            "type": "i64"
          },
          {
            "name": "revoked",
            "type": "bool"
          },
          {
            "name": "revoked_at",
            "docs": [
              "Timestamp of revocation. Vesting is evaluated as of this moment",
              "afterwards, which is what makes a revoke forward-only."
            ],
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
      "name": "VoteCast",
      "type": {
        "kind": "struct",
        "fields": [
          {
            "name": "proposal",
            "type": "pubkey"
          },
          {
            "name": "position",
            "type": "pubkey"
          },
          {
            "name": "voter",
            "type": "pubkey"
          },
          {
            "name": "choice",
            "type": {
              "defined": {
                "name": "VoteChoice"
              }
            }
          },
          {
            "name": "weight",
            "type": "u64"
          },
          {
            "name": "for_votes",
            "type": "u64"
          },
          {
            "name": "against_votes",
            "type": "u64"
          },
          {
            "name": "abstain_votes",
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
      "name": "VoteChoice",
      "type": {
        "kind": "enum",
        "variants": [
          {
            "name": "For"
          },
          {
            "name": "Against"
          },
          {
            "name": "Abstain"
          }
        ]
      }
    },
    {
      "name": "VoteRecord",
      "docs": [
        "One position's vote on one proposal. PDA: `[\"vote\", proposal, position]`.",
        "",
        "Seeding by *position* rather than by wallet is what lets a holder with three",
        "positions vote their full weight while keeping each position's vote exactly",
        "once-only. Double voting is an `init` constraint failure, not a runtime check."
      ],
      "type": {
        "kind": "struct",
        "fields": [
          {
            "name": "proposal",
            "type": "pubkey"
          },
          {
            "name": "position",
            "type": "pubkey"
          },
          {
            "name": "voter",
            "type": "pubkey"
          },
          {
            "name": "choice",
            "type": {
              "defined": {
                "name": "VoteChoice"
              }
            }
          },
          {
            "name": "weight",
            "docs": [
              "Weight counted, retained so the tally can be audited after the fact."
            ],
            "type": "u64"
          },
          {
            "name": "voted_at",
            "type": "i64"
          },
          {
            "name": "bump",
            "type": "u8"
          }
        ]
      }
    }
  ]
};

export default idl;
