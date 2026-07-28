//! Toolchain smoke test: does LiteSVM load our BPF programs at all?
//!
//! Deliberately the first test written. It proves the harness works before any
//! protocol logic depends on it.

use litesvm::LiteSVM;

const PROGRAMS: [(&str, &str); 4] = [
    ("helix_token_manager", "token_manager"),
    ("helix_staking", "staking"),
    ("helix_governance", "governance"),
    ("helix_treasury", "treasury"),
];

fn so_path(lib_name: &str) -> std::path::PathBuf {
    // Tests run with CWD at the crate root, so walk up to the workspace root.
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/deploy")
        .join(format!("{lib_name}.so"))
}

#[test]
fn all_four_programs_load() {
    let mut svm = LiteSVM::new();

    for (lib_name, _) in PROGRAMS {
        let path = so_path(lib_name);
        assert!(
            path.exists(),
            "{} not built — run `anchor build` first",
            path.display()
        );
    }

    let ids = [
        helix_token_manager::ID,
        helix_staking::ID,
        helix_governance::ID,
        helix_treasury::ID,
    ];

    for ((lib_name, _), id) in PROGRAMS.iter().zip(ids) {
        let bytes = std::fs::read(so_path(lib_name)).unwrap();
        svm.add_program(id, &bytes)
            .unwrap_or_else(|e| panic!("{lib_name} failed to load: {e:?}"));

        let account = svm
            .get_account(&id)
            .unwrap_or_else(|| panic!("{lib_name} did not load at {id}"));
        assert!(
            account.executable,
            "{lib_name} loaded but is not executable"
        );
    }
}

#[test]
fn program_ids_are_distinct() {
    // A copy-paste in `declare_id!` would make two programs share an address and
    // fail in confusing ways much later.
    let ids = [
        helix_token_manager::ID,
        helix_staking::ID,
        helix_governance::ID,
        helix_treasury::ID,
    ];
    for (i, a) in ids.iter().enumerate() {
        for b in ids.iter().skip(i + 1) {
            assert_ne!(a, b, "two programs declare the same ID");
        }
    }
}
