# Working in this repository

A Rust implementation of the CWA draft specification, started from the assembler template (PORTING.md). Its target is every published conformance case: each case's payload byte for byte and its trace, plus every rejection case rejected. `conformance-report.json` records the result, and the website imports it beside the other implementations' reports.

Everything here is the rule set the Python reference assembler and the TypeScript assembler work under. It is not optional.

## Rules

- **Test-first.** Red, then green, then refactor. Write the failing test, watch it fail for the reason you expect, then make it pass with the least code. A test that passes before the change, or fails on an import error, proves nothing.
- **Commit unasked at each green step.** One behavior, or one refactor, per commit. Don't wait to be asked.
- **Conventional Commits, signed off.** `git commit -s` with a Conventional Commits subject (`feat:`, `fix:`, `test:`, `docs:`, `chore:`, `refactor:`, `build:`, `ci:`).
- **Never push.** The maintainer publishes commits. Don't open pull requests or fetch-and-rebase either.
- **Docs change in the same commit as the behavior.** A commit that changes behavior also updates README.md, this file, or the code comments that describe it. Never batch doc updates at the end.
- **Prove a test protects something.** Before claiming it does, break the code on purpose and watch the test fail. Restore the code from a copy made beforehand, never with `git checkout` or `git restore`, which also discard uncommitted work; then clear any bytecode or build cache that could outlive the edit.

## Working from the spec

- Work from the published spec only: the contract vendored in `vendor/cwa/`. Read `conformance/README.md` first, then `contract/requirements.json`, then the schemas. Don't read the other implementations (the Python reference assembler, the TypeScript assembler, or any other port) and don't take their code. A port built independently is what makes it worth having: two ports that disagree while both pass every case expose a gap in the spec, and that has already happened twice.
- Where the spec leaves a behavior open, don't decide it here. Ask the maintainer. The fix goes into the website repository first, with its tests and a case that pins it, then re-vendor, then implement.
- `vendor/cwa.lock.json` pins every vendored file by SHA-256, with the website commit it came from and whether the vendored sources were dirty there. Change vendored files only with `python3 scripts/vendor_contract.py --website <checkout>`, from a committed website state. A test in this repository fails when a vendored file no longer matches its hash, or when one is added or missing (PORTING.md, step 3).
- Anything generated from the vendored contract (types, embedded schemas, embedded reason codes or slot defaults) lives in one generated directory, is regenerated after every re-vendor, and is never edited by hand. A test fails when it is stale.
- `conformance-report.json` is committed and must be the current run. Rerun the conformance command after any change to the assembler or the vendored contract, and commit the report with the change. `python3 scripts/check_report.py` checks that it is complete and well-formed.
- One place in the code names the implementation (name, version, language) as reports name it, and a test holds it together with the package manifest.

## Conformance cases as tests

- Every case under `vendor/cwa/conformance/cases/` is a test: assemble its snapshot, validate the trace against `trace.schema.json`, compare the trace without `trace_id`, `timings` and `recovery.detail`, and compare the payload bytes (README, Running a case). Every rejection case under `rejections/` is a test that the snapshot is rejected before assembly, with no payload and no trace.
- A case this port does not pass yet goes in a `PENDING` set in the conformance test, which marks it as a strict expected failure: the suite stays green, and fails as soon as the case starts passing. The set only shrinks. Remove a case from it in the commit that makes it pass. Rejection cases have no `PENDING`: a vendored one this port does not reject yet is fixed in its vendor commit. A case or rejection case that needs an optional component the port does not provide is skipped, not held in `PENDING` (README, Reporting results).
- When a re-vendor makes existing code fail in a way `PENDING` cannot hold (a wrong payload, a crash), no vendor-only commit can pass: vendor and fix in one commit, and say why in its body.

## Commits

