#!/usr/bin/env python3
"""Run the vendored conformance cases against an assembler in any language, and write conformance-report.json.

    python3 scripts/conformance.py --command "<adapter>" --name <package> --version <version> --language <Language>

The adapter command is started once per snapshot with the snapshot file's bytes on stdin. It answers by exit code:

    0  assembled, refusals included: stdout holds {"payload": <base64 of the payload bytes, or null when refused>,
       "trace": <the trace>}
    2  rejected before assembly (R-17): stderr lists the problems in the port's words; stdout is not read
    3  a tokenizer or renderer the port does not provide: stderr names each one it lacks on a line of its own, as
       "tokenizer <id> is not provided" or "renderer <id> is not provided". The case is skipped only when one it
       names is optional, a component the vendored README does not require, that the case uses; otherwise, a
       required one lacking or none named, it has failed (conformance/README.md, Reporting results)

Any other exit code, or stdout that is not such an object, fails the case with stderr as the detail. The runner
compares as conformance/README.md, Running a case, says: the payload byte for byte, the trace field for field
without trace_id, timings and recovery.detail, and it validates each trace against trace.schema.json when the jsonschema package
is installed. It writes the report in the shape of conformance_report.schema.json (Reporting results), prints a
summary, and exits 1 unless every case passed and every rejection snapshot was rejected, apart from those skipped
for an optional component the port leaves out.

This is a bootstrap tool: it gets a port a report before it has a runner of its own. A port may keep it or
replace it with a native runner that does the same; scripts/check_report.py checks either's report.
"""
from __future__ import annotations

import argparse
import base64
import json
import re
import shlex
import subprocess
import sys
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parent.parent
LOCK = ROOT / "vendor" / "cwa.lock.json"
SCHEMAS = ROOT / "vendor" / "cwa" / "schema"
MISSING = object()


def utf16(s: str) -> bytes:
    """The sort key the README uses for ids and member names: UTF-16 code units (Ordering)."""
    return s.encode("utf-16-be")


def read_json(path: Path) -> Any:
    return json.loads(path.read_text(encoding="utf-8"))


def report_contract(lock: dict) -> dict:
    """The report's contract member, from the lock: the specification repository and commit the vendored cases came
    from, and the lock's dirty flag."""
    return {"repository": lock["repository"], "commit": lock["spec_commit"], "dirty": lock["dirty"]}


def validator_for(name: str) -> Any:
    """A jsonschema validator for one published schema, resolving $refs among the vendored ones; None without jsonschema."""
    try:
        import jsonschema
    except ImportError:
        return None
    schemas = [read_json(p) for p in sorted(SCHEMAS.glob("*.schema.json"))]
    schema = next(s for s in schemas if s["$id"].endswith("/" + name))
    try:
        from referencing import Registry, Resource

        registry = Registry().with_resources((s["$id"], Resource.from_contents(s)) for s in schemas)
        return jsonschema.Draft202012Validator(schema, registry=registry)
    except ImportError:  # jsonschema before 4.18
        resolver = jsonschema.RefResolver.from_schema(schema, store={s["$id"]: s for s in schemas})
        return jsonschema.Draft202012Validator(schema, resolver=resolver)


def schema_problem(validator: Any, instance: Any) -> str | None:
    """The most relevant schema error for an instance, or None when it validates or nothing can validate it."""
    if validator is None:
        return None
    from jsonschema.exceptions import best_match

    errors = list(validator.iter_errors(instance))
    if not errors:
        return None
    error = best_match(errors)
    where = "/" + "/".join(str(p) for p in error.absolute_path) if error.absolute_path else "/"
    return f"{where}: {error.message}"


def same_kind(a: Any, b: Any) -> bool:
    """JSON kinds: true is not 1, but 12 and 12.0 are the same number."""
    if isinstance(a, bool) or isinstance(b, bool):
        return isinstance(a, bool) and isinstance(b, bool)
    if isinstance(a, (int, float)) and isinstance(b, (int, float)):
        return True
    return type(a) is type(b)


def pointer(key: str) -> str:
    return key.replace("~", "~0").replace("/", "~1")


def first_difference(expected: Any, actual: Any, path: str = "") -> str | None:
    """The JSON Pointer of the first place two JSON values differ, with both values; None when they are equal."""

    def show(value: Any) -> str:
        return "nothing" if value is MISSING else json.dumps(value, ensure_ascii=False)

    if isinstance(expected, list) and isinstance(actual, list):
        for i in range(max(len(expected), len(actual))):
            e = expected[i] if i < len(expected) else MISSING
            a = actual[i] if i < len(actual) else MISSING
            found = first_difference(e, a, f"{path}/{i}")
            if found:
                return found
        return None
    if isinstance(expected, dict) and isinstance(actual, dict):
        for key in sorted(set(expected) | set(actual), key=utf16):
            found = first_difference(expected.get(key, MISSING), actual.get(key, MISSING), f"{path}/{pointer(key)}")
            if found:
                return found
        return None
    if expected is not MISSING and actual is not MISSING and same_kind(expected, actual) and expected == actual:
        return None
    return f"{path or '/'}: expected {show(expected)}, got {show(actual)}"


