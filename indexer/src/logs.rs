//! Turning a transaction's log lines into attributed events.
//!
//! The runtime interleaves every program's output into one flat list, so a
//! `Program data:` line says nothing about who emitted it. The only way to know
//! is to track the invoke stack:
//!
//! ```text
//! Program <governance> invoke [1]
//! Program <treasury> invoke [2]
//! Program <token-2022> invoke [3]
//! Program <token-2022> success
//! Program data: ...        <- treasury's, not token-2022's and not governance's
//! Program <treasury> success
//! Program data: ...        <- governance's
//! Program <governance> success
//! ```
//!
//! That is not a contrived example — it is `execute_treasury_transfer`, the
//! deepest call stack in the protocol, and both of its events land after a
//! deeper program has already returned. Attributing by "most recent invoke" or
//! by "the transaction's top-level program" gets both wrong.

use anchor_lang::prelude::Pubkey;
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use std::str::FromStr as _;

use crate::event::{HelixEvent, Program};

/// An event, with enough context to store it exactly once.
#[derive(Clone, Debug, PartialEq)]
pub struct EmittedEvent {
    pub event: HelixEvent,
    /// Which program's invocation emitted it.
    pub program: Program,
    /// Index of the originating line in the transaction's log.
    ///
    /// The second half of the idempotency key. A signature alone is not enough:
    /// one transaction routinely emits several events, and two of them can be
    /// byte-identical — the same amount staked twice into the same position in
    /// one transaction is a legitimate thing to do.
    pub log_index: usize,
    /// CPI depth, 1 for a top-level instruction.
    pub depth: usize,
}

/// Something about the log that means the events below are not the whole story.
///
/// Reported rather than ignored. An indexer that quietly drops what it cannot
/// read produces analytics that are wrong precisely when something unusual
/// happened, which is when anyone is looking.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Anomaly {
    /// The runtime cut the log off. Events after this point were never written,
    /// and no amount of parsing recovers them — the transaction has to be
    /// re-fetched with inner instructions, or its accounts read directly.
    Truncated { log_index: usize },
    /// A `Program data:` line from one of our programs that did not decode.
    /// Either the payload is corrupt or the program emits an event this build
    /// does not know about, which means the indexer is older than the chain.
    UndecodableData { log_index: usize, program: Program },
    /// A `success`/`failed` line with no matching `invoke`, or invokes left open
    /// at the end. Attribution below such a point cannot be trusted.
    UnbalancedInvokeStack { log_index: usize },
}

/// Everything a transaction's log yields.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParsedLogs {
    pub events: Vec<EmittedEvent>,
    pub anomalies: Vec<Anomaly>,
}

impl ParsedLogs {
    /// Whether the events are known to be the complete set.
    ///
    /// Callers should refuse to commit an incomplete transaction rather than
    /// store a partial one — see [`Anomaly`].
    pub fn is_complete(&self) -> bool {
        self.anomalies.is_empty()
    }

    pub fn events_of(&self, program: Program) -> impl Iterator<Item = &EmittedEvent> {
        self.events.iter().filter(move |e| e.program == program)
    }
}

/// Parses one transaction's logs.
pub fn parse(logs: &[String]) -> ParsedLogs {
    let mut parsed = ParsedLogs::default();
    let mut stack: Vec<Option<Program>> = Vec::new();

    for (log_index, line) in logs.iter().enumerate() {
        let line = line.trim();

        if line.starts_with("Log truncated") {
            parsed.anomalies.push(Anomaly::Truncated { log_index });
            break;
        }

        if let Some(payload) = line.strip_prefix("Program data: ") {
            // Not one of ours, or emitted outside any invocation. Either way
            // there is nothing to attribute it to.
            let Some(Some(program)) = stack.last().copied() else {
                continue;
            };
            match BASE64
                .decode(payload.trim())
                .ok()
                .and_then(|bytes| HelixEvent::decode(program, &bytes))
            {
                Some(event) => parsed.events.push(EmittedEvent {
                    event,
                    program,
                    log_index,
                    depth: stack.len(),
                }),
                None => parsed
                    .anomalies
                    .push(Anomaly::UndecodableData { log_index, program }),
            }
            continue;
        }

        // `Program <id> invoke [n]`. Matched before the terminators below
        // because all three share the same prefix.
        if let Some(id) = line
            .strip_prefix("Program ")
            .and_then(|rest| rest.split_once(" invoke ["))
            .and_then(|(id, _)| Pubkey::from_str(id).ok())
        {
            stack.push(Program::from_id(&id));
            continue;
        }

        // `Program <id> success` / `Program <id> failed: ...`.
        //
        // A failed invocation still pops: its events are discarded by the
        // runtime along with its writes, and the frames above it continue. Note
        // that `Program <id> consumed N of M compute units` also matches the
        // outer prefix and must *not* pop, which is why the suffix is checked.
        if line.starts_with("Program ") && (line.ends_with(" success") || line.contains(" failed"))
        {
            if stack.pop().is_none() {
                parsed
                    .anomalies
                    .push(Anomaly::UnbalancedInvokeStack { log_index });
            }
            continue;
        }
    }

    // Frames still open means the log ended mid-invocation without being marked
    // truncated. Attribution held up to that point, but the transaction is not
    // whole.
    if !stack.is_empty()
        && !parsed
            .anomalies
            .iter()
            .any(|a| matches!(a, Anomaly::Truncated { .. }))
    {
        parsed.anomalies.push(Anomaly::UnbalancedInvokeStack {
            log_index: logs.len(),
        });
    }

    parsed
}

