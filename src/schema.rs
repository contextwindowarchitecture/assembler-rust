//! A JSON Schema 2020-12 validator for the vendored schemas (conformance/README.md, Running a case, step 1).
//!
//! It interprets the schemas as published rather than restating them in code, and implements exactly the
//! keywords they use; a test fails when a re-vendored schema uses another. Three choices follow the README:
//!
//! - `format` is asserted, for `date-time` and `date`, by the rules in Timestamps, because the patterns do not
//!   check the calendar.
//! - Patterns are ECMAScript regular expressions. The published ones use `\uXXXX` escapes, which Rust's `regex`
//!   reads the same way, and the end anchor `(?![\s\S])`, which it lacks and which means `\z`. A test checks
//!   that every vendored pattern is one this translation covers.
//! - Numbers are the doubles they read as (Numbers), so `integer` means a double with no fractional part.

use std::collections::HashMap;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value};

use crate::instant::{is_date, Instant};

const BASE: &str = "https://contextwindowarchitecture.io/schema/";

/// The vendored schemas, by file name.
pub const SOURCES: &[(&str, &str)] = &[
    ("conflict_group.schema.json", include_str!("../vendor/cwa/schema/conflict_group.schema.json")),
    ("conformance_report.schema.json", include_str!("../vendor/cwa/schema/conformance_report.schema.json")),
    ("context_item.schema.json", include_str!("../vendor/cwa/schema/context_item.schema.json")),
    ("producer_batch.schema.json", include_str!("../vendor/cwa/schema/producer_batch.schema.json")),
    ("profile.schema.json", include_str!("../vendor/cwa/schema/profile.schema.json")),
    ("registry_lock.schema.json", include_str!("../vendor/cwa/schema/registry_lock.schema.json")),
    ("route_policy.schema.json", include_str!("../vendor/cwa/schema/route_policy.schema.json")),
    ("snapshot.schema.json", include_str!("../vendor/cwa/schema/snapshot.schema.json")),
    ("trace.schema.json", include_str!("../vendor/cwa/schema/trace.schema.json")),
];

/// The keywords this validator implements; annotations and identifiers included.
pub const KEYWORDS: &[&str] = &[
    "$schema", "$id", "$defs", "$ref", "title", "description", "type", "enum", "const", "required", "properties",
    "additionalProperties", "propertyNames", "minProperties", "items", "minItems", "maxItems", "uniqueItems",
    "minLength", "maxLength", "pattern", "format", "minimum", "maximum", "allOf", "anyOf", "not", "if", "then", "else",
];

/// One way an instance fails a schema: where in the instance, which keyword, and what in words.
#[derive(Debug, Clone, PartialEq)]
pub struct SchemaError {
    /// JSON Pointer into the instance.
    pub path: String,
    pub keyword: &'static str,
    pub message: String,
    /// For `required`, the missing member's name.
    pub missing: Option<String>,
}

impl std::fmt::Display for SchemaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", if self.path.is_empty() { "/" } else { &self.path }, self.message)
    }
}

struct Schemas {
    documents: HashMap<String, Value>,
    patterns: HashMap<String, Regex>,
}

static SCHEMAS: LazyLock<Schemas> = LazyLock::new(|| {
    let mut documents = HashMap::new();
    let mut patterns = HashMap::new();
    for (name, text) in SOURCES {
        let document: Value = serde_json::from_str(text).unwrap_or_else(|e| panic!("vendored {name}: {e}"));
        collect_patterns(&document, &mut patterns);
        documents.insert(format!("{BASE}{name}"), document);
    }
    Schemas { documents, patterns }
});

fn collect_patterns(schema: &Value, patterns: &mut HashMap<String, Regex>) {
    match schema {
        Value::Object(map) => {
            if let Some(Value::String(p)) = map.get("pattern") {
                let regex = translate_pattern(p).unwrap_or_else(|e| panic!("vendored pattern {p:?}: {e}"));
                patterns.insert(p.clone(), regex);
            }
            map.values().for_each(|v| collect_patterns(v, patterns));
        }
        Value::Array(items) => items.iter().for_each(|v| collect_patterns(v, patterns)),
        _ => {}
    }
}

