//! Test-only helpers for manipulating durable-state fixtures.
//!
//! These routines exist solely to *simulate crashes* for recovery tests. They
//! rewrite supposedly-immutable durable artifacts on disk. They are deliberately
//! NOT part of the production API: no production code should ever "rewrite the
//! journal because the test asked nicely."

use prometheos_lite::workflow::evaluate::EvaluationState;
use std::path::Path;

/// Rewind the on-disk durable journal and checkpoint so the journal ends at
/// `target` (a non-terminal stage), faithfully simulating a crash at that
/// point.
///
/// The journal is the authoritative recovery source and a completed run now
/// records a terminal outcome. Tests that rewind the registry entry to a
/// non-terminal state (to exercise takeover/resume) must ALSO rewind the
/// journal and remove its checkpoint; otherwise recovery correctly refuses to
/// resume a journal that already reached a terminal outcome.
pub(crate) fn rewind_durable_to(repo: &Path, target: EvaluationState) {
    let tag = serde_json::to_value(target)
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();
    let journal_root = repo.join(".prometheos").join("workflow").join("journal");
    if journal_root.exists() {
        for identity_dir in std::fs::read_dir(&journal_root)
            .unwrap()
            .flatten()
            .filter_map(|e| e.path().is_dir().then(|| e.path()))
        {
            let mut events: Vec<std::path::PathBuf> = std::fs::read_dir(&identity_dir)
                .unwrap()
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "json"))
                .collect();
            events.sort();
            // Keep the contiguous prefix ending at the last event that reached
            // `target`; remove everything after it (the target event stays as
            // the last durable record).
            let mut last_at_target: Option<usize> = None;
            for (i, path) in events.iter().enumerate() {
                let text = std::fs::read_to_string(path).unwrap();
                let value: serde_json::Value = serde_json::from_str(&text).unwrap();
                if value.get("to_state").and_then(|v| v.as_str()) == Some(tag.as_str()) {
                    last_at_target = Some(i);
                }
            }
            // If the target state never appears (unexpected), drop the whole
            // journal to a clean state so recovery reports no durable record.
            let keep = last_at_target;
            for (i, path) in events.iter().enumerate() {
                if keep.is_none() || Some(i) != keep {
                    std::fs::remove_file(path).unwrap();
                }
            }
        }
    }
    // Remove checkpoints so a stale snapshot cannot disagree with the journal.
    let checkpoint_dir = repo.join(".prometheos").join("workflow").join("checkpoint");
    if checkpoint_dir.exists() {
        for entry in std::fs::read_dir(&checkpoint_dir).unwrap().flatten() {
            if entry.path().extension().is_some_and(|x| x == "json") {
                std::fs::remove_file(entry.path()).unwrap();
            }
        }
    }
}
