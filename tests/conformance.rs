//! Every vendored case is a test: its payload byte for byte and its trace field for field, apart from
//! `trace_id`, `timings` and `recovery.detail`; and every rejection snapshot is rejected before assembly
//! (conformance/README.md, Running a case).
//!
//! `PENDING` holds the cases this package does not pass yet, as strict expected failures: the suite fails as soon
//! as one starts passing, so the set only shrinks. Rejection cases have no such set. A case that needs an optional
//! component this package lacks is skipped, not held here; this package provides every published one.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use contextwindowarchitecture_assembler::conformance::{ids, run_case, run_rejection};

const PENDING: &[&str] = &[];

fn conformance() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("vendor/cwa/conformance")
}

#[test]
fn every_case_passes_except_those_pending() {
    let dir = conformance().join("cases");
    let mut failing = BTreeSet::new();
    let mut details = Vec::new();
    for id in ids(&dir) {
        let outcome = run_case(&dir.join(&id));
        match outcome.outcome {
            "passed" => {}
            "skipped" => panic!("{id} was skipped, but this package provides every published component: {:?}", outcome.detail),
            _ => {
                details.push(format!("{id}: {}", outcome.detail.unwrap_or_default()));
                failing.insert(id);
            }
        }
    }
    let pending: BTreeSet<String> = PENDING.iter().map(|s| s.to_string()).collect();
    let newly_passing: Vec<&String> = pending.difference(&failing).collect();
    let newly_failing: Vec<&String> = failing.difference(&pending).collect();
    assert!(newly_passing.is_empty(), "these cases pass now; remove them from PENDING: {newly_passing:?}");
    assert!(newly_failing.is_empty(), "these cases fail:\n{}", details.join("\n"));
}

#[test]
fn every_rejection_snapshot_is_rejected() {
    let dir = conformance().join("rejections");
    let wrong: Vec<String> = ids(&dir).into_iter()
        .map(|id| (run_rejection(&dir.join(&id)), id))
        .filter(|(outcome, _)| outcome.outcome != "rejected")
        .map(|(outcome, id)| format!("{id}: {} {}", outcome.outcome, outcome.detail.unwrap_or_default()))
        .collect();
    assert!(wrong.is_empty(), "not rejected:\n{}", wrong.join("\n"));
}

#[test]
fn every_vendored_case_directory_is_run() {
    let lock: serde_json::Value = serde_json::from_slice(&std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("vendor/cwa.lock.json")).unwrap()).unwrap();
    for kind in ["cases", "rejections"] {
        let prefix = format!("conformance/{kind}/");
        let locked: BTreeSet<&str> = lock["files"].as_object().unwrap().keys()
            .filter_map(|path| path.strip_prefix(&prefix)).filter_map(|rest| rest.split_once('/').map(|(id, _)| id)).collect();
        let found = ids(&conformance().join(kind));
        assert!(!found.is_empty(), "no {kind} found");
        assert_eq!(found.iter().map(String::as_str).collect::<BTreeSet<_>>(), locked, "{kind}");
    }
}