/// An ECMAScript pattern from the vendored schemas as a Rust regex: `(?![\s\S])`, the end of the input, is `\z`.
/// `\s` elsewhere would mean a different set in Rust, so a pattern holding one is refused, and so is a `.`
/// outside a character class; inside one (`[A-Za-z0-9_.-]`) it is a literal dot in both languages.
pub fn translate_pattern(pattern: &str) -> Result<Regex, String> {
    let translated = pattern.replace(r"(?![\s\S])", r"\z");
    let unanchored = without_class_dots(&translated.replace(r"[\s\S]", "").replace(r"\.", ""));
    if unanchored.contains(r"\s") || unanchored.contains(r"\S") || unanchored.contains(r"\d") || unanchored.contains(r"\w")
        || unanchored.contains(r"\b") || unanchored.contains('.')
    {
        return Err("uses a class whose meaning differs between ECMAScript and Rust".into());
    }
    Regex::new(&translated).map_err(|e| e.to_string())
}

/// The pattern without the dots inside its character classes, which are literal in ECMAScript and Rust alike.
fn without_class_dots(pattern: &str) -> String {
    let mut out = String::with_capacity(pattern.len());
    let (mut in_class, mut escaped) = (false, false);
    for c in pattern.chars() {
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if in_class && c == ']' {
            in_class = false;
        } else if c == '[' {
            in_class = true;
        } else if in_class && c == '.' {
            continue;
        }
        out.push(c);
    }
    out
}

/// Validates an instance against a vendored schema, named by file (`"snapshot.schema.json"`), and returns every
/// error found, in document order. An empty list means the instance is valid.
pub fn validate(schema_file: &str, instance: &Value) -> Vec<SchemaError> {
    let uri = format!("{BASE}{schema_file}");
    let schema = SCHEMAS.documents.get(&uri).unwrap_or_else(|| panic!("no vendored schema {schema_file}"));
    let mut errors = Vec::new();
    Validator { base: &uri }.check(schema, instance, "", &mut errors);
    errors
}

/// Whether an instance is valid against a vendored schema.
pub fn is_valid(schema_file: &str, instance: &Value) -> bool {
    validate(schema_file, instance).is_empty()
}

struct Validator<'a> {
    base: &'a str,
}

