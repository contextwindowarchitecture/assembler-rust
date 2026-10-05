//! A Rust assembler for the Context Window Architecture (CWA) draft specification.
//!
//! [`assemble`] takes a frozen snapshot, in the shape of `schema/snapshot.schema.json`, and returns the rendered
//! payload, or none when the assembly is refused, together with the trace (R-17, R-21). A snapshot that fails its
//! schemas or the snapshot checks is rejected with its problems and no trace; a tokenizer or renderer this
//! package does not provide is unsupported. Everything but `trace_id` and `timings` is deterministic (R-23).

use std::collections::BTreeMap;
use std::fmt;

pub mod canonical;
pub mod contract;
pub mod instant;
pub mod json;
pub mod model;
pub mod render;
pub mod schema;
pub mod snapshot;
pub mod strings;
pub mod tokenize;

pub use tokenize::Tokenizer;

/// The implementation, as conformance reports name it. A test holds it to `Cargo.toml`.
pub const IMPLEMENTATION_NAME: &str = "cwa-assembler";
pub const IMPLEMENTATION_VERSION: &str = "0.0.1";
pub const IMPLEMENTATION_LANGUAGE: &str = "Rust";

/// Why a call produced no assembly at all: no payload and no trace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The snapshot fails its schemas or the snapshot checks, or is not I-JSON (R-17). Each entry is one problem.
    Rejected(Vec<String>),
    /// The snapshot names a tokenizer or renderer this package does not provide, and the caller passed none.
    /// Each entry reads `tokenizer <id> is not provided` or `renderer <id> is not provided`.
    Unsupported(Vec<String>),
    /// The caller passed a component under the id of a published one, which would let a trace name a published
    /// tokenizer while counting with another (R-16). Not a refusal: refusals end with a trace.
    PublishedId(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Rejected(problems) => write!(f, "snapshot rejected: {}", problems.join("; ")),
            Error::Unsupported(lines) => write!(f, "{}", lines.join("\n")),
            Error::PublishedId(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for Error {}

/// What a call may add to the snapshot: the application's own tokenizers, under ids no published tokenizer uses.
#[derive(Default)]
pub struct Options {
    tokenizers: BTreeMap<String, Box<dyn Tokenizer>>,
}

impl Options {
    pub fn new() -> Options {
        Options::default()
    }

    /// Adds a tokenizer under an id. Assembly stops before it begins if the id is a published tokenizer's (R-16).
    pub fn tokenizer(mut self, id: impl Into<String>, tokenizer: impl Tokenizer + 'static) -> Options {
        self.tokenizers.insert(id.into(), Box::new(tokenizer));
        self
    }
}

/// A snapshot ready to assemble: checked, with its components resolved.
pub(crate) struct Prepared<'o> {
    pub loaded: snapshot::Loaded,
    pub tokenizer: &'o dyn Tokenizer,
    pub renderer: render::Renderer,
}

/// Everything before assembly: the published-id guard, loading and checking the snapshot, resolving its
/// tokenizer and renderer, and the renderer's realizability check.
pub(crate) fn prepare<'o>(bytes: &[u8], options: &'o Options) -> Result<Prepared<'o>, Error> {
    for id in options.tokenizers.keys() {
        if contract::is_published("tokenizer", id) {
            return Err(Error::PublishedId(format!("tokenizer {id} is published; an application's tokenizer needs an id of its own")));
        }
    }
    let loaded = snapshot::load(bytes).map_err(Error::Rejected)?;
    let s = &loaded.snapshot;
    let tokenizer: Option<&dyn Tokenizer> = match options.tokenizers.get(&s.tokenizer) {
        Some(own) => Some(own.as_ref()),
        None => tokenize::builtin(&s.tokenizer),
    };
    let renderer = render::Renderer::builtin(&s.renderer);
    let mut missing = Vec::new();
    if tokenizer.is_none() {
        missing.push(format!("tokenizer {} is not provided", s.tokenizer));
    }
    if renderer.is_none() {
        missing.push(format!("renderer {} is not provided", s.renderer));
    }
    let (Some(tokenizer), Some(renderer)) = (tokenizer, renderer) else {
        return Err(Error::Unsupported(missing));
    };
    if let Some(problem) = renderer.unrealizable(&s.profile) {
        return Err(Error::Rejected(vec![format!("{} cannot realize the profile: {problem}", s.renderer)]));
    }
    Ok(Prepared { loaded, tokenizer, renderer })
}

#[doc(hidden)]
/// The snapshot digest, or the error that stops the call before assembly. For the conformance tests.
pub fn check(bytes: &[u8], options: &Options) -> Result<String, Error> {
    prepare(bytes, options).map(|p| p.loaded.digest)
}
