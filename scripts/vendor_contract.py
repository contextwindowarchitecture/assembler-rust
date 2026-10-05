#!/usr/bin/env python3
"""Copy the published CWA contract from a website checkout into vendor/cwa/, pinned by SHA-256.

    python3 scripts/vendor_contract.py --website ../website           # vendor, and rewrite vendor/cwa.lock.json
    python3 scripts/vendor_contract.py --website ../website --check   # exit 1 when vendor/cwa/ drifts from the website
    python3 scripts/vendor_contract.py --verify                       # exit 1 when vendor/cwa/ disagrees with its lock

The lock records every vendored file's SHA-256, the website commit it came from, and whether the vendored sources
had uncommitted changes there. Vendor from a committed website state: the website counts a report against a
dirty commit as stale. The port's own suite re-checks the lock natively (PORTING.md, step 3); --verify is the
same check for CI and for a port that has no suite yet. Standard library only.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
VENDOR = ROOT / "vendor" / "cwa"
LOCK = ROOT / "vendor" / "cwa.lock.json"
# The contract an assembler needs: the license, the schemas, the contract data, the conformance README, the
# cases and rejection cases, and the registry whose lock lets a port test its RFC 8785 digests.
SOURCES = (
    "LICENSE",
    "NOTICE",
    "schema",
    "contract/requirements.json",
    "contract/reasons.json",
    "contract/slot-defaults.json",
    "conformance/README.md",
    "conformance/cases",
    "conformance/rejections",
    "conformance/registry",
)


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def website_files(website: Path) -> dict[str, Path]:
    """Every file to vendor, keyed by its path relative to the website checkout, with / separators."""
    files: dict[str, Path] = {}
    for source in SOURCES:
        root = website / source
        if not root.exists():
            sys.exit(f"{root} does not exist; is {website} a checkout of the website repository?")
        paths = [root] if root.is_file() else sorted(p for p in root.rglob("*") if p.is_file())
        for path in paths:
            files[path.relative_to(website).as_posix()] = path
    return files


def git(website: Path, *args: str) -> str:
    return subprocess.run(["git", "-C", str(website), *args], capture_output=True, text=True, check=True).stdout.strip()


def vendored_files() -> dict[str, str]:
    """Every file under vendor/cwa/ with its SHA-256, keyed by its vendored path."""
    if not VENDOR.exists():
        return {}
    return {p.relative_to(VENDOR).as_posix(): sha256(p) for p in sorted(VENDOR.rglob("*")) if p.is_file()}


def report(problems: list[str]) -> int:
    for problem in problems:
        print(problem, file=sys.stderr)
    return 1 if problems else 0


def vendor(website: Path) -> int:
    files = website_files(website)
    if VENDOR.exists():
        shutil.rmtree(VENDOR)
    for rel, src in files.items():
        dest = VENDOR / rel
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(src, dest)
    lock = {
        "website_commit": git(website, "rev-parse", "HEAD"),
        "dirty": git(website, "status", "--porcelain", "--", *SOURCES) != "",
        "files": {rel: sha256(VENDOR / rel) for rel in sorted(files)},
    }
    LOCK.write_text(json.dumps(lock, indent=2) + "\n")
    print(f"vendored {len(files)} files from website {lock['website_commit'][:7]}{' (dirty)' if lock['dirty'] else ''}")
    return 0


def check(website: Path) -> int:
    """Drift between vendor/cwa/ and the website checkout, file for file."""
    files = website_files(website)
    have = vendored_files()
    problems = [f"missing: {rel}" for rel in files if rel not in have]
    problems += [f"extra: {rel}" for rel in have if rel not in files]
    problems += [f"differs: {rel}" for rel, src in files.items() if rel in have and have[rel] != sha256(src)]
    if not problems:
        print(f"vendor/cwa matches {website}: {len(have)} files")
    return report(problems)


def verify() -> int:
    """Disagreement between vendor/cwa/ and vendor/cwa.lock.json."""
    have = vendored_files()
    if not LOCK.exists():
        if not have:
            print("nothing vendored yet: run vendor_contract.py --website <checkout> (PORTING.md, step 2)")
            return 0
        return report([f"{LOCK} does not exist but vendor/cwa/ holds {len(have)} files"])
    lock = json.loads(LOCK.read_text())
    problems = [f"missing: {rel}" for rel in lock["files"] if rel not in have]
    problems += [f"unlocked: {rel}" for rel in have if rel not in lock["files"]]
    problems += [f"hash differs: {rel}" for rel, digest in lock["files"].items() if rel in have and have[rel] != digest]
    if not problems:
        print(f"vendor/cwa matches its lock: {len(have)} files at website {lock['website_commit'][:7]}{' (dirty)' if lock['dirty'] else ''}")
    return report(problems)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--website", type=Path, help="a checkout of the website repository")
    parser.add_argument("--check", action="store_true", help="with --website: report drift instead of vendoring")
    parser.add_argument("--verify", action="store_true", help="check vendor/cwa/ against its lock; needs no website")
    args = parser.parse_args()
    if args.verify:
        return verify()
    if args.website is None:
        parser.error("--website is required unless --verify is given")
    website = args.website.resolve()
    return check(website) if args.check else vendor(website)


if __name__ == "__main__":
    sys.exit(main())