- Gate every commit on the suite's exit code. When piping test output, `set -o pipefail` first, or `| tail` reports success for a failing run.
- [Conventional Commits 1.0.0](https://www.conventionalcommits.org/en/v1.0.0/): `type(scope): summary` in the imperative mood, lower case, no trailing period, at most 72 characters. Types: `feat`, `fix`, `test`, `refactor`, `perf`, `docs`, `build`, `chore`, `ci`. Scopes: `admission`, `defaults`, `conflicts`, `supersede`, `dedupe`, `diversity`, `fitting`, `render`, `tokenize`, `snapshot`, `trace`, `canonical`, `strings`, `contract`, `conformance`. The body says *why*, and lists the requirement IDs affected. Breaking changes use `!` after the scope and a `BREAKING CHANGE:` footer. A `test:` commit is only for tests that add coverage to existing, already-passing behavior.
- Re-vendor commits are `build(contract): vendor website <short sha>`.
- `git commit -s`. The maintainer's commit hook rejects a commit without a matching `Signed-off-by`, and strips `Co-Authored-By` trailers; don't add or restore them.
- The changelog is generated by git-cliff from the commit history (`cliff.toml`), so a commit's subject line is its changelog entry. Never edit `CHANGELOG.md` by hand; regenerate it at a release (`uvx git-cliff --tag vX.Y.Z -o CHANGELOG.md`).
- Stage paths explicitly. Never commit caches, build output, or anything under `.claude/`.

## Architecture rules

- **`assemble` is pure.** No network, filesystem, clock, randomness in the payload, environment reads, or model calls. Everything that can change the payload arrives in the snapshot (R-18, R-23). Only `trace_id` and `timings` may differ between two runs of the same snapshot; add a test that enforces this, and extend it when a new module could reach outside.
- **Validate at the boundary, once.** The schemas and the snapshot checks (README, Snapshot checks) run when the snapshot is loaded. Code after that trusts the model types and does not re-validate.
- **Determinism.** Order anything that reaches the payload or trace by explicit keys, as the README states them. Never iterate a hash map or set into output. Compare instants at full precision (R-2) and strings by UTF-16 code units (README, Ordering).
- **Don't guess.** If a snapshot needs behavior not built yet, fail loudly, naming the gap. Never emit a payload the spec would not allow.
- **Reason codes come from the registry.** Exclusions and refusals use the codes in `contract/reasons.json`, in its order (R-21). A new condition needs a new code in the website repository first.
- **Ship the spec's components.** Tokenizers `fixture-whitespace/v1` and `estimate-utf8/v1` and renderers `fixture-xml/v1` and `cwa-messages/v1`, resolved by id. They are the README's required set, the bullets under Tokenizers and renderers before its Optional heading, so a case that uses only them is never skipped. A snapshot naming another id is unsupported, not invalid: only a case that uses an optional component the port does not provide, such as `cwa-message-blocks/v1`, is skipped (README, Reporting results). Callers may register their own tokenizers per call, and renderers if the port takes any, but never under the id of a published component of that kind, even one the port does not provide: that stops the call before assembly, with no payload and no trace (R-16; PORTING.md, Step 4).
- PORTING.md's portability checklist lists where languages disagree. Each row is a unit test to write before the case that catches it.

## Documentation

- Update documentation incrementally: README.md status, this file when a rule changes, and code comments, in the commit that changes the behavior.
- Use Mermaid diagrams wherever a flow, ordering or plan reads faster as a picture. Check that each parses with Mermaid 12 before committing.

## Commands

| Command | What it does |
| --- | --- |
| `rustup toolchain install stable` | Install the toolchain and dependencies |
| `cargo build` | Build, if the language needs a build step |
| `cargo test` | The full suite; must pass before every commit |
| `python3 scripts/vendor_contract.py --website ../website` | Re-vendor the contract from a website checkout, and rewrite the lock |
| `python3 scripts/vendor_contract.py --verify` | Check `vendor/cwa/` against its lock |
| `cargo run --release --bin cwa-conformance` | Run every case and rejection and write `conformance-report.json`; until a native runner exists, `python3 scripts/conformance.py --command "<adapter>" ...` does it (PORTING.md, step 5) |
| `python3 scripts/check_report.py` | Check the committed report is complete and well-formed |
