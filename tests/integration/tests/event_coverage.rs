//! The indexer must know every event the programs can emit.
//!
//! `event.rs` argues, correctly, that using the programs' own structs makes a
//! *changed field* a compile error. It does not make a *new event type* anything
//! at all: Rust cannot enumerate the types in a module, so the variant list is
//! hand-maintained, and an event added to a program compiles fine on both sides
//! while never decoding.
//!
//! That is not a hypothetical failure mode — it is what happened.
//! `RealmParamsUpdated` and `RealmAuthorityChanged` were added to governance in
//! Phase 7.1 (the F-11 fix) and were absent from `HelixEvent` until Phase 4.2.
//! For two phases the indexer would have reported the two events that record
//! governance becoming self-governing as `Anomaly::UndecodableData` — the one
//! transition the whole protocol exists to make, arriving as "I do not recognise
//! this".
//!
//! The check compares against the IDLs `anchor build` writes, because the IDL is
//! generated from the programs and therefore cannot drift from them the way a
//! hand-written list can. A test that compared the list to a number would have
//! passed the entire time; this one fails the moment a program grows an event.

use helix_indexer::event::{HelixEvent, Program};
use std::collections::BTreeSet;
use std::path::PathBuf;

/// Where `anchor build` leaves the generated IDLs.
fn idl_path(program: Program) -> PathBuf {
    let lib = match program {
        Program::TokenManager => "helix_token_manager",
        Program::Staking => "helix_staking",
        Program::Governance => "helix_governance",
        Program::Treasury => "helix_treasury",
    };
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/idl")
        .join(format!("{lib}.json"))
}

/// Event names the IDL says `program` can emit.
fn declared_by_the_chain(program: Program) -> BTreeSet<String> {
    let path = idl_path(program);
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "cannot read {} ({e}) — run `anchor build` first",
            path.display()
        )
    });
    let idl: serde_json::Value = serde_json::from_str(&raw).expect("IDL is not valid JSON");

    let events = idl
        .get("events")
        .and_then(|e| e.as_array())
        .unwrap_or_else(|| panic!("{} has no `events` array", path.display()));

    // An IDL with no events at all would make this test vacuously pass, which is
    // the failure mode of every check written against generated input.
    assert!(
        !events.is_empty(),
        "{} declares no events, so this test proves nothing",
        path.display()
    );

    events
        .iter()
        .map(|e| {
            e.get("name")
                .and_then(|n| n.as_str())
                .expect("IDL event without a name")
                .to_owned()
        })
        .collect()
}

#[test]
fn the_indexer_decodes_every_event_the_programs_declare() {
    let known: BTreeSet<&str> = HelixEvent::NAMES.iter().copied().collect();

    // Iterating `Program::ALL` rather than a written-out list, so a fifth program
    // is covered without a second edit here.
    let mut missing: Vec<String> = Vec::new();
    for program in Program::ALL {
        for event in declared_by_the_chain(*program) {
            if !known.contains(event.as_str()) {
                missing.push(format!("{}::{event}", program.name()));
            }
        }
    }

    assert!(
        missing.is_empty(),
        "the programs emit events this build cannot decode, so they would arrive \
         as Anomaly::UndecodableData and contribute nothing: {missing:#?}\n\
         Add them to the `helix_events!` list in indexer/src/event.rs.",
    );
}

/// The converse: a variant the chain no longer declares.
///
/// Less dangerous — a name that never arrives is dead code, not lost data — but
/// worth failing on, because the usual cause is a rename, and a rename presents
/// as one missing name and one extra one. Only the pair identifies it.
#[test]
fn the_indexer_declares_no_event_the_programs_do_not() {
    let declared: BTreeSet<String> = Program::ALL
        .iter()
        .flat_map(|program| declared_by_the_chain(*program))
        .collect();

    let stale: Vec<&str> = HelixEvent::NAMES
        .iter()
        .copied()
        .filter(|name| !declared.contains(*name))
        .collect();

    assert!(
        stale.is_empty(),
        "the indexer declares events no program emits — usually half of a rename: {stale:?}",
    );
}

/// Every event carries an on-chain timestamp, and it is the one the chain wrote.
///
/// The `block_time` column is populated from `HelixEvent::timestamp()`, which the
/// macro generates by reaching into every variant's `timestamp` field. This pins
/// the value against the body rather than against ingestion time — storing the
/// latter would make history depend on when the indexer happened to be running.
#[test]
fn the_timestamp_comes_from_the_event_body() {
    let event = HelixEvent::Staked(helix_staking::events::Staked {
        pool: anchor_lang::prelude::Pubkey::new_unique(),
        position: anchor_lang::prelude::Pubkey::new_unique(),
        owner: anchor_lang::prelude::Pubkey::new_unique(),
        position_id: 0,
        amount_sent: 1,
        amount_credited: 1,
        weighted_amount: 1,
        tier: helix_staking::state::LockTier::Flexible,
        lock_end: 0,
        timestamp: 1_700_000_000,
    });
    assert_eq!(event.timestamp(), 1_700_000_000);
}
