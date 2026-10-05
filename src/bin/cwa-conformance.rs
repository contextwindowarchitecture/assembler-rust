//! Runs every vendored conformance case and rejection snapshot and writes `conformance-report.json`
//! (conformance/README.md, Reporting results). Exits 1 unless every case passed and every rejection snapshot was
//! rejected, apart from those skipped for an optional component this package leaves out.
//!
//!     cargo run --release --bin cwa-conformance [-- --out <path>]

use std::path::PathBuf;
use std::process::ExitCode;

use contextwindowarchitecture_assembler::conformance::{report, report_text};

fn main() -> ExitCode {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut out = root.join("conformance-report.json");
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match (arg.as_str(), args.next()) {
            ("--out", Some(path)) => out = PathBuf::from(path),
            _ => {
                eprintln!("usage: cwa-conformance [--out <path>]");
                return ExitCode::from(2);
            }
        }
    }
    let lock: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("vendor/cwa.lock.json")).expect("vendor/cwa.lock.json"))
        .expect("the lock is JSON");
    let report = report(&root.join("vendor/cwa/conformance"), &lock);
    std::fs::write(&out, report_text(&report)).expect("the report is writable");

    let count = |list: &str, outcome: &str| report[list].as_array().unwrap().iter().filter(|e| e["outcome"] == outcome).count();
    let (cases, rejections) = (report["cases"].as_array().unwrap(), report["rejections"].as_array().unwrap());
    let skipped = count("cases", "skipped") + count("rejections", "skipped");
    println!("{}: {}/{} cases passed, {}/{} rejection snapshots rejected{}", out.display(), count("cases", "passed"), cases.len(),
             count("rejections", "rejected"), rejections.len(), if skipped > 0 { format!(", {skipped} skipped for an optional component") } else { String::new() });
    for entry in cases.iter().chain(rejections) {
        if let Some(detail) = entry["detail"].as_str() {
            println!("  {}: {}: {detail}", entry["id"].as_str().unwrap(), entry["outcome"].as_str().unwrap());
        }
    }
    let good = count("cases", "passed") + count("rejections", "rejected") + skipped;
    if good == cases.len() + rejections.len() { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}