impl Validator<'_> {
    fn valid(&self, schema: &Value, instance: &Value) -> bool {
        let mut errors = Vec::new();
        self.check(schema, instance, "", &mut errors);
        errors.is_empty()
    }

    fn check(&self, schema: &Value, instance: &Value, path: &str, errors: &mut Vec<SchemaError>) {
        let map = match schema {
            Value::Bool(true) => return,
            Value::Bool(false) => {
                errors.push(error(path, "false", "is not allowed here"));
                return;
            }
            Value::Object(map) => map,
            _ => panic!("a schema is an object or a boolean"),
        };
        if let Some(Value::String(reference)) = map.get("$ref") {
            let (uri, target) = self.resolve(reference);
            Validator { base: &uri }.check(target, instance, path, errors);
        }
        if let Some(t) = map.get("type") {
            let allowed: Vec<&str> = match t {
                Value::String(s) => vec![s.as_str()],
                Value::Array(ts) => ts.iter().filter_map(Value::as_str).collect(),
                _ => panic!("type is a string or an array"),
            };
            if !allowed.iter().any(|t| has_type(instance, t)) {
                errors.push(error(path, "type", &format!("is not of type {}", allowed.join(" or "))));
                return;
            }
        }
        if let Some(Value::Array(options)) = map.get("enum") {
            if !options.contains(instance) {
                errors.push(error(path, "enum", &format!("{} is not one of {}", show(instance), show(&Value::Array(options.clone())))));
            }
        }
        if let Some(expected) = map.get("const") {
            if instance != expected {
                errors.push(error(path, "const", &format!("{} is not {}", show(instance), show(expected))));
            }
        }
        match instance {
            Value::Object(object) => self.check_object(map, object, path, errors),
            Value::Array(items) => self.check_array(map, items, path, errors),
            Value::String(s) => self.check_string(map, s, path, errors),
            Value::Number(n) => {
                let x = n.as_f64().expect("numbers are doubles");
                if let Some(min) = map.get("minimum").and_then(Value::as_f64) {
                    if x < min {
                        errors.push(error(path, "minimum", &format!("{x} is less than {min}")));
                    }
                }
                if let Some(max) = map.get("maximum").and_then(Value::as_f64) {
                    if x > max {
                        errors.push(error(path, "maximum", &format!("{x} is greater than {max}")));
                    }
                }
            }
            _ => {}
        }
        if let Some(Value::Array(all)) = map.get("allOf") {
            for sub in all {
                self.check(sub, instance, path, errors);
            }
        }
        if let Some(Value::Array(any)) = map.get("anyOf") {
            if !any.iter().any(|sub| self.valid_at(sub, instance)) {
                errors.push(error(path, "anyOf", "matches none of the allowed shapes"));
            }
        }
        if let Some(not) = map.get("not") {
            if self.valid_at(not, instance) {
                errors.push(error(path, "not", "matches a shape that is not allowed"));
            }
        }
        if let Some(condition) = map.get("if") {
            let branch = if self.valid_at(condition, instance) { map.get("then") } else { map.get("else") };
            if let Some(branch) = branch {
                self.check(branch, instance, path, errors);
            }
        }
    }

    fn valid_at(&self, schema: &Value, instance: &Value) -> bool {
        self.valid(schema, instance)
    }

    fn check_object(&self, map: &Map<String, Value>, object: &Map<String, Value>, path: &str, errors: &mut Vec<SchemaError>) {
        if let Some(Value::Array(required)) = map.get("required") {
            for name in required.iter().filter_map(Value::as_str) {
                if !object.contains_key(name) {
                    errors.push(SchemaError {
                        path: path.to_string(),
                        keyword: "required",
                        message: format!("{name:?} is required"),
                        missing: Some(name.to_string()),
                    });
                }
            }
        }
        if let Some(min) = map.get("minProperties").and_then(Value::as_f64) {
            if (object.len() as f64) < min {
                errors.push(error(path, "minProperties", &format!("has fewer than {min} members")));
            }
        }
        let properties = map.get("properties").and_then(Value::as_object);
        for (name, value) in object {
            let child = format!("{path}/{}", name.replace('~', "~0").replace('/', "~1"));
            if let Some(names) = map.get("propertyNames") {
                let mut name_errors = Vec::new();
                self.check(names, &Value::String(name.clone()), &child, &mut name_errors);
                if !name_errors.is_empty() {
                    errors.push(error(&child, "propertyNames", &format!("{name:?} is not an allowed member name")));
                }
            }
            match properties.and_then(|p| p.get(name)) {
                Some(sub) => self.check(sub, value, &child, errors),
                None => {
                    if let Some(additional) = map.get("additionalProperties") {
                        if additional == &Value::Bool(false) {
                            errors.push(error(path, "additionalProperties", &format!("{name:?} is not an allowed member")));
                        } else {
                            self.check(additional, value, &child, errors);
                        }
                    }
                }
            }
        }
    }

    fn check_array(&self, map: &Map<String, Value>, items: &[Value], path: &str, errors: &mut Vec<SchemaError>) {
        if let Some(min) = map.get("minItems").and_then(Value::as_f64) {
            if (items.len() as f64) < min {
                errors.push(error(path, "minItems", &format!("has fewer than {min} items")));
            }
        }
        if let Some(max) = map.get("maxItems").and_then(Value::as_f64) {
            if (items.len() as f64) > max {
                errors.push(error(path, "maxItems", &format!("has more than {max} items")));
            }
        }
        if map.get("uniqueItems") == Some(&Value::Bool(true)) {
            for (i, item) in items.iter().enumerate() {
                if items[..i].contains(item) {
                    errors.push(error(path, "uniqueItems", &format!("repeats {}", show(item))));
                    break;
                }
            }
        }
        if let Some(sub) = map.get("items") {
            for (i, item) in items.iter().enumerate() {
                self.check(sub, item, &format!("{path}/{i}"), errors);
            }
        }
    }

    fn check_string(&self, map: &Map<String, Value>, s: &str, path: &str, errors: &mut Vec<SchemaError>) {
        let length = s.chars().count() as f64;
        if let Some(min) = map.get("minLength").and_then(Value::as_f64) {
            if length < min {
                errors.push(error(path, "minLength", &format!("is shorter than {min} characters")));
            }
        }
        if let Some(max) = map.get("maxLength").and_then(Value::as_f64) {
            if length > max {
                errors.push(error(path, "maxLength", &format!("is longer than {max} characters")));
            }
        }
        if let Some(Value::String(pattern)) = map.get("pattern") {
            let regex = SCHEMAS.patterns.get(pattern).expect("every vendored pattern is compiled at load");
            if !regex.is_match(s) {
                errors.push(error(path, "pattern", &format!("{} does not match {pattern:?}", show(&Value::String(s.into())))));
            }
        }
        if let Some(Value::String(format)) = map.get("format") {
            let ok = match format.as_str() {
                "date-time" => Instant::parse(s).is_some(),
                "date" => is_date(s),
                _ => true,
            };
            if !ok {
                errors.push(error(path, "format", &format!("{} is not a valid {format}", show(&Value::String(s.into())))));
            }
        }
    }

    /// Resolves a `$ref` against this schema's base URI: a fragment in the same document, a file beside it, or an
    /// absolute URI, each with an optional JSON Pointer fragment.
    fn resolve(&self, reference: &str) -> (String, &'static Value) {
        let (location, fragment) = reference.split_once('#').unwrap_or((reference, ""));
        let uri = if location.is_empty() {
            self.base.to_string()
        } else if location.contains("://") {
            location.to_string()
        } else {
            format!("{}{location}", &self.base[..=self.base.rfind('/').expect("base URIs have a path")])
        };
        let document = SCHEMAS.documents.get(&uri).unwrap_or_else(|| panic!("unresolvable $ref {reference}"));
        let target = if fragment.is_empty() {
            document
        } else {
            document.pointer(fragment).unwrap_or_else(|| panic!("unresolvable $ref {reference}"))
        };
        (uri, target)
    }
}

