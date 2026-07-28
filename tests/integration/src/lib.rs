//! Shared fixtures for the runtime test suite.
//!
//! The point of this harness is to build the *wired* system — mint, pool, realm,
//! treasury, with authorities actually handed over — because the wiring is exactly
//! what the unit tests cannot reach. A fixture that stops short of the handover
//! only exercises the parts that already work.
//!
//! See `docs/TESTING.md` for what these cover and what they still do not.

use anchor_lang::prelude::*;
use anchor_lang::{InstructionData, ToAccountMetas};
use anchor_spl::token_2022::spl_token_2022;
use litesvm::types::TransactionMetadata;
use litesvm::LiteSVM;
use solana_instruction::Instruction;
use solana_keypair::Keypair;
use solana_native_token::LAMPORTS_PER_SOL;
use solana_signer::Signer as _;
use solana_transaction::Transaction;

use spl_token_2022::extension::ExtensionType;
use spl_token_2022::state::{Account as T22Account, Mint as T22Mint};

pub mod bootstrap;
pub use bootstrap::{Staker, System};

/// Fee configuration for a Token-2022 mint.
///
/// The whole reason this harness exists: staking must be exercised against a mint
/// where the amount sent is *not* the amount received.
#[derive(Clone, Copy, Debug)]
pub struct TransferFee {
    pub basis_points: u16,
    pub maximum_fee: u64,
}

impl TransferFee {
    /// The fee Token-2022 will withhold on a transfer of `amount`.
    ///
    /// Mirrors the token program's own calculation — ceiling division, capped at
    /// `maximum_fee` — so tests can assert exact credited amounts rather than
    /// merely "less than".
    pub fn expected_on(&self, amount: u64) -> u64 {
        if self.basis_points == 0 {
            return 0;
        }
        let fee = (amount as u128)
            .saturating_mul(self.basis_points as u128)
            .div_ceil(10_000) as u64;
        fee.min(self.maximum_fee)
    }
}

pub struct TestEnv {
    pub svm: LiteSVM,
    pub payer: Keypair,
}

impl TestEnv {
    /// Boots a VM with all four programs loaded and a funded payer.
    pub fn new() -> Self {
        let mut svm = LiteSVM::new();
        let payer = Keypair::new();

        svm.airdrop(&payer.pubkey(), 1_000 * LAMPORTS_PER_SOL)
            .expect("airdrop failed");

        for (lib_name, id) in [
            ("helix_token_manager", helix_token_manager::ID),
            ("helix_staking", helix_staking::ID),
            ("helix_governance", helix_governance::ID),
            ("helix_treasury", helix_treasury::ID),
        ] {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/deploy")
                .join(format!("{lib_name}.so"));
            let bytes = std::fs::read(&path).unwrap_or_else(|e| {
                panic!(
                    "cannot read {} ({e}) — run `anchor build` first",
                    path.display()
                )
            });
            svm.add_program(id, &bytes)
                .unwrap_or_else(|e| panic!("{lib_name} failed to load: {e:?}"));
        }

        Self { svm, payer }
    }

    pub fn payer_pubkey(&self) -> Pubkey {
        self.payer.pubkey()
    }

    /// Signs and sends `instructions`, panicking with the program logs on failure.
    ///
    /// Panicking is right for the happy path: a fixture step that fails is a broken
    /// test, and the logs are what makes it diagnosable. Use [`Self::try_send`]
    /// where the failure *is* the assertion.
    pub fn send(&mut self, instructions: &[Instruction], extra_signers: &[&Keypair]) {
        if let Err(e) = self.try_send(instructions, extra_signers) {
            panic!("transaction failed: {e}");
        }
    }

    /// Sends and returns the failure as a string so tests can assert on it.
    pub fn try_send(
        &mut self,
        instructions: &[Instruction],
        extra_signers: &[&Keypair],
    ) -> std::result::Result<(), String> {
        self.dispatch(instructions, extra_signers).map(|_| ())
    }

