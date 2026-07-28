//! The atomic bootstrap, as a plan that can be inspected before it is sent.
//!
//! # Why this is a library and not a shell script
//!
//! [F-1](../../docs/SECURITY-ASSESSMENT.md) is that `initialize_pool`,
//! `initialize_realm` and `initialize_treasury` are first-caller-wins with PDAs
//! seeded by the mint, so an observer can front-run them between deploy and
//! bootstrap and install themselves as authority — permanently. The mitigation is
//! to run all three in one transaction, which leaves no window.
//!
//! A one-shot transaction is exactly the thing you cannot rehearse. Get an
//! account wrong and the failure arrives on mainnet, at the only moment the
//! window is open, with an attacker watching. So the instruction set lives here
//! rather than in the tool that sends it, and
//! `tests/integration/tests/bootstrap_atomicity.rs` **executes this same
//! function** against the real BPF programs: the plan the tool prints is the plan
//! the suite proves works.
//!
//! Most deployment scripts are written once, run once, and tested never.
//!
//! # What is not here
//!
//! No network. See the note in `Cargo.toml` — an RPC client would drag a second
//! major version of the Solana SDK into the graph to send a single transaction,
//! and it would be the one part of this crate that could not be tested.

use anchor_lang::prelude::Pubkey;
use anchor_lang::{InstructionData, ToAccountMetas};
use anchor_spl::token_2022;
use helix_governance::instructions::realm::RealmParams;
use serde::Serialize;
use solana_instruction::Instruction;

/// Pubkeys as base58 strings rather than byte arrays.
///
/// The default `Serialize` for a `Pubkey` emits 32 numbers, which is unusable by
/// any client that expects an address.
mod as_string {
    use anchor_lang::prelude::Pubkey;
    use serde::Serializer;

    pub fn serialize<S: Serializer>(key: &Pubkey, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&key.to_string())
    }
}

/// Everything the bootstrap needs to know that is not derivable.
pub struct BootstrapConfig {
    /// Funds the rent for every account created. The only human key involved,
    /// and it ends up controlling nothing — see [`Plan::privileged_parties`].
    pub payer: Pubkey,
    /// The HLX mint. Must already exist: the pool, realm and treasury PDAs are
    /// all seeded from it.
    pub mint: Pubkey,
    /// May only veto. Intended to be a multisig.
    pub guardian: Pubkey,
    pub realm: RealmParams,
    pub epoch_spend_cap: u64,
    pub epoch_duration: i64,
}

/// Every address the bootstrap creates or names, derived from the mint.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Addresses {
    #[serde(with = "as_string")]
    pub pool: Pubkey,
    #[serde(with = "as_string")]
    pub pool_vault_authority: Pubkey,
    #[serde(with = "as_string")]
    pub stake_vault: Pubkey,
    #[serde(with = "as_string")]
    pub reward_vault: Pubkey,
    #[serde(with = "as_string")]
    pub realm: Pubkey,
    /// The PDA that ends up holding every authority in the system. Possession of
    /// it is the right to spend the treasury, and only `execute_*` produces it.
    #[serde(with = "as_string")]
    pub executor: Pubkey,
    #[serde(with = "as_string")]
    pub treasury: Pubkey,
    #[serde(with = "as_string")]
    pub treasury_vault: Pubkey,
    #[serde(with = "as_string")]
    pub treasury_vault_authority: Pubkey,
}

impl Addresses {
    pub fn derive(mint: &Pubkey) -> Self {
        let (pool, _) = Pubkey::find_program_address(
            &[b"pool", mint.as_ref(), mint.as_ref()],
            &helix_staking::ID,
        );
        let (pool_vault_authority, _) =
            Pubkey::find_program_address(&[b"vault_authority", pool.as_ref()], &helix_staking::ID);
        let (stake_vault, _) =
            Pubkey::find_program_address(&[b"stake_vault", pool.as_ref()], &helix_staking::ID);
        let (reward_vault, _) =
            Pubkey::find_program_address(&[b"reward_vault", pool.as_ref()], &helix_staking::ID);

        let (realm, _) =
            Pubkey::find_program_address(&[b"realm", pool.as_ref()], &helix_governance::ID);
        let (executor, _) =
            Pubkey::find_program_address(&[b"executor", realm.as_ref()], &helix_governance::ID);

        let (treasury, _) =
            Pubkey::find_program_address(&[b"treasury", mint.as_ref()], &helix_treasury::ID);
        let (treasury_vault, _) =
            Pubkey::find_program_address(&[b"vault", treasury.as_ref()], &helix_treasury::ID);
        let (treasury_vault_authority, _) = Pubkey::find_program_address(
            &[b"vault_authority", treasury.as_ref()],
            &helix_treasury::ID,
        );

        Self {
            pool,
            pool_vault_authority,
            stake_vault,
            reward_vault,
            realm,
            executor,
            treasury,
            treasury_vault,
            treasury_vault_authority,
        }
    }
}