def comparable(trace: Any) -> Any:
    """Trace ids, timings and recovery.detail may differ (R-23); every other member is compared."""
    if not isinstance(trace, dict):
        return trace
    kept = {k: v for k, v in trace.items() if k not in ("trace_id", "timings")}
    if isinstance(kept.get("recovery"), dict):
        kept["recovery"] = {k: v for k, v in kept["recovery"].items() if k != "detail"}
    return kept


class Adapter:
    def __init__(self, command: list[str], timeout: float):
        self.command = command
        self.timeout = timeout

    def run(self, snapshot: bytes) -> tuple[int | None, bytes, str]:
        """Exit code (None on timeout), stdout bytes, stderr text."""
        try:
            proc = subprocess.run(self.command, input=snapshot, capture_output=True, timeout=self.timeout)
        except subprocess.TimeoutExpired:
            return None, b"", f"timed out after {self.timeout:g} s"
        except OSError as error:
            return 127, b"", f"could not start {shlex.join(self.command)}: {error}"
        return proc.returncode, proc.stdout, proc.stderr.decode("utf-8", "replace").strip()


def parse_result(stdout: bytes) -> tuple[Any, str | None]:
    """The adapter's {"payload", "trace"} object, or a detail saying why it is unusable."""
    try:
        result = json.loads(stdout.decode("utf-8"))
    except (UnicodeDecodeError, ValueError):
        return None, "stdout is not a JSON object with payload and trace"
    if not isinstance(result, dict) or "payload" not in result or "trace" not in result:
        return None, "stdout is not a JSON object with payload and trace"
    if result["payload"] is not None:
        try:
            result["payload"] = base64.b64decode(result["payload"], validate=True)
        except (TypeError, ValueError):
            return None, "payload is not base64 or null"
    return result, None


def refusal(trace: Any) -> str:
    refused = trace.get("refused") if isinstance(trace, dict) else None
    reason = refused.get("reason") if isinstance(refused, dict) else None
    return f"refused with {reason}" if reason else "refused"


def required_components(conformance: Path) -> set[str]:
    """The tokenizers and renderers every implementation provides: the bullets under the vendored README's
    Tokenizers and renderers heading, which opens "Every implementation provides the tokenizers and renderers below,
    before Optional". The section stops at the next heading of any level, so a component listed under its Optional
    subheading is published but not required."""
    text = (conformance / "README.md").read_text(encoding="utf-8")
    if "\n## Tokenizers and renderers\n" not in text:
        sys.exit(f"{conformance / 'README.md'} has no Tokenizers and renderers section; re-vendor the contract")
    section = re.split(r"^#+ ", text.split("\n## Tokenizers and renderers\n", 1)[1], maxsplit=1, flags=re.M)[0]
    return set(re.findall(r"^- `([^`]+)`", section, re.M))


LACKING = re.compile(r"^(tokenizer|renderer) (\S+) is not provided$", re.M)


def optional_lacking(snapshot: Any, fields: tuple[str, ...], required: set[str], text: str) -> list[str]:
    """The optional components text says the port lacks, one "<kind> <id> is not provided" line each, that the
    snapshot uses in these fields: an ID outside the required set. Reporting results skips a case for one of these
    alone; a component the case does not use, or a required one, never makes a skip, whatever else the snapshot names."""
    named = {field: snapshot.get(field) for field in fields} if isinstance(snapshot, dict) else {}
    return [f"{kind} {id}" for kind, id in LACKING.findall(text or "") if named.get(kind) == id and id not in required]


def unsupported(case: Path, fields: tuple[str, ...], required: set[str], stderr: str) -> dict[str, str]:
    """Exit 3. Reporting results skips a case only for an optional component the port lacks, whatever else it lacks;
    a case the port lacks only required components for, or that names nothing it lacks, has failed. A rejection
    looks at its renderer alone: no snapshot check needs a tokenizer."""
    try:
        snapshot = read_json(case / "snapshot.json")
    except ValueError:
        snapshot = None
    optional = optional_lacking(snapshot, fields, required, stderr)
    if optional:
        return {"outcome": "skipped", "detail": "\n".join(f"{component} is not provided" for component in optional)}
    if not LACKING.search(stderr or ""):
        return {"outcome": "failed", "detail": f"exited 3 without naming a component it lacks: {stderr}".rstrip(": ")}
    return {"outcome": "failed", "detail": f"a required component is not provided: {stderr}"}