    /// Sends and returns the compute units the transaction consumed.
    ///
    /// The runtime reports one accumulated figure per transaction, so this is only
    /// attributable when the transaction carries a single instruction. Callers in
    /// `compute_budget.rs` keep to that; nothing else needs this at all.
    pub fn compute_units(
        &mut self,
        instructions: &[Instruction],
        extra_signers: &[&Keypair],
    ) -> u64 {
        self.dispatch(instructions, extra_signers)
            .unwrap_or_else(|e| panic!("transaction failed: {e}"))
            .compute_units_consumed
    }

    fn dispatch(
        &mut self,
        instructions: &[Instruction],
        extra_signers: &[&Keypair],
    ) -> std::result::Result<TransactionMetadata, String> {
        let mut signers: Vec<&Keypair> = vec![&self.payer];
        signers.extend_from_slice(extra_signers);

        // Force a fresh blockhash for every transaction.
        //
        // Two identical instruction sets signed under the same blockhash produce
        // the same transaction signature, and the runtime rejects the second as
        // `AlreadyProcessed` — which surfaces as a confusing failure in tests that
        // legitimately repeat a call (claim twice, unstake in a loop). A real
        // cluster gets new blockhashes as slots advance; LiteSVM has to be told.
        self.svm.expire_blockhash();

        let tx = Transaction::new_signed_with_payer(
            instructions,
            Some(&self.payer.pubkey()),
            &signers,
            self.svm.latest_blockhash(),
        );

        self.svm
            .send_transaction(tx)
            // Include the logs: an Anchor error code alone ("custom program error:
            // 0x1771") tells you nothing, and the log line names the constraint.
            .map_err(|e| format!("{:?}\nlogs:\n  {}", e.err, e.meta.logs.join("\n  ")))
    }

    /// Deserialises an Anchor account, skipping the 8-byte discriminator.
    pub fn anchor_account<T: AccountDeserialize>(&self, address: &Pubkey) -> T {
        let raw = self
            .svm
            .get_account(address)
            .unwrap_or_else(|| panic!("account {address} does not exist"));
        T::try_deserialize(&mut raw.data.as_slice())
            .unwrap_or_else(|e| panic!("failed to deserialise {address}: {e:?}"))
    }

    /// Balance of an SPL/Token-2022 token account.
    pub fn token_balance(&self, token_account: &Pubkey) -> u64 {
        use spl_token_2022::extension::StateWithExtensions;
        let raw = self
            .svm
            .get_account(token_account)
            .unwrap_or_else(|| panic!("token account {token_account} does not exist"));
        StateWithExtensions::<T22Account>::unpack(&raw.data)
            .expect("not a token account")
            .base
            .amount
    }

    /// Advances the cluster clock by `seconds`.
    ///
    /// Time-dependent behaviour is the bulk of what needs testing here — lock
    /// expiry, voting windows, timelocks, vesting — and none of it is reachable
    /// without this.
    pub fn warp_forward(&mut self, seconds: i64) {
        let mut clock = self.svm.get_sysvar::<Clock>();
        clock.unix_timestamp += seconds;
        // Keep the slot moving too, so blockhashes stay distinct.
        clock.slot += (seconds.max(1) as u64) / 2 + 1;
        self.svm.set_sysvar(&clock);
        self.svm.expire_blockhash();
    }

    pub fn now(&self) -> i64 {
        self.svm.get_sysvar::<Clock>().unix_timestamp
    }

    // ------------------------------------------------------------------ tokens

    /// Creates a Token-2022 mint, optionally carrying the transfer-fee extension.
    ///
    /// Extension state must be initialised *before* `initialize_mint2`, and the
    /// account must be sized for the extensions up front — Token-2022 will not
    /// grow it later.
    pub fn create_mint(
        &mut self,
        decimals: u8,
        mint_authority: &Pubkey,
        fee: Option<TransferFee>,
    ) -> Keypair {
        self.create_mint_with_keypair(Keypair::new(), decimals, mint_authority, fee)
    }