#[cfg(test)]
mod tests {
    use super::*;
    use anchor_lang::{AnchorSerialize, Discriminator};

    fn encode<T: Discriminator + AnchorSerialize>(event: &T) -> String {
        let mut bytes = T::DISCRIMINATOR.to_vec();
        event.serialize(&mut bytes).expect("serialize");
        format!("Program data: {}", BASE64.encode(bytes))
    }

    fn spent() -> helix_treasury::events::Spent {
        helix_treasury::events::Spent {
            treasury: Pubkey::new_unique(),
            destination: Pubkey::new_unique(),
            amount: 1_000,
            remaining_epoch_budget: 42,
            total_spent: 9_000,
            timestamp: 5,
        }
    }

    fn executed() -> helix_governance::events::ProposalExecuted {
        helix_governance::events::ProposalExecuted {
            proposal: Pubkey::new_unique(),
            action: helix_governance::state::ProposalAction::Signal,
            timestamp: 6,
        }
    }

    fn invoke(program: Program, depth: usize) -> String {
        format!("Program {} invoke [{depth}]", program.id())
    }

    fn success(program: Program) -> String {
        format!("Program {} success", program.id())
    }

    /// The shape of a real `execute_treasury_transfer`, captured from the
    /// runtime: two events, at two depths, each after a deeper program returned.
    ///
    /// Returns the events alongside the log so assertions compare against the
    /// exact values encoded, not against a second call to the constructors.
    fn nested_cpi_log() -> (
        Vec<String>,
        helix_treasury::events::Spent,
        helix_governance::events::ProposalExecuted,
    ) {
        let token_2022 = Pubkey::new_unique();
        let (spent, executed) = (spent(), executed());
        let logs = vec![
            invoke(Program::Governance, 1),
            "Program log: Instruction: ExecuteTreasuryTransfer".into(),
            invoke(Program::Treasury, 2),
            "Program log: Instruction: Spend".into(),
            format!("Program {token_2022} invoke [3]"),
            "Program log: Instruction: TransferChecked".into(),
            format!("Program {token_2022} consumed 1791 of 169607 compute units"),
            format!("Program {token_2022} success"),
            encode(&spent),
            format!(
                "Program {} consumed 15636 of 181900 compute units",
                Program::Treasury.id()
            ),
            success(Program::Treasury),
            encode(&executed),
            format!(
                "Program {} consumed 35883 of 200000 compute units",
                Program::Governance.id()
            ),
            success(Program::Governance),
        ];
        (logs, spent, executed)
    }

    #[test]
    fn attributes_events_to_the_invocation_that_emitted_them() {
        let (logs, spent, executed) = nested_cpi_log();
        let parsed = parse(&logs);
        assert!(parsed.is_complete(), "anomalies: {:?}", parsed.anomalies);
        assert_eq!(parsed.events.len(), 2);

        assert_eq!(parsed.events[0].program, Program::Treasury);
        assert_eq!(parsed.events[0].depth, 2);
        assert_eq!(parsed.events[0].event, HelixEvent::Spent(spent));

        assert_eq!(parsed.events[1].program, Program::Governance);
        assert_eq!(parsed.events[1].depth, 1);
        assert_eq!(
            parsed.events[1].event,
            HelixEvent::ProposalExecuted(executed)
        );
    }

