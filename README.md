# cwa-assembler

A Rust assembler for the [Context Window Architecture](https://contextwindowarchitecture.io) (CWA) draft specification. It admits candidate items, resolves declared conflicts, fits them to a token budget, renders the payload and emits the trace.

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
rustup toolchain install stable
cargo build
cargo test
```

## Cost

Every reduction under budget pressure is its own fit test, and every fit test renders and counts the whole payload (conformance/README.md, Fitting). Keep the cost down at the source, as the spec advises: send no more passages than the route's budget can use, and bound slots with `max_per_source` or `max_tokens`.

## Conformance

```sh
cargo run --release --bin cwa-conformance
```

This runs every vendored case and rejection snapshot as `conformance/README.md` describes. It writes `conformance-report.json`, valid against `schema/conformance_report.schema.json`, and exits 1 unless every case passed and every rejection snapshot was rejected, apart from those skipped for an optional component. A case passes only when its payload matches byte for byte and its trace matches field for field, except `trace_id` and `timings`. The committed report is the current run: a test fails when it goes stale. A case is skipped only when it uses a tokenizer or renderer the vendored README lists under Optional, such as `cwa-message-blocks/v1`, and this package does not provide it; a case that uses only required ones and does not pass has failed.

## The contract

`vendor/cwa/` holds the published contract this implementation follows: the schemas, the contract data and the conformance cases, copied from the website repository. `vendor/cwa.lock.json` pins each file by SHA-256 and records the website commit. It is Apache-2.0 licensed; see `vendor/cwa/LICENSE` and `vendor/cwa/NOTICE`.

See [AGENTS.md](AGENTS.md) for the working rules.

## License

Apache License 2.0, the same as the specification: see [LICENSE](LICENSE) and [NOTICE](NOTICE).
