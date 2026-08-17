//! The dashboard's copy of the IDLs must be the IDLs.
//!
//! The dashboard builds every transaction from the IDL rather than from
//! hand-written discriminators and account orders — instruction data, account
//! order, writable and signer flags, PDA seeds and error codes all come from
//! `app/src/idl/`. That removes a whole class of client drift, and replaces it
//! with exactly one seam: those files are a *copy* of what `anchor build`
//! generates, because `target/` is gitignored and the CI job that builds the
//! dashboard has no Anchor toolchain.
//!
//! This test is that seam's guard. It runs in the job that does have
//! `anchor build` output, and it fails if a copy has stopped matching.
//!
//! # Why it matters more than the event list did
//!
//! `event_coverage.rs` exists because a hand-maintained list stopped decoding
//! two events — bad, and *visible*: the indexer reported
//! `Anomaly::UndecodableData`. A stale account order is worse in kind. Anchor's
//! account list is positional and carries no names on the wire, so a client
//! using yesterday's order does not fail to encode. It builds a well-formed
//! transaction that names the right accounts in the wrong slots, and asks a
//! wallet to sign it. The program then either refuses it for a reason that
//! points at the wrong thing, or — where two accounts of the same type were
//! swapped — does not refuse it at all.
//!
//! Regenerate with `node app/scripts/sync-idl.mjs` after `anchor build`.

use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Every IDL the dashboard has checked in, by program library name.
fn copied_by_the_dashboard() -> Vec<(String, PathBuf)> {
    let directory = repo_root().join("app/src/idl");

    let entries = std::fs::read_dir(&directory)
        .unwrap_or_else(|e| panic!("cannot read {} ({e})", directory.display()));

    let mut found: Vec<(String, PathBuf)> = entries
        .filter_map(|entry| {
            let path = entry.expect("unreadable directory entry").path();
            let name = path.file_stem()?.to_str()?.to_owned();
            (path.extension()? == "ts").then_some((name, path))
        })
        .collect();

    // An empty directory would make every assertion below vacuous — the failure
    // mode of every test written against generated input.
    assert!(
        !found.is_empty(),
        "{} contains no IDL modules, so this test proves nothing. \
         Run `node app/scripts/sync-idl.mjs`.",
        directory.display()
    );

    found.sort();
    found
}

/// The object literal out of a generated `.ts` module, as JSON.
///
/// The module is `const idl: Idl = { .. };` with a banner above and an export
/// below. Extracting the literal rather than parsing TypeScript is sound because
/// `sync-idl.mjs` writes it with `JSON.stringify` — the body is JSON by
/// construction, and if it ever stops being JSON this fails loudly rather than
/// comparing something else.
fn embedded_idl(path: &PathBuf) -> serde_json::Value {
    let source = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("cannot read {} ({e})", path.display()))
        // `.gitattributes` pins `.ts` to LF, but a file written by a Windows
        // tool that ignores it would otherwise fail the search below and be
        // reported as truncated — a misleading message for a harmless
        // difference.
        .replace("\r\n", "\n");

    const OPEN: &str = "const idl: Idl = ";
    const CLOSE: &str = "\n\nexport default idl;";

    let start = source.find(OPEN).unwrap_or_else(|| {
        panic!(
            "{} is not a generated IDL module — no `{OPEN}`. Did someone edit it by hand?",
            path.display()
        )
    }) + OPEN.len();

    let end = source.rfind(CLOSE).unwrap_or_else(|| {
        panic!(
            "{} has no trailing export — it is truncated",
            path.display()
        )
    });

    // Trim the statement's semicolon.
    let literal = source[start..end].trim().trim_end_matches(';');

    serde_json::from_str(literal).unwrap_or_else(|e| {
        panic!(
            "{} does not contain a JSON object literal ({e}) — regenerate it",
            path.display()
        )
    })
}

fn generated_idl(library: &str) -> serde_json::Value {
    let path = repo_root()
        .join("target/idl")
        .join(format!("{library}.json"));
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "cannot read {} ({e}) — run `anchor build` first",
            path.display()
        )
    });
    serde_json::from_str(&raw).expect("generated IDL is not valid JSON")
}

#[test]
fn the_dashboard_builds_transactions_from_the_current_idls() {
    for (library, path) in copied_by_the_dashboard() {
        let copied = embedded_idl(&path);
        let generated = generated_idl(&library);

        assert_eq!(
            copied,
            generated,
            "{} is not what `anchor build` now generates for {library}.\n\
             The dashboard would build transactions against a program that has changed \
             underneath it — account order is positional and unnamed on the wire, so this \
             does not fail to encode, it encodes something else.\n\
             Regenerate with: node app/scripts/sync-idl.mjs",
            path.display()
        );
    }
}

/// The address the dashboard sends to is the address `declare_id!` names.
///
/// Separate from the equality check above so a failure says which of the two
/// things went wrong. This one has its own failure mode: the copy could be a
/// current IDL from a *different* build of the workspace, with every instruction
/// identical and the program id from someone else's keypair.
#[test]
fn the_dashboard_targets_the_declared_program_ids() {
    let declared = [
        ("helix_staking", helix_staking::ID),
        ("helix_governance", helix_governance::ID),
    ];

    for (library, id) in declared {
        let path = repo_root()
            .join("app/src/idl")
            .join(format!("{library}.ts"));
        if !path.exists() {
            continue;
        }

        let address = embedded_idl(&path)
            .get("address")
            .and_then(|a| a.as_str())
            .unwrap_or_else(|| panic!("{} has no address", path.display()))
            .to_owned();

        assert_eq!(
            address,
            id.to_string(),
            "{} would send transactions to {address}, but {library} declares {id}",
            path.display()
        );
    }
}

/// The instructions the dashboard drives must still exist, by name.
///
/// The equality test above catches a *changed* IDL; this catches the narrower
/// and more likely case of a renamed instruction, and names the flow that would
/// break. Without it the first symptom is a runtime throw in the browser.
#[test]
fn every_instruction_the_dashboard_drives_still_exists() {
    let required: [(&str, &[&str]); 2] = [
        (
            "helix_staking",
            &["stake", "unstake", "claim", "close_position"],
        ),
        ("helix_governance", &["cast_vote"]),
    ];

    for (library, instructions) in required {
        let idl = generated_idl(library);
        let declared: Vec<&str> = idl["instructions"]
            .as_array()
            .expect("IDL has no instructions array")
            .iter()
            .filter_map(|i| i["name"].as_str())
            .collect();

        for wanted in instructions {
            assert!(
                declared.contains(wanted),
                "{library} no longer has `{wanted}`, which the dashboard's write flows call. \
                 It declares: {declared:?}"
            );
        }
    }
}
