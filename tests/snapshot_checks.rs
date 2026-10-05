//! Every rejection snapshot is rejected before assembly, and every case's snapshot passes and has the digest
//! its expected trace records (conformance/README.md, Snapshot checks, Snapshot digest).

use std::fs;
use std::path::{Path, PathBuf};

use cwa_assembler::{check, Error, Options};

fn dirs(kind: &str) -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("vendor/cwa/conformance").join(kind);
    let mut dirs: Vec<PathBuf> = fs::read_dir(root).unwrap().map(|e| e.unwrap().path()).filter(|p| p.is_dir()).collect();
    dirs.sort();
    dirs
}

#[test]
fn every_rejection_snapshot_is_rejected() {
    let mut wrong = Vec::new();
    for dir in dirs("rejections") {
        match check(&fs::read(dir.join("snapshot.json")).unwrap(), &Options::new()) {
            Err(Error::Rejected(_)) => {}
            other => wrong.push(format!("{}: {other:?}", dir.file_name().unwrap().to_string_lossy())),
        }
    }
    assert!(wrong.is_empty(), "not rejected:\n{}", wrong.join("\n"));
}

#[test]
fn every_case_snapshot_loads_with_its_recorded_digest() {
    let mut wrong = Vec::new();
    for dir in dirs("cases") {
        let trace: serde_json::Value = serde_json::from_slice(&fs::read(dir.join("expected.trace.json")).unwrap()).unwrap();
        let expected = trace["context"]["snapshot_digest"].as_str().unwrap().to_string();
        match check(&fs::read(dir.join("snapshot.json")).unwrap(), &Options::new()) {
            Ok(digest) if digest == expected => {}
            other => wrong.push(format!("{}: {other:?}", dir.file_name().unwrap().to_string_lossy())),
        }
    }
    assert!(wrong.is_empty(), "wrong:\n{}", wrong.join("\n"));
}
