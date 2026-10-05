#!/usr/bin/env python3
"""Check a conformance report against the vendored contract, whichever runner wrote it.

    python3 scripts/check_report.py                     # conformance-report.json: exit 1 on any problem or any failure
    python3 scripts/check_report.py --allow-failures    # exit 1 on structural problems only, while cases are pending

Checks that the report validates against conformance_report.schema.json (when the jsonschema package is
installed); that its contract names the lock's website commit and dirty flag; that cases and rejections list
every directory under the vendored conformance cases and rejections, in id order, with the rules from each
case.json; that every skipped row's detail names, as "<kind> <id> is not provided", an optional component its case
uses, since a port that lacks only required ones has failed the case; and, unless --allow-failures, that every case passed and
every rejection snapshot was rejected, apart from those skipped for an optional component the port leaves out
(conformance/README.md, Reporting results). CI runs it so a partial or stale report never lands.
"""
from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from conformance import LOCK, ROOT, ids, optional_lacking, read_json, required_components, schema_problem, utf16, validator_for  # noqa: E402


def check_rows(report: dict, kind: str, directory: Path, ok: str, required: set[str]) -> tuple[list[str], list[str]]:
    """Structural problems with one list of the report, and the rows that neither reached the counting outcome
    nor were skipped. A skip whose detail names no optional component its case uses is a problem, so any other skip
    is for an optional component, which a port may leave out."""
    rows = report.get(kind)
    if rows is None:
        note = ", so no rejection case counts (Reporting results)" if kind == "rejections" else ""
        return [f"the report has no {kind} list{note}"], []
    expected = ids(directory)
    got = [row.get("id") for row in rows]
    problems = [f"{kind}: {id} is not in the report" for id in expected if id not in got]
    problems += [f"{kind}: {id} is not a directory under {directory}" for id in got if id not in expected]
    if not problems and got != expected:
        problems.append(f"{kind} are not in id order (UTF-16 code units)")
    for row in rows:
        if row.get("id") in expected and row.get("rules") != read_json(directory / row["id"] / "case.json")["rules"]:
            problems.append(f"{kind}: {row['id']} does not carry the rules from its case.json")
        if row.get("id") in expected and row.get("outcome") == "skipped":
            snapshot = read_json(directory / row["id"] / "snapshot.json")
            fields = ("tokenizer", "renderer") if kind == "cases" else ("renderer",)
            if not optional_lacking(snapshot, fields, required, row.get("detail", "")):
                problems.append(f"{kind}: {row['id']} is skipped, but its detail names no optional component it uses as "
                                "\"<kind> <id> is not provided\", so it has failed")
    failures = [f"{row.get('id')}: {row.get('outcome')}: {row.get('detail', '')}".rstrip(": ") for row in rows if row.get("outcome") not in (ok, "skipped")]
    return problems, failures


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("report", nargs="?", type=Path, default=ROOT / "conformance-report.json")
    parser.add_argument("--conformance", type=Path, default=ROOT / "vendor" / "cwa" / "conformance", help="the cases directory")
    parser.add_argument("--allow-failures", action="store_true", help="exit 0 although some cases did not pass")
    args = parser.parse_args()

    if not LOCK.exists():
        print("nothing vendored yet, so there is no report to check (PORTING.md, step 2)")
        return 0
    if not args.report.exists():
        print(f"{args.report} does not exist; run the conformance command and commit its report", file=sys.stderr)
        return 1
    report = read_json(args.report)
    lock = read_json(LOCK)
    problems: list[str] = []
    validator = validator_for("conformance_report.schema.json")
    if validator is None:
        print("note: jsonschema is not installed, so the report is not validated against conformance_report.schema.json", file=sys.stderr)
    problem = schema_problem(validator, report)
    if problem:
        problems.append(f"the report fails conformance_report.schema.json at {problem}")
    contract = {"website_commit": lock["website_commit"], "dirty": lock["dirty"]}
    if report.get("contract") != contract:
        problems.append(f"contract is {json.dumps(report.get('contract'))}, but the lock says {json.dumps(contract)}: rerun the conformance command")
    failures: list[str] = []
    for kind, ok in (("cases", "passed"), ("rejections", "rejected")):
        found, failed = check_rows(report, kind, args.conformance / kind, ok, required_components(args.conformance))
        problems += found
        failures += failed

    for problem in problems:
        print(problem, file=sys.stderr)
    cases, rejections = report.get("cases") or [], report.get("rejections") or []
    skipped = sum(r.get("outcome") == "skipped" for r in cases + rejections)
    print(f"{args.report}: {sum(r.get('outcome') == 'passed' for r in cases)}/{len(cases)} cases passed, "
          f"{sum(r.get('outcome') == 'rejected' for r in rejections)}/{len(rejections)} rejection snapshots rejected"
          + (f", {skipped} skipped" if skipped else ""))
    for failure in failures:
        print(f"  {failure}")
    if problems:
        return 1
    return 0 if args.allow_failures or not failures else 1


if __name__ == "__main__":
    sys.exit(main())
