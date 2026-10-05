//! Renderers (conformance/README.md, Tokenizers and renderers; R-7, R-11, R-16).
//!
//! A renderer turns the included occurrences, already in placement order and each placement's item order, into
//! the payload text, and counts it with the declared tokenizer. Every renderer here is a published one; this
//! package takes no application renderers.

use serde_json::{json, Map, Value};

use crate::canonical;
use crate::model::Profile;
use crate::tokenize::Tokenizer;

/// One included occurrence of an item, as a renderer sees it.
#[derive(Debug, Clone)]
pub struct Occurrence<'a> {
    pub wrap: &'a str,
    pub slot: &'a str,
    pub id: &'a str,
    /// The body this occurrence renders: the item's own, or the variant fitting chose.
    pub body: &'a str,
    /// The surfaced conflict group the item belongs to, if any.
    pub conflict: Option<&'a str>,
    /// The item's lineage is `generated`, which makes a history turn the assistant's.
    pub generated: bool,
}

/// A rendered payload and its count, the renderer's count of everything it emits as text.
#[derive(Debug, Clone)]
pub struct Rendered {
    pub payload: String,
    pub input_tokens: u64,
}

/// The renderers this package provides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Renderer {
    FixtureXml,
    CwaMessages,
    CwaMessageBlocks,
}

impl Renderer {
    /// The renderer with a published id, when this package provides it: every published one.
    pub fn builtin(id: &str) -> Option<Renderer> {
        match id {
            "fixture-xml/v1" => Some(Renderer::FixtureXml),
            "cwa-messages/v1" => Some(Renderer::CwaMessages),
            "cwa-message-blocks/v1" => Some(Renderer::CwaMessageBlocks),
            _ => None,
        }
    }

    /// Why this renderer cannot realize a profile, or `None` when it can (Snapshot checks, Profile).
    pub fn unrealizable(self, profile: &Profile) -> Option<String> {
        let mut seen_xml = false;
        for placement in &profile.placement {
            let (slot, wrap) = (placement.slot.as_str(), placement.wrap.as_str());
            if let Some(tag) = wrap.strip_prefix("xml:") {
                if !is_tag(tag) {
                    return Some(format!("{slot}'s wrap {wrap:?} has an invalid tag: it must match [A-Za-z_][A-Za-z0-9_.-]*"));
                }
                seen_xml = true;
                continue;
            }
            if self == Renderer::FixtureXml {
                return Some(format!("{slot}'s wrap {wrap:?} is not an xml: wrap, the only kind fixture-xml/v1 renders"));
            }
            match wrap {
                "system" if !slot.starts_with("governance.") => {
                    return Some(format!("{slot} is placed as system, a platform role only governance slots may take"))
                }
                "system" if seen_xml => {
                    return Some(format!("{slot} is placed as system after an xml: placement, and a message request cannot put material ahead of its system text"))
                }
                "tools" if slot != "governance.capabilities" => {
                    return Some(format!("{slot} is placed as tools, which only governance.capabilities may use"))
                }
                "system" | "tools" => {}
                _ => return Some(format!("{slot}'s wrap {wrap:?} is not system, tools or an xml: wrap")),
            }
        }
        None
    }

    /// Renders the occurrences and counts the payload.
    pub fn render(self, occurrences: &[Occurrence], tokenizer: &dyn Tokenizer) -> Rendered {
        match self {
            Renderer::FixtureXml => {
                let mut payload = String::new();
                for o in occurrences {
                    payload.push_str(&xml_element(o, false));
                }
                let input_tokens = tokenizer.count(&payload);
                Rendered { payload, input_tokens }
            }
            Renderer::CwaMessages | Renderer::CwaMessageBlocks => {
                let mut system = Vec::new();
                let mut tools = Vec::new();
                let mut blocks = Vec::new();
                let mut content = String::new();
                let mut input_tokens = 0;
                for o in occurrences {
                    match o.wrap {
                        "system" | "tools" => {
                            let text = match o.conflict {
                                Some(group) => format!("<conflict group=\"{}\">\n{}\n</conflict>", escape_attribute(group), o.body),
                                None => o.body.to_string(),
                            };
                            input_tokens += tokenizer.count(&text);
                            let entry = entry(o, text);
                            if o.wrap == "system" { system.push(entry) } else { tools.push(entry) }
                        }
                        _ => {
                            let text = xml_element(o, true);
                            if self == Renderer::CwaMessageBlocks {
                                input_tokens += tokenizer.count(&text);
                                blocks.push(entry(o, text));
                            } else {
                                content.push_str(&text);
                            }
                        }
                    }
                }
                let content = if self == Renderer::CwaMessageBlocks {
                    Value::Array(blocks)
                } else {
                    input_tokens += tokenizer.count(&content);
                    Value::String(content)
                };
                let request = json!({
                    "system": system,
                    "tools": tools,
                    "messages": [{"role": "user", "content": content}],
                });
                Rendered { payload: canonical::to_string(&request), input_tokens }
            }
        }
    }
}