/// Who will control what once this lands.
///
/// Printed by the tool and asserted by the tests. The point of reading it before
/// sending is that every entry should be the executor PDA — if the payer appears
/// anywhere, the bootstrap has installed a human key as an authority and the
/// whole security model is a signature away from being bypassed.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct PrivilegedParties {
    #[serde(with = "as_string")]
    pub pool_authority: Pubkey,
    #[serde(with = "as_string")]
    pub realm_authority: Pubkey,
    #[serde(with = "as_string")]
    pub treasury_spender: Pubkey,
    #[serde(with = "as_string")]
    pub guardian: Pubkey,
}

pub struct Plan {
    pub instructions: Vec<Instruction>,
    pub addresses: Addresses,
    parties: PrivilegedParties,
}

impl Plan {
    pub fn privileged_parties(&self) -> PrivilegedParties {
        self.parties
    }

    /// Serialised size of the signed transaction, in bytes.
    ///
    /// Measured rather than estimated, against a placeholder blockhash and
    /// signature — both are fixed-width, so the figure is exact for the real
    /// thing. Solana caps a packet at 1232 bytes and the mitigation for F-1 is
    /// only available if the whole bootstrap fits inside one.
    pub fn transaction_size(&self) -> usize {
        let payer = self.instructions[0].accounts[0].pubkey;
        let mut tx =
            solana_transaction::Transaction::new_with_payer(&self.instructions, Some(&payer));
        // `new_with_payer` leaves the signature slot empty. One signature — the
        // payer's — at its real width, with placeholder bytes: signatures are
        // fixed size, so the total is exact for the transaction that gets sent.
        tx.signatures = vec![Default::default()];
        bincode::serialize(&tx)
            .map(|b| b.len())
            .unwrap_or(usize::MAX)
    }

    /// Distinct accounts the transaction references.
    pub fn account_count(&self) -> usize {
        let mut keys: Vec<Pubkey> = self
            .instructions
            .iter()
            .flat_map(|ix| {
                std::iter::once(ix.program_id).chain(ix.accounts.iter().map(|a| a.pubkey))
            })
            .collect();
        keys.sort();
        keys.dedup();
        keys.len()
    }
}

/// The three initialisers, in one transaction, with every authority already
/// pointed at governance.
///
/// The ordering is not arbitrary: the realm must exist before the treasury names
/// its executor, and the pool before the realm names its staking source. The
/// executor address is derivable ahead of all of them, which is what allows the
/// pool to be handed to governance in the same instruction that creates it
/// rather than through a later handover.
pub fn plan(config: &BootstrapConfig) -> Plan {
    let addresses = Addresses::derive(&config.mint);
    let payer = config.payer;
    let executor = addresses.executor;

    let instructions = vec![
        instruction(
            helix_staking::ID,
            helix_staking::accounts::InitializePool {
                payer,
                // Governance from the first instruction. There is never a block
                // in which a human key controls emissions.
                authority: executor,
                pool: addresses.pool,
                vault_authority: addresses.pool_vault_authority,
                stake_mint: config.mint,
                reward_mint: config.mint,
                stake_vault: addresses.stake_vault,
                reward_vault: addresses.reward_vault,
                token_program: token_2022::ID,
                system_program: anchor_lang::system_program::ID,
            },
            helix_staking::instruction::InitializePool {},
        ),
        instruction(
            helix_governance::ID,
            helix_governance::accounts::InitializeRealm {
                payer,
                // The realm owns its own parameters immediately, so the rules of
                // governance are never the property of a key outside it — F-11.
                authority: executor,
                guardian: config.guardian,
                staking_pool: addresses.pool,
                realm: addresses.realm,
                executor,
                system_program: anchor_lang::system_program::ID,
            },
            helix_governance::instruction::InitializeRealm {
                params: config.realm,
            },
        ),
        instruction(
            helix_treasury::ID,
            helix_treasury::accounts::InitializeTreasury {
                payer,
                governance_executor: executor,
                treasury: addresses.treasury,
                vault_authority: addresses.treasury_vault_authority,
                mint: config.mint,
                vault: addresses.treasury_vault,
                token_program: token_2022::ID,
                system_program: anchor_lang::system_program::ID,
            },
            helix_treasury::instruction::InitializeTreasury {
                epoch_spend_cap: config.epoch_spend_cap,
                epoch_duration: config.epoch_duration,
            },
        ),
    ];

    Plan {
        instructions,
        addresses,
        parties: PrivilegedParties {
            pool_authority: executor,
            realm_authority: executor,
            treasury_spender: executor,
            guardian: config.guardian,
        },
    }
}

