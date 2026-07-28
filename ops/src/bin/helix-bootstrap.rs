//! Prints the atomic bootstrap before anyone sends it.
//!
//! ```bash
//! cargo run -p helix-ops --bin helix-bootstrap -- \
//!     --payer <PUBKEY> --mint <PUBKEY> --guardian <PUBKEY>
//!
//! # ... and the instructions in a form any client can submit:
//! cargo run -p helix-ops --bin helix-bootstrap -- ... --json > bootstrap.json
//! ```
//!
//! The point of a plan you can read is that the bootstrap is a one-shot
//! transaction against a front-running window (F-1). There is no rehearsal: get
//! an account wrong and you find out on mainnet, at the one moment an attacker
//! is watching for it. So this prints who will control what, and refuses to
//! print anything at all if the answer is "a human key".

use anchor_lang::prelude::Pubkey;
use helix_governance::instructions::realm::RealmParams;
use helix_ops::{plan, to_json, BootstrapConfig};
use std::str::FromStr as _;

const USAGE: &str = "\
helix-bootstrap — plan the atomic bootstrap

    --payer     <PUBKEY>   funds account rent; ends up controlling nothing
    --mint      <PUBKEY>   the HLX mint, which must already exist
    --guardian  <PUBKEY>   may only veto; intended to be a multisig
    --json                 emit the instructions instead of the report

Realm parameters default to the values in docs/RUNBOOK.md and can be set with
--quorum-bps, --approval-bps, --voting-period, --timelock-delay and
--min-weight-to-propose.";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{USAGE}");
        return;
    }

    let get = |name: &str| -> Option<String> {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let key = |name: &str| -> Pubkey {
        let raw = get(name).unwrap_or_else(|| fail(&format!("missing {name}\n\n{USAGE}")));
        Pubkey::from_str(&raw).unwrap_or_else(|_| fail(&format!("{name} is not a pubkey: {raw}")))
    };
    let number = |name: &str, default: i64| -> i64 {
        get(name)
            .map(|v| {
                v.parse()
                    .unwrap_or_else(|_| fail(&format!("{name} is not a number")))
            })
            .unwrap_or(default)
    };

    let config = BootstrapConfig {
        payer: key("--payer"),
        mint: key("--mint"),
        guardian: key("--guardian"),
        realm: RealmParams {
            quorum_bps: number("--quorum-bps", 2_000) as u16,
            approval_bps: number("--approval-bps", 5_001) as u16,
            voting_period: number("--voting-period", 3 * 86_400),
            timelock_delay: number("--timelock-delay", 2 * 86_400),
            min_weight_to_propose: number("--min-weight-to-propose", 1_000) as u64,
        },
        epoch_spend_cap: number("--epoch-spend-cap", 1_000_000_000) as u64,
        epoch_duration: number("--epoch-duration", 24 * 3_600),
    };

    // Validated here rather than by the cluster, because the cluster rejects the
    // whole transaction and the operator then has to work out which of five
    // numbers was wrong while the window is open.
    if let Err(e) = config.realm.validate() {
        fail(&format!(
            "realm parameters would be rejected on chain: {e:?}"
        ));
    }

    let plan = plan(&config);
    let parties = plan.privileged_parties();

    // The refusal that makes the tool worth running. If any authority is not the
    // executor PDA, the bootstrap installs a key that can bypass governance, and
    // printing a plan that does that would be handing someone a loaded foot-gun.
    for (name, holder) in [
        ("pool authority", parties.pool_authority),
        ("realm authority", parties.realm_authority),
        ("treasury spender", parties.treasury_spender),
    ] {
        if holder != plan.addresses.executor {
            fail(&format!(
                "refusing to emit: {name} would be {holder}, not the executor PDA {}",
                plan.addresses.executor
            ));
        }
    }

    if args.iter().any(|a| a == "--json") {
        println!(
            "{}",
            serde_json::to_string_pretty(&to_json(&plan)).expect("serialise")
        );
        return;
    }

    let size = plan.transaction_size();
    let a = plan.addresses;

    println!("Atomic bootstrap — one transaction, three programs, no window (F-1)\n");
    println!(
        "  transaction   {size} bytes of 1232, {} accounts",
        plan.account_count()
    );
    println!("  instructions  {}\n", plan.instructions.len());

    println!("Derived from the mint alone:");
    println!("  pool               {}", a.pool);
    println!("  stake vault        {}", a.stake_vault);
    println!("  reward vault       {}", a.reward_vault);
    println!("  realm              {}", a.realm);
    println!("  executor           {}", a.executor);
    println!("  treasury           {}", a.treasury);
    println!("  treasury vault     {}\n", a.treasury_vault);

    println!("Who controls what once this lands:");
    println!(
        "  pool authority     {}  (executor)",
        parties.pool_authority
    );
    println!(
        "  realm authority    {}  (executor)",
        parties.realm_authority
    );
    println!(
        "  treasury spender   {}  (executor)",
        parties.treasury_spender
    );
    println!("  guardian           {}  (veto only)\n", parties.guardian);

    println!("The payer {} controls none of the above.", config.payer);
    println!();
    println!("Still to do by hand, because neither can be atomic — see docs/RUNBOOK.md:");
    println!("  * register the first minter, then hand the token-manager admin to");
    println!("    governance (F-9): only an admin can register a minter, so the role");
    println!("    has to start as a human key");
    println!("  * verify every authority on chain afterwards; the checklist assumes");
    println!("    nothing this tool printed actually happened");

    if size > 1232 {
        fail("\nthe transaction exceeds the packet limit and cannot be sent atomically");
    }
}

fn fail(message: &str) -> ! {
    eprintln!("{message}");
    std::process::exit(1);
}
