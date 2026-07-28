//! Every event the four programs can emit, and how to decode one.
//!
//! Anchor writes an event as an 8-byte discriminator followed by a Borsh body,
//! base64-encoded onto a `Program data:` log line. Decoding is therefore: work
//! out which program emitted it, match the discriminator, deserialise the rest.
//!
//! The variants here are the programs' own event structs rather than copies. A
//! field added to `Staked` shows up in the indexer without anyone editing the
//! indexer, and a field *removed* is a compile error rather than a column that
//! quietly stops being populated.

use anchor_lang::prelude::Pubkey;
use anchor_lang::{AnchorDeserialize, Discriminator};

/// Splits Anchor's 8-byte discriminator off `data`, returning the Borsh body if
/// it belongs to `T`.
fn body_of<T: Discriminator>(data: &[u8]) -> Option<&[u8]> {
    let (discriminator, body) = data.split_at_checked(T::DISCRIMINATOR.len())?;
    (discriminator == T::DISCRIMINATOR).then_some(body)
}

macro_rules! helix_events {
    ($(
        $program:ident ( $id:expr ) {
            $( $variant:ident : $ty:ty ),* $(,)?
        }
    )*) => {
        /// The programs that make up the protocol.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Program {
            $( $program, )*
        }

        impl Program {
            /// Which of ours a program id belongs to, if any.
            ///
            /// Returning `None` for everything else is what lets the log parser
            /// walk transactions that also touch Token-2022 or the system
            /// program without special-casing them.
            pub fn from_id(id: &Pubkey) -> Option<Self> {
                $( if *id == $id { return Some(Self::$program); } )*
                None
            }

            pub fn id(self) -> Pubkey {
                match self { $( Self::$program => $id, )* }
            }

            pub fn name(self) -> &'static str {
                match self { $( Self::$program => stringify!($program), )* }
            }
        }

        /// A decoded event, tagged with which one it is.
        ///
        /// Deliberately a closed enum. An indexer that accepts "some event I do
        /// not recognise" will happily skip the one transition that mattered, so
        /// an unknown discriminator is reported as an anomaly instead — see
        /// [`crate::logs::Anomaly::UndecodableData`].
        #[derive(Clone, Debug, PartialEq)]
        pub enum HelixEvent {
            $( $( $variant($ty), )* )*
        }

        impl HelixEvent {
            /// Decodes one `Program data:` payload emitted by `program`.
            pub fn decode(program: Program, data: &[u8]) -> Option<Self> {
                match program {
                    $(
                        Program::$program => {
                            $(
                                if let Some(body) = body_of::<$ty>(data) {
                                    // Anchor's `try_from_slice` rejects trailing
                                    // bytes, so a payload that merely starts with
                                    // a matching discriminator still fails here
                                    // rather than decoding into nonsense.
                                    return <$ty>::try_from_slice(body).ok().map(Self::$variant);
                                }
                            )*
                            None
                        }
                    )*
                }
            }

            pub fn program(&self) -> Program {
                match self { $( $( Self::$variant(_) => Program::$program, )* )* }
            }

            /// The variant name, for logging and for the `kind` column.
            pub fn name(&self) -> &'static str {
                match self { $( $( Self::$variant(_) => stringify!($variant), )* )* }
            }
        }
    };
}