fn has_type(instance: &Value, t: &str) -> bool {
    match t {
        "object" => instance.is_object(),
        "array" => instance.is_array(),
        "string" => instance.is_string(),
        "boolean" => instance.is_boolean(),
        "null" => instance.is_null(),
        "number" => instance.is_number(),
        "integer" => instance.as_f64().is_some_and(|x| x.fract() == 0.0),
        _ => panic!("unknown type {t}"),
    }
}

fn error(path: &str, keyword: &'static str, message: &str) -> SchemaError {
    SchemaError { path: path.to_string(), keyword, message: message.to_string(), missing: None }
}

fn show(value: &Value) -> String {
    let text = value.to_string();
    if text.chars().count() > 80 {
        format!("{}…", text.chars().take(80).collect::<String>())
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn keywords_in(schema: &Value, under_properties: bool, found: &mut Vec<String>) {
        match schema {
            Value::Object(map) => {
                for (key, value) in map {
                    if !under_properties {
                        found.push(key.clone());
                    }
                    let names_children = matches!(key.as_str(), "properties" | "$defs") && !under_properties;
                    if names_children {
                        value.as_object().unwrap().values().for_each(|v| keywords_in(v, false, found));
                    } else if !matches!(key.as_str(), "enum" | "const" | "required") {
                        keywords_in(value, false, found);
                    }
                }
            }
            Value::Array(items) => items.iter().for_each(|v| keywords_in(v, false, found)),
            _ => {}
        }
    }

    #[test]
    fn every_keyword_in_the_vendored_schemas_is_implemented() {
        for (name, text) in SOURCES {
            let mut found = Vec::new();
            keywords_in(&serde_json::from_str(text).unwrap(), false, &mut found);
            for keyword in found {
                assert!(KEYWORDS.contains(&keyword.as_str()), "{name} uses {keyword}, which this validator does not implement");
            }
        }
    }

    #[test]
    fn every_vendored_format_is_asserted() {
        for (name, text) in SOURCES {
            for format in text.match_indices("\"format\": \"").map(|(i, _)| &text[i + 11..]) {
                let format = &format[..format.find('"').unwrap()];
                assert!(matches!(format, "date-time" | "date"), "{name} uses format {format}");
            }
        }
    }

    #[test]
    fn every_vendored_pattern_translates() {
        LazyLock::force(&SCHEMAS);
        assert!(SCHEMAS.patterns.len() >= 4);
    }

    #[test]
    fn patterns_follow_ecmascript() {
        let item: Value = serde_json::from_str(SOURCES[2].1).unwrap();
        let published = item["properties"]["id"]["pattern"].as_str().unwrap();
        assert!(published.contains(r"\u00a0") && published.contains(r"\ufeff"), "the published escapes reach the translation");
        let non_blank = translate_pattern(published).unwrap();
        assert!(!non_blank.is_match("\u{feff}"));
        assert!(!non_blank.is_match(" \u{b}\u{3000}"));
        assert!(non_blank.is_match("\u{1c}"));
        let end = translate_pattern(r"^[0-9]{4}(?![\s\S])").unwrap();
        assert!(end.is_match("2026"));
        assert!(!end.is_match("2026\n"));
        assert!(translate_pattern(r"^\s+$").is_err());
    }

    #[test]
    fn a_dot_in_a_class_is_a_literal_dot() {
        let repository = translate_pattern(r"^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+(?![\s\S])").unwrap();
        assert!(repository.is_match("contextwindowarchitecture/website"));
        assert!(repository.is_match("a.b/c-d_e"));
        assert!(!repository.is_match("a,b/c"));
        assert!(!repository.is_match("a/b\n"));
        assert!(translate_pattern(r"^a.b$").is_err(), "a dot outside a class matches a different set in each language");
        assert!(translate_pattern(r"^[\d.]$").is_err(), "a class does not excuse the other escapes");
    }

    #[test]
    fn date_times_need_both_the_pattern_and_the_calendar() {
        let item = |freshness: &str| json!({"id": "a", "slot": "interaction.query", "source": "s", "source_version": "1",
            "authority": "user", "trust": "unverified", "freshness": freshness, "body": "b"});
        assert!(is_valid("context_item.schema.json", &item("2026-09-22T12:00:00.5+02:00")));
        let errors = validate("context_item.schema.json", &item("2026-02-30T12:00:00Z"));
        assert_eq!(errors.iter().map(|e| e.keyword).collect::<Vec<_>>(), ["format"]);
        let errors = validate("context_item.schema.json", &item("2016-12-31T23:59:60Z"));
        assert!(errors.iter().any(|e| e.keyword == "pattern"));
    }

    #[test]
    fn conditional_requirements_report_the_missing_member() {
        let item = json!({"id": "m", "slot": "interaction.memory", "source": "turn:1", "source_version": "1",
            "authority": "generated", "trust": "unverified", "freshness": "2026-09-22T12:00:00Z"});
        let missing: Vec<_> = validate("context_item.schema.json", &item).into_iter().filter_map(|e| e.missing).collect();
        assert_eq!(missing, ["body", "expires"]);
    }

    #[test]
    fn integers_are_doubles_without_a_fraction() {
        let budget = |input: Value| json!({"input": input, "reserved_output": 0});
        let snapshot_budget = |b| validate("snapshot.schema.json", &json!({"budget": b})).into_iter().any(|e| e.path.starts_with("/budget"));
        assert!(!snapshot_budget(budget(crate::json::to_doubles(json!(8192.0)))));
        assert!(snapshot_budget(budget(json!(81.5))));
        assert!(snapshot_budget(budget(json!(-1))));
    }
}