def run_case(adapter: Adapter, trace_validator: Any, case: Path, required: set[str]) -> dict[str, str]:
    code, stdout, stderr = adapter.run((case / "snapshot.json").read_bytes())
    if code == 3:
        return unsupported(case, ("tokenizer", "renderer"), required, stderr)
    if code == 2:
        return {"outcome": "failed", "detail": f"rejected the snapshot where the case expects an assembly: {stderr}"}
    if code != 0:
        return {"outcome": "failed", "detail": f"exited {code}: {stderr}" if code is not None else stderr}
    result, problem = parse_result(stdout)
    if problem:
        return {"outcome": "failed", "detail": problem}
    trace, payload = result["trace"], result["payload"]
    problem = schema_problem(trace_validator, trace)
    if problem:
        return {"outcome": "failed", "detail": f"the trace fails trace.schema.json at {problem}"}
    payload_path = case / "expected.payload.txt"
    expected_payload = payload_path.read_bytes() if payload_path.exists() else None
    if expected_payload is None and payload is not None:
        return {"outcome": "failed", "detail": "rendered a payload where the case expects a refusal"}
    if expected_payload is not None and payload is None:
        return {"outcome": "failed", "detail": f"{refusal(trace)} where the case expects a payload"}
    if expected_payload is not None and payload != expected_payload:
        return {"outcome": "failed", "detail": "the payload bytes differ from expected.payload.txt"}
    difference = first_difference(comparable(read_json(case / "expected.trace.json")), comparable(trace))
    if difference:
        return {"outcome": "failed", "detail": f"the trace differs at {difference}"}
    return {"outcome": "passed"}


def run_rejection(adapter: Adapter, case: Path, required: set[str]) -> dict[str, str]:
    code, stdout, stderr = adapter.run((case / "snapshot.json").read_bytes())
    if code == 2:
        return {"outcome": "rejected"}
    if code == 3:
        return unsupported(case, ("renderer",), required, stderr)
    if code == 0:
        result, problem = parse_result(stdout)
        if problem:
            return {"outcome": "failed", "detail": f"exited 0 instead of rejecting, and {problem}"}
        if result["payload"] is None:
            return {"outcome": "failed", "detail": f"{refusal(result['trace'])} instead of rejecting"}
        return {"outcome": "failed", "detail": "assembled a payload instead of rejecting"}
    return {"outcome": "failed", "detail": f"exited {code}: {stderr}" if code is not None else stderr}


def ids(directory: Path) -> list[str]:
    return sorted((p.name for p in directory.iterdir() if p.is_dir()), key=utf16) if directory.exists() else []


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--command", required=True, help="the adapter command, started once per snapshot")
    parser.add_argument("--name", required=True, help="the implementation's package name, as the report names it")
    parser.add_argument("--version", required=True, help="the implementation's version")
    parser.add_argument("--language", help="the implementation's language, e.g. Go")
    parser.add_argument("--conformance", type=Path, default=ROOT / "vendor" / "cwa" / "conformance", help="the cases directory")
    parser.add_argument("--out", type=Path, default=ROOT / "conformance-report.json", help="where to write the report")
    parser.add_argument("--timeout", type=float, default=60, help="seconds allowed per snapshot")
    args = parser.parse_args()

    if not LOCK.exists():
        sys.exit("vendor/cwa.lock.json does not exist; run scripts/vendor_contract.py --spec <checkout> first")
    lock = read_json(LOCK)
    adapter = Adapter(shlex.split(args.command), args.timeout)
    trace_validator = validator_for("trace.schema.json")
    if trace_validator is None:
        print("note: jsonschema is not installed, so traces are compared but not validated against trace.schema.json", file=sys.stderr)

    implementation = {"name": args.name, "version": args.version}
    if args.language:
        implementation["language"] = args.language
    cases_dir, rejections_dir = args.conformance / "cases", args.conformance / "rejections"
    required = required_components(args.conformance)
    report = {
        "implementation": implementation,
        "contract": report_contract(lock),
        "cases": [{"id": id, "rules": read_json(cases_dir / id / "case.json")["rules"], **run_case(adapter, trace_validator, cases_dir / id, required)}
                  for id in ids(cases_dir)],
        "rejections": [{"id": id, "rules": read_json(rejections_dir / id / "case.json")["rules"], **run_rejection(adapter, rejections_dir / id, required)}
                       for id in ids(rejections_dir)],
    }
    args.out.write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")

    cases, rejections = report["cases"], report["rejections"]
    passed = sum(row["outcome"] == "passed" for row in cases)
    rejected = sum(row["outcome"] == "rejected" for row in rejections)
    skipped = sum(row["outcome"] == "skipped" for row in cases + rejections)
    print(f"{args.out}: {passed}/{len(cases)} cases passed, {rejected}/{len(rejections)} rejection snapshots rejected"
          + (f", {skipped} skipped for an optional component" if skipped else ""))
    for row in cases + rejections:
        if "detail" in row:
            print(f"  {row['id']}: {row['outcome']}: {row['detail']}")
    # unsupported() skips only for an optional component, which a port may leave out, so a skip is not a failure.
    return 0 if passed + rejected + skipped == len(cases) + len(rejections) else 1


if __name__ == "__main__":
    sys.exit(main())