helix_events! {
    TokenManager (helix_token_manager::ID) {
        TokenInitialized:      helix_token_manager::events::TokenInitialized,
        MinterRegistered:      helix_token_manager::events::MinterRegistered,
        MinterUpdated:         helix_token_manager::events::MinterUpdated,
        MinterRevoked:         helix_token_manager::events::MinterRevoked,
        TokensMinted:          helix_token_manager::events::TokensMinted,
        TokensBurned:          helix_token_manager::events::TokensBurned,
        AdminTransferProposed: helix_token_manager::events::AdminTransferProposed,
        AdminTransferCancelled: helix_token_manager::events::AdminTransferCancelled,
        AdminTransferAccepted: helix_token_manager::events::AdminTransferAccepted,
        PauseToggled:          helix_token_manager::events::PauseToggled,
    }

    Staking (helix_staking::ID) {
        PoolInitialized:            helix_staking::events::PoolInitialized,
        Staked:                     helix_staking::events::Staked,
        Unstaked:                   helix_staking::events::Unstaked,
        RewardsClaimed:             helix_staking::events::RewardsClaimed,
        RewardsFunded:              helix_staking::events::RewardsFunded,
        RewardRateChanged:          helix_staking::events::RewardRateChanged,
        PoolPauseToggled:           helix_staking::events::PoolPauseToggled,
        AuthorityTransferProposed:  helix_staking::events::AuthorityTransferProposed,
        AuthorityTransferAccepted:  helix_staking::events::AuthorityTransferAccepted,
    }

    Governance (helix_governance::ID) {
        RealmInitialized:  helix_governance::events::RealmInitialized,
        ProposalCreated:   helix_governance::events::ProposalCreated,
        ProposalActivated: helix_governance::events::ProposalActivated,
        VoteCast:          helix_governance::events::VoteCast,
        ProposalFinalized: helix_governance::events::ProposalFinalized,
        ProposalQueued:    helix_governance::events::ProposalQueued,
        ProposalExecuted:  helix_governance::events::ProposalExecuted,
        ProposalCancelled: helix_governance::events::ProposalCancelled,
    }

    Treasury (helix_treasury::ID) {
        TreasuryInitialized:       helix_treasury::events::TreasuryInitialized,
        Deposited:                 helix_treasury::events::Deposited,
        Spent:                     helix_treasury::events::Spent,
        StreamCreated:             helix_treasury::events::StreamCreated,
        StreamClaimed:             helix_treasury::events::StreamClaimed,
        StreamRevoked:             helix_treasury::events::StreamRevoked,
        SpendCapChanged:           helix_treasury::events::SpendCapChanged,
        GovernanceExecutorChanged: helix_treasury::events::GovernanceExecutorChanged,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anchor_lang::AnchorSerialize;

    /// Builds the wire form Anchor would emit for `event`.
    fn wire<T: Discriminator + AnchorSerialize>(event: &T) -> Vec<u8> {
        let mut bytes = T::DISCRIMINATOR.to_vec();
        event.serialize(&mut bytes).expect("serialize");
        bytes
    }

    fn staked() -> helix_staking::events::Staked {
        helix_staking::events::Staked {
            pool: Pubkey::new_unique(),
            position: Pubkey::new_unique(),
            owner: Pubkey::new_unique(),
            position_id: 7,
            amount_sent: 1_000,
            amount_credited: 990,
            weighted_amount: 1_237,
            tier: helix_staking::state::LockTier::Bronze,
            lock_end: 1_234,
            timestamp: 99,
        }
    }

    #[test]
    fn round_trips_through_the_wire_format() {
        let event = staked();
        let decoded = HelixEvent::decode(Program::Staking, &wire(&event));
        assert_eq!(decoded, Some(HelixEvent::Staked(event)));
    }

    #[test]
    fn program_ids_are_distinct_and_round_trip() {
        let all = [
            Program::TokenManager,
            Program::Staking,
            Program::Governance,
            Program::Treasury,
        ];
        for program in all {
            assert_eq!(Program::from_id(&program.id()), Some(program));
        }
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert_ne!(a.id(), b.id(), "{} and {} share an id", a.name(), b.name());
            }
        }
    }

    #[test]
    fn a_foreign_program_id_is_not_ours() {
        assert_eq!(Program::from_id(&Pubkey::new_unique()), None);
        assert_eq!(Program::from_id(&anchor_lang::system_program::ID), None);
    }

    /// The same bytes emitted by a different program must not decode.
    ///
    /// Discriminators are only 8 bytes of a hash of the *type name*, so nothing
    /// makes them globally unique — two programs could legitimately share one.
    /// Attribution has to come from the log's invoke stack, and this pins that
    /// the decoder respects it rather than trying every program in turn.
    #[test]
    fn decoding_is_scoped_to_the_emitting_program() {
        let bytes = wire(&staked());
        assert!(HelixEvent::decode(Program::Staking, &bytes).is_some());
        assert_eq!(HelixEvent::decode(Program::Treasury, &bytes), None);
        assert_eq!(HelixEvent::decode(Program::Governance, &bytes), None);
        assert_eq!(HelixEvent::decode(Program::TokenManager, &bytes), None);
    }

    #[test]
    fn a_truncated_payload_does_not_decode() {
        let bytes = wire(&staked());
        for cut in [0, 1, 7, 8, 9, bytes.len() - 1] {
            assert_eq!(
                HelixEvent::decode(Program::Staking, &bytes[..cut]),
                None,
                "{cut} bytes should not decode"
            );
        }
    }

    /// A correct discriminator followed by extra bytes is not a valid event.
    ///
    /// Borsh would otherwise stop at the end of the struct and ignore the rest,
    /// which is how a subtly wrong payload becomes a confidently wrong row.
    #[test]
    fn trailing_bytes_are_rejected() {
        let mut bytes = wire(&staked());
        bytes.push(0);
        assert_eq!(HelixEvent::decode(Program::Staking, &bytes), None);
    }

    #[test]
    fn name_and_program_agree_with_the_variant() {
        let event = HelixEvent::Staked(staked());
        assert_eq!(event.name(), "Staked");
        assert_eq!(event.program(), Program::Staking);
    }
}