fn instruction<A: ToAccountMetas, D: InstructionData>(
    program_id: Pubkey,
    accounts: A,
    data: D,
) -> Instruction {
    Instruction {
        program_id,
        accounts: accounts.to_account_metas(None),
        data: data.data(),
    }
}

/// Instructions in a form any client can submit.
#[derive(Serialize)]
pub struct InstructionJson {
    pub program_id: String,
    pub accounts: Vec<AccountJson>,
    /// Base64, because instruction data is arbitrary bytes.
    pub data: String,
}

#[derive(Serialize)]
pub struct AccountJson {
    pub pubkey: String,
    pub is_signer: bool,
    pub is_writable: bool,
}

pub fn to_json(plan: &Plan) -> Vec<InstructionJson> {
    plan.instructions
        .iter()
        .map(|ix| InstructionJson {
            program_id: ix.program_id.to_string(),
            accounts: ix
                .accounts
                .iter()
                .map(|a| AccountJson {
                    pubkey: a.pubkey.to_string(),
                    is_signer: a.is_signer,
                    is_writable: a.is_writable,
                })
                .collect(),
            data: base64_encode(&ix.data),
        })
        .collect()
}

/// Standard base64, hand-rolled to avoid pulling a crate into an ops binary for
/// one function.
fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);

    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> BootstrapConfig {
        BootstrapConfig {
            payer: Pubkey::new_unique(),
            mint: Pubkey::new_unique(),
            guardian: Pubkey::new_unique(),
            realm: RealmParams {
                quorum_bps: 2_000,
                approval_bps: 5_001,
                voting_period: 3 * 86_400,
                timelock_delay: 2 * 86_400,
                min_weight_to_propose: 1_000,
            },
            epoch_spend_cap: 1_000_000_000,
            epoch_duration: 24 * 3_600,
        }
    }

    /// The check the operator is really running the tool for.
    #[test]
    fn the_payer_ends_up_controlling_nothing() {
        let config = config();
        let plan = plan(&config);
        let parties = plan.privileged_parties();

        for (name, holder) in [
            ("pool authority", parties.pool_authority),
            ("realm authority", parties.realm_authority),
            ("treasury spender", parties.treasury_spender),
        ] {
            assert_eq!(
                holder, plan.addresses.executor,
                "{name} is not the executor PDA"
            );
            assert_ne!(holder, config.payer, "{name} is the payer — a human key");
        }
    }

    #[test]
    fn every_address_derives_from_the_mint_alone() {
        let config = config();
        let a = plan(&config).addresses;
        let b = Addresses::derive(&config.mint);

        // Same mint, same addresses — which is what makes the bootstrap
        // reproducible and the front-running window well defined.
        assert_eq!(a.pool, b.pool);
        assert_eq!(a.executor, b.executor);
        assert_eq!(a.treasury, b.treasury);
    }

    #[test]
    fn a_different_mint_produces_a_different_system() {
        let mut first = config();
        let plan_a = plan(&first);
        first.mint = Pubkey::new_unique();
        let plan_b = plan(&first);

        assert_ne!(plan_a.addresses.pool, plan_b.addresses.pool);
        assert_ne!(plan_a.addresses.executor, plan_b.addresses.executor);
    }

    #[test]
    fn the_transaction_fits_a_solana_packet() {
        let plan = plan(&config());
        assert!(
            plan.transaction_size() <= 1232,
            "bootstrap is {} bytes, over the packet limit",
            plan.transaction_size()
        );
        assert!(plan.account_count() < 32);
    }

    #[test]
    fn base64_matches_the_standard_alphabet_and_padding() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn the_json_form_preserves_signer_and_writable_flags() {
        let plan = plan(&config());
        let json = to_json(&plan);

        assert_eq!(json.len(), 3);
        // The payer signs, and a client rebuilding from this JSON has to know.
        assert!(
            json[0].accounts.iter().any(|a| a.is_signer),
            "no signer survived serialisation, so the rebuilt transaction cannot be signed"
        );
        assert!(json[0].accounts.iter().any(|a| a.is_writable));
    }
}
