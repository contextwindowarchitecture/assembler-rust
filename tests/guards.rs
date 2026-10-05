//! The guard tests (PORTING.md, step 3): the vendored contract matches its lock, the license is the
//! specification's, the implementation is named as the manifest names it, and the committed report is the
//! current run.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use cwa_assembler::canonical::sha256_hex;
use cwa_assembler::conformance::report;
use serde_json::Value;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn json(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))).unwrap()
}

fn files_under(dir: &Path, base: &Path, out: &mut BTreeSet<String>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files_under(&path, base, out);
        } else {
            out.insert(path.strip_prefix(base).unwrap().to_string_lossy().replace('\\', "/"));
        }
    }
}

#[test]
fn the_vendored_contract_matches_its_lock() {
    let lock = json(&root().join("vendor/cwa.lock.json"));
    let vendored = root().join("vendor/cwa");
    let locked = lock["files"].as_object().unwrap();
    let mut present = BTreeSet::new();
    files_under(&vendored, &vendored, &mut present);
    let expected: BTreeSet<String> = locked.keys().cloned().collect();
    assert_eq!(present, expected, "vendor/cwa/ holds other files than its lock lists");
    for (path, sha) in locked {
        assert_eq!(sha256_hex(&fs::read(vendored.join(path)).unwrap()), sha.as_str().unwrap(), "{path} does not match its lock");
    }
}

#[test]
fn the_license_is_the_specifications() {
    assert_eq!(fs::read(root().join("LICENSE")).unwrap(), fs::read(root().join("vendor/cwa/LICENSE")).unwrap());
    assert!(fs::read_to_string(root().join("NOTICE")).unwrap().contains("Apache License, Version 2.0"));
}

#[test]
fn the_implementation_is_named_as_the_manifest_names_it() {
    assert_eq!(cwa_assembler::IMPLEMENTATION_NAME, env!("CARGO_PKG_NAME"));
    assert_eq!(cwa_assembler::IMPLEMENTATION_VERSION, env!("CARGO_PKG_VERSION"));
    assert_eq!(cwa_assembler::IMPLEMENTATION_LANGUAGE, "Rust");
}

#[test]
fn the_committed_report_is_the_current_run() {
    let committed = json(&root().join("conformance-report.json"));
    let fresh = report(&root().join("vendor/cwa/conformance"), &json(&root().join("vendor/cwa.lock.json")));
    assert!(committed == fresh, "conformance-report.json is stale; run `cargo run --release --bin cwa-conformance`");
}

#[test]
fn the_report_is_valid_against_its_schema() {
    let committed = json(&root().join("conformance-report.json"));
    let errors = cwa_assembler::schema::validate("conformance_report.schema.json", &committed);
    assert!(errors.is_empty(), "{errors:?}");
}
