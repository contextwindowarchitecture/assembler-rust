#!/usr/bin/env python3
"""Name a new port made from the template: fill the language, package and commands into its files.

    python3 scripts/init_port.py --language Go --package github.com/contextwindowarchitecture/assembler-go \
        --repository contextwindowarchitecture/assembler-go \
        --install "go mod download" --build "go build ./..." --test "go test ./..." --conformance "go run ./cmd/conformance"

Run it once, from the port's root, right after creating the repository from the template (PORTING.md, step 1). It:

  - fills <Language> and the command placeholders into AGENTS.md and the commented test job in .github/workflows/ci.yml;
  - names the package and language in NOTICE;
  - replaces the template's README.md with the port's starter README (the outline at the end of PORTING.md);
  - fills the language and package into PORTING.md, so its commands can be pasted as they are;
  - adds the GitHub remote as origin when --repository is given and the checkout has no origin yet.

A command left out stays a placeholder to fill by hand. It refuses to run twice. Standard library only.
"""
from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

README = """# {package}

A {language} assembler for the [Context Window Architecture](https://contextwindowarchitecture.io) (CWA) draft specification. It admits candidate items, resolves declared conflicts, fits them to a token budget, renders the payload and emits the trace.

Status: in development. It passes none of the published conformance cases yet; `conformance-report.json` records the current run.

## Install

TODO: how to add the package to a project.

## Use

TODO: the call and its types, in the language's terms. What it must say:

- `assemble(snapshot)` takes a snapshot in the shape of `schema/snapshot.schema.json`, the frozen assembly input (R-23), and returns the payload, the rendered UTF-8 bytes, or null when the assembly is refused (`trace.refused.reason` says why), together with a trace valid against `schema/trace.schema.json`.
- A snapshot that fails its schemas or the snapshot checks is rejected with its problems in words: no payload and no trace (R-17).
- A snapshot that names a tokenizer or renderer this package does not provide is unsupported, not invalid. The package provides the tokenizers `fixture-whitespace/v1` and `estimate-utf8/v1` and the renderers `fixture-xml/v1` and `cwa-messages/v1`; callers pass their model's tokenizer, and a renderer if the package takes any, under an id no published component of that kind uses; a published id, even one this package does not provide, stops the call before assembly with no payload and no trace (R-16).
- `trace_id` and `timings` may differ between runs of the same snapshot (R-23); everything else, the payload bytes included, is deterministic.

## Requirements

TODO: the runtime versions this package supports.

## Development

```sh
{install}
{build}
{test}
```

## Cost

Every reduction under budget pressure is its own fit test, and every fit test renders and counts the whole payload (conformance/README.md, Fitting). Keep the cost down at the source, as the spec advises: send no more passages than the route's budget can use, and bound slots with `max_per_source` or `max_tokens`.

## Conformance

```sh
{conformance}
```

This runs every vendored case and rejection snapshot as `conformance/README.md` describes. It writes `conformance-report.json`, valid against `schema/conformance_report.schema.json`, and exits 1 unless every case passed and every rejection snapshot was rejected, apart from those skipped for an optional component. A case passes only when its payload matches byte for byte and its trace matches field for field, except `trace_id` and `timings`. The committed report is the current run: a test fails when it goes stale. A case is skipped only when it uses a tokenizer or renderer the vendored README lists under Optional, such as `cwa-message-blocks/v1`, and this package does not provide it; a case that uses only required ones and does not pass has failed.

## The contract

`vendor/cwa/` holds the published contract this implementation follows: the schemas, the contract data and the conformance cases, copied from the website repository. `vendor/cwa.lock.json` pins each file by SHA-256 and records the website commit. It is Apache-2.0 licensed; see `vendor/cwa/LICENSE` and `vendor/cwa/NOTICE`.

See [AGENTS.md](AGENTS.md) for the working rules.

## License

Apache License 2.0, the same as the specification: see [LICENSE](LICENSE) and [NOTICE](NOTICE).
"""