    /// The failure this parser exists to avoid.
    ///
    /// Both events sit after a `success` line, so "attribute to the most recent
    /// invoke" hands the treasury's event to Token-2022 and drops it, and
    /// "attribute to the transaction's program" hands both to governance.
    #[test]
    fn a_returned_inner_program_does_not_capture_later_events() {
        let parsed = parse(&nested_cpi_log().0);
        assert!(
            parsed
                .events
                .iter()
                .all(|e| e.program != Program::Governance || e.depth == 1),
            "an event was attributed to the wrong frame: {:?}",
            parsed.events
        );
        assert_eq!(parsed.events_of(Program::Treasury).count(), 1);
        assert_eq!(parsed.events_of(Program::Governance).count(), 1);
    }

    #[test]
    fn log_indices_are_the_originating_lines() {
        let (logs, ..) = nested_cpi_log();
        let parsed = parse(&logs);
        for emitted in &parsed.events {
            assert!(
                logs[emitted.log_index].starts_with("Program data: "),
                "log_index {} points at {:?}",
                emitted.log_index,
                logs[emitted.log_index]
            );
        }
        // Distinct, which is what makes (signature, log_index) a key.
        assert_ne!(parsed.events[0].log_index, parsed.events[1].log_index);
    }

    #[test]
    fn events_from_foreign_programs_are_ignored_not_mangled() {
        let foreign = Pubkey::new_unique();
        let logs = vec![
            format!("Program {foreign} invoke [1]"),
            format!("Program data: {}", BASE64.encode([1u8; 32])),
            format!("Program {foreign} success"),
        ];
        let parsed = parse(&logs);
        assert!(parsed.events.is_empty());
        assert!(parsed.is_complete(), "anomalies: {:?}", parsed.anomalies);
    }

    #[test]
    fn a_truncated_log_is_reported_rather_than_silently_short() {
        let (mut logs, ..) = nested_cpi_log();
        logs.truncate(9);
        logs.push("Log truncated".into());
        let parsed = parse(&logs);

        assert_eq!(parsed.events.len(), 1, "the first event is still readable");
        assert!(!parsed.is_complete());
        assert_eq!(parsed.anomalies, vec![Anomaly::Truncated { log_index: 9 }]);
    }

    #[test]
    fn an_undecodable_payload_from_our_program_is_an_anomaly() {
        let logs = vec![
            invoke(Program::Staking, 1),
            format!("Program data: {}", BASE64.encode([9u8; 24])),
            success(Program::Staking),
        ];
        let parsed = parse(&logs);
        assert!(parsed.events.is_empty());
        assert_eq!(
            parsed.anomalies,
            vec![Anomaly::UndecodableData {
                log_index: 1,
                program: Program::Staking
            }]
        );
    }

    #[test]
    fn a_compute_line_does_not_close_a_frame() {
        let logs = vec![
            invoke(Program::Staking, 1),
            format!(
                "Program {} consumed 100 of 200 compute units",
                Program::Staking.id()
            ),
            encode(&spent()),
            success(Program::Staking),
        ];
        // The frame is still staking's, so a treasury event does not decode —
        // proving the compute line did not pop the stack and leave it empty.
        let parsed = parse(&logs);
        assert!(parsed.events.is_empty());
        assert_eq!(parsed.anomalies.len(), 1);
        assert!(matches!(
            parsed.anomalies[0],
            Anomaly::UndecodableData {
                program: Program::Staking,
                ..
            }
        ));
    }

    #[test]
    fn a_failed_inner_invocation_still_pops() {
        let logs = vec![
            invoke(Program::Governance, 1),
            invoke(Program::Treasury, 2),
            format!(
                "Program {} failed: custom program error: 0x1",
                Program::Treasury.id()
            ),
            encode(&executed()),
            success(Program::Governance),
        ];
        let parsed = parse(&logs);
        assert!(parsed.is_complete(), "anomalies: {:?}", parsed.anomalies);
        assert_eq!(parsed.events.len(), 1);
        assert_eq!(parsed.events[0].program, Program::Governance);
        assert_eq!(parsed.events[0].depth, 1);
    }

    #[test]
    fn an_unbalanced_stack_is_reported() {
        let unclosed = vec![invoke(Program::Staking, 1)];
        assert!(matches!(
            parse(&unclosed).anomalies.as_slice(),
            [Anomaly::UnbalancedInvokeStack { .. }]
        ));

        let extra_close = vec![success(Program::Staking)];
        assert!(matches!(
            parse(&extra_close).anomalies.as_slice(),
            [Anomaly::UnbalancedInvokeStack { log_index: 0 }]
        ));
    }

    #[test]
    fn an_empty_log_yields_nothing_and_complains_about_nothing() {
        assert_eq!(parse(&[]), ParsedLogs::default());
    }
}