    /// The same, with the mint key supplied rather than generated.
    pub fn create_mint_with_keypair(
        &mut self,
        mint: Keypair,
        decimals: u8,
        mint_authority: &Pubkey,
        fee: Option<TransferFee>,
    ) -> Keypair {
        let extensions: Vec<ExtensionType> = if fee.is_some() {
            vec![ExtensionType::TransferFeeConfig]
        } else {
            vec![]
        };
        let space = ExtensionType::try_calculate_account_len::<T22Mint>(&extensions)
            .expect("cannot size mint");
        let lamports = self.svm.minimum_balance_for_rent_exemption(space);

        let mut ixs = vec![solana_system_interface::instruction::create_account(
            &self.payer.pubkey(),
            &mint.pubkey(),
            lamports,
            space as u64,
            &spl_token_2022::ID,
        )];

        if let Some(fee) = fee {
            ixs.push(
                spl_token_2022::extension::transfer_fee::instruction::initialize_transfer_fee_config(
                    &spl_token_2022::ID,
                    &mint.pubkey(),
                    Some(mint_authority),
                    Some(mint_authority),
                    fee.basis_points,
                    fee.maximum_fee,
                )
                .expect("bad transfer fee config"),
            );
        }

        ixs.push(
            spl_token_2022::instruction::initialize_mint2(
                &spl_token_2022::ID,
                &mint.pubkey(),
                mint_authority,
                None,
                decimals,
            )
            .expect("bad initialize_mint2"),
        );

        self.try_send(&ixs, &[&mint])
            .unwrap_or_else(|e| panic!("create_mint failed: {e}"));

        mint
    }

    /// Creates and initialises a token account for `owner`.
    pub fn create_token_account(&mut self, mint: &Pubkey, owner: &Pubkey) -> Keypair {
        let account = Keypair::new();

        // A token account for a fee-bearing mint needs room for the withheld
        // amount the extension tracks per account.
        let mint_raw = self.svm.get_account(mint).expect("mint missing");
        let has_fee = {
            use spl_token_2022::extension::{BaseStateWithExtensions, StateWithExtensions};
            StateWithExtensions::<T22Mint>::unpack(&mint_raw.data)
                .map(|m| {
                    m.get_extension_types()
                        .map(|e| e.contains(&ExtensionType::TransferFeeConfig))
                        .unwrap_or(false)
                })
                .unwrap_or(false)
        };

        let extensions: Vec<ExtensionType> = if has_fee {
            vec![ExtensionType::TransferFeeAmount]
        } else {
            vec![]
        };
        let space = ExtensionType::try_calculate_account_len::<T22Account>(&extensions)
            .expect("cannot size token account");
        let lamports = self.svm.minimum_balance_for_rent_exemption(space);

        let ixs = vec![
            solana_system_interface::instruction::create_account(
                &self.payer.pubkey(),
                &account.pubkey(),
                lamports,
                space as u64,
                &spl_token_2022::ID,
            ),
            spl_token_2022::instruction::initialize_account3(
                &spl_token_2022::ID,
                &account.pubkey(),
                mint,
                owner,
            )
            .expect("bad initialize_account3"),
        ];

        self.try_send(&ixs, &[&account])
            .unwrap_or_else(|e| panic!("create_token_account failed: {e}"));

        account
    }

    /// Mints `amount` directly, using the raw Token-2022 mint authority.
    ///
    /// Used to fund test actors. The protocol's own gated issuance path is
    /// `helix_token_manager::mint_tokens`, tested separately.
    pub fn mint_tokens_raw(
        &mut self,
        mint: &Pubkey,
        destination: &Pubkey,
        authority: &Keypair,
        amount: u64,
    ) {
        let ix = spl_token_2022::instruction::mint_to(
            &spl_token_2022::ID,
            mint,
            destination,
            &authority.pubkey(),
            &[],
            amount,
        )
        .expect("bad mint_to");

        self.try_send(&[ix], &[authority])
            .unwrap_or_else(|e| panic!("mint_tokens_raw failed: {e}"));
    }

    // -------------------------------------------------------------- ix builder