NOTICE = """{package}, a {language} assembler for the Context Window Architecture draft specification
Copyright 2026 Melvin Hillsman

This product is licensed under the Apache License, Version 2.0 (see LICENSE).

vendor/cwa/ holds the published CWA JSON Schemas, contract data and conformance cases this implementation
follows. They come from the Context Window Architecture specification and website, Copyright 2026 Melvin
Hillsman, also licensed under the Apache License, Version 2.0; see vendor/cwa/LICENSE and vendor/cwa/NOTICE.
"""

# The paragraph in AGENTS.md that points at this script, and what it becomes once the port is named.
FILL_IN = ("`python3 scripts/init_port.py` fills the `<placeholders>` in this file, README.md, NOTICE and the CI workflow (PORTING.md, step 1); "
           "fill any it left by hand before the first commit. Everything else here")
FILLED = "Everything here"
LEFT = "Fill the remaining `<placeholders>` in this file, README.md and the CI workflow by hand before the first commit. Everything else here"


def replace(path: Path, substitutions: dict[str, str]) -> int:
    """Apply the substitutions to a file; the number of placeholders replaced."""
    text = original = path.read_text(encoding="utf-8")
    for old, new in substitutions.items():
        text = text.replace(old, new)
    if text != original:
        path.write_text(text, encoding="utf-8")
    return sum(original.count(old) for old in substitutions)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--language", required=True, help="the language, as the website names it, e.g. Go or TypeScript")
    parser.add_argument("--package", required=True, help="the package name in the language's ecosystem")
    parser.add_argument("--slug", help="the lower-case language name in repository names; default: the language lower-cased")
    parser.add_argument("--repository", help="the GitHub repository as owner/name, e.g. contextwindowarchitecture/assembler-go")
    parser.add_argument("--install", help="the command that installs the toolchain and dependencies")
    parser.add_argument("--build", help="the command that builds, if the language needs one")
    parser.add_argument("--test", help="the command that runs the full suite")
    parser.add_argument("--conformance", help="the command that writes conformance-report.json")
    args = parser.parse_args()

    agents = ROOT / "AGENTS.md"
    if "<Language>" not in agents.read_text(encoding="utf-8"):
        sys.exit("AGENTS.md no longer has the <Language> placeholder: this port is already named")
    slug = args.slug or re.sub(r"[^a-z0-9]+", "-", args.language.lower()).strip("-")
    commands = {"<install>": args.install, "<build>": args.build, "<test>": args.test, "<conformance>": args.conformance}
    given = {old: new for old, new in commands.items() if new}

    left = [old for old, new in commands.items() if not new]
    replaced = replace(agents, {"<Language>": args.language, **{f"`{old}`": f"`{new}`" for old, new in given.items()},
                                FILL_IN: LEFT if left else FILLED})
    replaced += replace(ROOT / ".github" / "workflows" / "ci.yml", {"<runtime>": args.language, **given})
    replaced += replace(ROOT / "PORTING.md", {"<Language>": args.language, "<language>": slug, "<package>": args.package})
    (ROOT / "NOTICE").write_text(NOTICE.format(package=args.package, language=args.language), encoding="utf-8")
    (ROOT / "README.md").write_text(README.format(package=args.package, language=args.language,
                                                  **{key.strip("<>"): value or key for key, value in commands.items()}), encoding="utf-8")
    print(f"named the port {args.package} ({args.language}); {replaced} placeholders filled in AGENTS.md, ci.yml and PORTING.md; NOTICE and README.md written")
    if left:
        print(f"still to fill by hand in AGENTS.md, README.md and ci.yml: {', '.join(left)}")

    if args.repository:
        url = f"https://github.com/{args.repository}.git"
        git = lambda *a: subprocess.run(["git", "-C", str(ROOT), *a], capture_output=True, text=True)
        if git("rev-parse", "--git-dir").returncode != 0:
            print(f"not a git repository yet: git init -b main, then git remote add origin {url}")
        elif git("remote", "get-url", "origin").returncode == 0:
            print(f"origin already set: {git('remote', 'get-url', 'origin').stdout.strip()}")
        elif git("remote", "add", "origin", url).returncode == 0:
            print(f"added remote origin {url} (never push; the maintainer publishes)")
    print(f"next: review the changes, add the language's ignores and manifest, then: git commit -s -m 'chore: start the {slug} assembler from the template'")
    return 0


if __name__ == "__main__":
    sys.exit(main())