/// An occurrence's rendered body, which its per-item `tokens` count: escaped inside an `xml:` wrap, and as it
/// is in `system` and `tools`, whose conflict mark, like an `xml:` wrapper, counts only in the payload.
pub fn rendered_body(wrap: &str, body: &str) -> String {
    if wrap.starts_with("xml:") { escape_body(body) } else { body.to_string() }
}

fn entry(o: &Occurrence, text: String) -> Value {
    let mut entry = Map::new();
    entry.insert("id".into(), Value::String(o.id.into()));
    entry.insert("text".into(), Value::String(text));
    if let Some(group) = o.conflict {
        entry.insert("conflict".into(), Value::String(group.into()));
    }
    Value::Object(entry)
}

/// `<{tag} id="{id}">\n{body}\n</{tag}>\n`, with ` speaker` on a history turn in a message request and
/// ` conflict` on a member of a surfaced group.
fn xml_element(o: &Occurrence, speaker: bool) -> String {
    let tag = o.wrap.strip_prefix("xml:").expect("realizable profiles wrap xml: here");
    let mut open = format!("<{tag} id=\"{}\"", escape_attribute(o.id));
    if speaker && o.slot == "interaction.history" {
        open.push_str(if o.generated { " speaker=\"assistant\"" } else { " speaker=\"user\"" });
    }
    if let Some(group) = o.conflict {
        open.push_str(&format!(" conflict=\"{}\"", escape_attribute(group)));
    }
    format!("{open}>\n{}\n</{tag}>\n", escape_body(o.body))
}

fn is_tag(tag: &str) -> bool {
    let mut chars = tag.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

fn escape_body(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn escape_attribute(text: &str) -> String {
    escape_body(text).replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokenize::FixtureWhitespace;

    fn occurrence<'a>(wrap: &'a str, slot: &'a str, id: &'a str, body: &'a str) -> Occurrence<'a> {
        Occurrence { wrap, slot, id, body, conflict: None, generated: false }
    }

    #[test]
    fn fixture_xml_escapes_bodies_and_attributes() {
        let mut o = occurrence("xml:evidence", "evidence.knowledge", "a&<\">", "x < y & \"z\"");
        o.conflict = Some("g\"1");
        let r = Renderer::FixtureXml.render(&[o], &FixtureWhitespace);
        assert_eq!(r.payload, "<evidence id=\"a&amp;&lt;&quot;&gt;\" conflict=\"g&quot;1\">\nx &lt; y &amp; \"z\"\n</evidence>\n");
        assert_eq!(r.input_tokens, 9);
    }

    #[test]
    fn messages_keep_platform_roles_with_the_application() {
        let mut system = occurrence("system", "governance.instructions", "p", "Be <terse>.");
        system.conflict = Some("g");
        let mut turn = occurrence("xml:history", "interaction.history", "t", "Hi");
        turn.generated = true;
        let occurrences = [system, occurrence("tools", "governance.capabilities", "c", "{}"), turn];
        let r = Renderer::CwaMessages.render(&occurrences, &FixtureWhitespace);
        assert_eq!(r.payload, concat!(
            "{\"messages\":[{\"content\":\"<history id=\\\"t\\\" speaker=\\\"assistant\\\">\\nHi\\n</history>\\n\",\"role\":\"user\"}],",
            "\"system\":[{\"conflict\":\"g\",\"id\":\"p\",\"text\":\"<conflict group=\\\"g\\\">\\nBe <terse>.\\n</conflict>\"}],",
            "\"tools\":[{\"id\":\"c\",\"text\":\"{}\"}]}"));
        assert_eq!(r.input_tokens, 5 + 1 + 5);
        let blocks = Renderer::CwaMessageBlocks.render(&occurrences, &FixtureWhitespace);
        assert!(blocks.payload.starts_with("{\"messages\":[{\"content\":[{\"id\":\"t\",\"text\":\"<history id=\\\"t\\\" speaker=\\\"assistant\\\">"));
    }

    #[test]
    fn rendered_bodies_escape_only_inside_xml() {
        assert_eq!(rendered_body("xml:q", "a<b"), "a&lt;b");
        assert_eq!(rendered_body("system", "a<b"), "a<b");
    }

    #[test]
    fn tags_are_ascii_names() {
        assert!(is_tag("evidence.knowledge"));
        assert!(is_tag("_x-1"));
        assert!(!is_tag("1evidence"));
        assert!(!is_tag(""));
        assert!(!is_tag("évidence"));
    }
}