    /// Builds an Anchor instruction from its accounts and args structs.
    pub fn ix<A: ToAccountMetas, D: InstructionData>(
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
}

impl Default for TestEnv {
    fn default() -> Self {
        Self::new()
    }
}

/// PDA derivations, mirroring each program's seeds.
pub mod pda {
    use anchor_lang::prelude::Pubkey;

    pub fn token_config(mint: &Pubkey) -> (Pubkey, u8) {
        Pubkey::find_program_address(&[b"config", mint.as_ref()], &helix_token_manager::ID)
    }

    pub fn mint_authority(config: &Pubkey) -> (Pubkey, u8) {
        Pubkey::find_program_address(
            &[b"mint_authority", config.as_ref()],
            &helix_token_manager::ID,
        )
    }

    pub fn minter(config: &Pubkey, authority: &Pubkey) -> (Pubkey, u8) {
        Pubkey::find_program_address(
            &[b"minter", config.as_ref(), authority.as_ref()],
            &helix_token_manager::ID,
        )
    }

    pub fn pool(stake_mint: &Pubkey, reward_mint: &Pubkey) -> (Pubkey, u8) {
        Pubkey::find_program_address(
            &[b"pool", stake_mint.as_ref(), reward_mint.as_ref()],
            &helix_staking::ID,
        )
    }

    pub fn vault_authority(pool: &Pubkey) -> (Pubkey, u8) {
        Pubkey::find_program_address(&[b"vault_authority", pool.as_ref()], &helix_staking::ID)
    }

    pub fn stake_vault(pool: &Pubkey) -> (Pubkey, u8) {
        Pubkey::find_program_address(&[b"stake_vault", pool.as_ref()], &helix_staking::ID)
    }

    pub fn reward_vault(pool: &Pubkey) -> (Pubkey, u8) {
        Pubkey::find_program_address(&[b"reward_vault", pool.as_ref()], &helix_staking::ID)
    }

    pub fn position(pool: &Pubkey, owner: &Pubkey, position_id: u64) -> (Pubkey, u8) {
        Pubkey::find_program_address(
            &[
                b"position",
                pool.as_ref(),
                owner.as_ref(),
                &position_id.to_le_bytes(),
            ],
            &helix_staking::ID,
        )
    }

    pub fn realm(staking_pool: &Pubkey) -> (Pubkey, u8) {
        Pubkey::find_program_address(&[b"realm", staking_pool.as_ref()], &helix_governance::ID)
    }

    pub fn executor(realm: &Pubkey) -> (Pubkey, u8) {
        Pubkey::find_program_address(&[b"executor", realm.as_ref()], &helix_governance::ID)
    }

    pub fn proposal(realm: &Pubkey, id: u64) -> (Pubkey, u8) {
        Pubkey::find_program_address(
            &[b"proposal", realm.as_ref(), &id.to_le_bytes()],
            &helix_governance::ID,
        )
    }

    pub fn vote_record(proposal: &Pubkey, position: &Pubkey) -> (Pubkey, u8) {
        Pubkey::find_program_address(
            &[b"vote", proposal.as_ref(), position.as_ref()],
            &helix_governance::ID,
        )
    }

    pub fn treasury(mint: &Pubkey) -> (Pubkey, u8) {
        Pubkey::find_program_address(&[b"treasury", mint.as_ref()], &helix_treasury::ID)
    }

    pub fn treasury_vault(treasury: &Pubkey) -> (Pubkey, u8) {
        Pubkey::find_program_address(&[b"vault", treasury.as_ref()], &helix_treasury::ID)
    }

    pub fn treasury_vault_authority(treasury: &Pubkey) -> (Pubkey, u8) {
        Pubkey::find_program_address(
            &[b"vault_authority", treasury.as_ref()],
            &helix_treasury::ID,
        )
    }

    pub fn stream(treasury: &Pubkey, stream_id: u64) -> (Pubkey, u8) {
        Pubkey::find_program_address(
            &[b"stream", treasury.as_ref(), &stream_id.to_le_bytes()],
            &helix_treasury::ID,
        )
    }
}
