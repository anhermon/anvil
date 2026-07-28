//! Recovery of tool calls that a model wrote as *text* instead of emitting on
//! the native tool-call channel.
//!
//! Small local models (qwen2.5:3b class, served through Ollama) regularly slip
//! out of the OpenAI `tool_calls` channel and write the call into the assistant
//! message body instead — as a ```` ```json ```` fence, a `<tool_call>` tag, or a
//! bare JSON object. A harness that only reads native `tool_calls` sees an
//! ordinary end-of-turn and stops the run, so a recoverable formatting slip
//! becomes a silent task failure.
//!
//! This module is pure and provider-agnostic: it turns text into candidate
//! calls. Deciding whether to *execute* a recovered call is the agent loop's
//! job.

use serde_json::{Map, Value};

/// A tool call recovered from assistant text.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedCall {
    /// Tool name as written by the model.
    pub name: String,
    /// Arguments object (possibly repaired).
    pub input: Value,
    /// Which text encoding it was recovered from — useful for diagnostics.
    pub format: CallFormat,
}

/// The text encoding a recovered call was written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallFormat {
    /// ```` ```tool ```` or ```` ```json ```` fenced block.
    Fenced,
    /// `<tool_call>…</tool_call>` tag.
    Tag,
    /// A bare JSON object sitting in the prose.
    Bare,
}

/// Re-escape raw control characters that appear *inside* JSON string literals.
///
/// Models routinely emit a literal newline inside a string (common when the
/// argument is a shell command or file body), which is invalid JSON.
fn escape_control_chars_in_strings(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_string = false;
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if in_string && ch == '\\' {
            out.push(ch);
            if let Some(next) = chars.next() {
                out.push(next);
            }
            continue;
        }
        match ch {
            '"' => {
                in_string = !in_string;
                out.push(ch);
            }
            '\n' if in_string => out.push_str("\\n"),
            '\t' if in_string => out.push_str("\\t"),
            '\r' if in_string => out.push_str("\\r"),
            _ => out.push(ch),
        }
    }
    out
}

/// Balance unclosed `{`/`[` by appending the missing closers.
///
/// Only counts brackets outside string literals, so a `}` inside a shell
/// command doesn't throw the count off.
fn close_unbalanced(text: &str) -> String {
    let mut braces = 0i32;
    let mut squares = 0i32;
    let mut in_string = false;
    let mut escaped = false;
    for ch in text.chars() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => braces += 1,
            '}' => braces -= 1,
            '[' => squares += 1,
            ']' => squares -= 1,
            _ => {}
        }
    }
    let mut out = text.to_string();
    // An unterminated string must be closed before its containing brackets.
    if in_string {
        out.push('"');
    }
    for _ in 0..squares.max(0) {
        out.push(']');
    }
    for _ in 0..braces.max(0) {
        out.push('}');
    }
    out
}

/// Best-effort JSON repair for near-miss model output.
///
/// Tries progressively more aggressive fixes and returns the first that parses.
/// Returns `None` when the text cannot be salvaged into an object.
#[must_use]
pub fn repair_json(raw: &str) -> Option<Value> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    // 1. As-is.
    if let Ok(v @ Value::Object(_)) = serde_json::from_str::<Value>(trimmed) {
        return Some(v);
    }
    // 2. Re-escape raw control characters inside strings.
    let mut fixed = escape_control_chars_in_strings(trimmed);
    if let Ok(v @ Value::Object(_)) = serde_json::from_str::<Value>(&fixed) {
        return Some(v);
    }
    // 3. Drop trailing commas before a closer.
    fixed = fixed.replace(",}", "}").replace(",]", "]");
    fixed = fixed.replace(", }", "}").replace(", ]", "]");
    if let Ok(v @ Value::Object(_)) = serde_json::from_str::<Value>(&fixed) {
        return Some(v);
    }
    // 4. Close unbalanced brackets — the truncation case (model hit max_tokens
    //    or stopped mid-fence).
    let closed = close_unbalanced(&fixed);
    if let Ok(v @ Value::Object(_)) = serde_json::from_str::<Value>(&closed) {
        return Some(v);
    }
    None
}

/// Pull the arguments object out of a decoded call, accepting the several key
/// names models use interchangeably.
fn extract_input(obj: &Map<String, Value>) -> Value {
    for key in ["input", "arguments", "parameters", "args"] {
        if let Some(v) = obj.get(key) {
            // Some models double-encode the arguments as a JSON *string*.
            if let Value::String(s) = v {
                if let Some(parsed) = repair_json(s) {
                    return parsed;
                }
            }
            if v.is_object() {
                return v.clone();
            }
        }
    }
    Value::Object(Map::new())
}

/// Build a call from a decoded JSON object, if it names a tool.
fn call_from_value(v: &Value, format: CallFormat) -> Option<ParsedCall> {
    let obj = v.as_object()?;
    // Accept both the flat `{"name":…}` shape and OpenAI's `{"function":{…}}`.
    let inner = obj
        .get("function")
        .and_then(Value::as_object)
        .unwrap_or(obj);
    let name = inner.get("name")?.as_str()?.trim().to_string();
    if name.is_empty() {
        return None;
    }
    Some(ParsedCall {
        name,
        input: extract_input(inner),
        format,
    })
}

/// Find each region between `open` and `close`, returning the inner slices.
fn between<'a>(text: &'a str, open: &str, close: &str) -> Vec<&'a str> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find(open) {
        let after = &rest[start + open.len()..];
        // An unterminated final block still carries a usable (truncated) payload.
        let (body, next) = match after.find(close) {
            Some(end) => (&after[..end], &after[end + close.len()..]),
            None => (after, ""),
        };
        found.push(body);
        if next.is_empty() {
            break;
        }
        rest = next;
    }
    found
}

/// Extract the outermost balanced `{…}` regions of `text`, ignoring braces
/// inside string literals.
fn top_level_objects(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut depth = 0usize;
    let mut start = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (i, &b) in bytes.iter().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_string = false;
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' => {
                if depth == 0 {
                    start = i;
                }
                depth += 1;
            }
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 && i >= start {
                    if let Some(s) = text.get(start..=i) {
                        out.push(s);
                    }
                }
            }
            _ => {}
        }
    }
    // A truncated trailing object is still worth a repair attempt.
    if depth > 0 {
        if let Some(s) = text.get(start..) {
            out.push(s);
        }
    }
    out
}

/// Recover tool calls a model wrote into assistant text.
///
/// Checks, in order: ```` ``` ````-fenced blocks, `<tool_call>` tags, then bare
/// JSON objects. Bare objects are only considered when neither of the explicit
/// forms matched, since prose containing an unrelated JSON example would
/// otherwise be misread as a call.
#[must_use]
pub fn parse_text_tool_calls(text: &str) -> Vec<ParsedCall> {
    let mut calls = Vec::new();

    // Fenced: ```tool / ```json / ```tool_call
    for marker in ["```tool_call", "```tool", "```json"] {
        for body in between(text, marker, "```") {
            if let Some(v) = repair_json(body) {
                if let Some(c) = call_from_value(&v, CallFormat::Fenced) {
                    calls.push(c);
                }
            }
        }
        if !calls.is_empty() {
            break;
        }
    }

    // Tagged: <tool_call>…</tool_call>
    for body in between(text, "<tool_call>", "</tool_call>") {
        if let Some(v) = repair_json(body) {
            if let Some(c) = call_from_value(&v, CallFormat::Tag) {
                calls.push(c);
            }
        }
    }

    if calls.is_empty() {
        for body in top_level_objects(text) {
            if !body.contains("\"name\"") {
                continue;
            }
            if let Some(v) = repair_json(body) {
                if let Some(c) = call_from_value(&v, CallFormat::Bare) {
                    calls.push(c);
                }
            }
        }
    }

    calls
}

#[cfg(test)]
mod tests {
    use super::{parse_text_tool_calls, repair_json, CallFormat};
    use serde_json::json;

    #[test]
    fn parses_well_formed_fenced_call() {
        let calls = parse_text_tool_calls("Let me look:\n```json\n{\"name\":\"bash\",\"input\":{\"command\":\"ls\"}}\n```\n");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "bash");
        assert_eq!(calls[0].input, json!({"command": "ls"}));
        assert_eq!(calls[0].format, CallFormat::Fenced);
    }

    #[test]
    fn parses_tool_call_tag() {
        let calls =
            parse_text_tool_calls("<tool_call>{\"name\": \"grep\", \"arguments\": {\"pattern\": \"fn main\"}}</tool_call>");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "grep");
        assert_eq!(calls[0].input, json!({"pattern": "fn main"}));
        assert_eq!(calls[0].format, CallFormat::Tag);
    }

    #[test]
    fn parses_bare_json_object() {
        let calls = parse_text_tool_calls("I will run {\"name\":\"bash\",\"parameters\":{\"command\":\"pwd\"}} now");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "bash");
        assert_eq!(calls[0].format, CallFormat::Bare);
    }

    #[test]
    fn recovers_truncated_fence() {
        // The exact shape observed killing a baseline run: the model stopped
        // mid-fence, leaving the object unterminated.
        let calls = parse_text_tool_calls("Here we go:\n```json\n{\"name\": \"bash\", \"input\": {\"command\": \"git status\"");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "bash");
        assert_eq!(calls[0].input, json!({"command": "git status"}));
    }

    #[test]
    fn repairs_literal_newline_inside_string() {
        let v = repair_json("{\"command\": \"echo one\ntwo\"}").expect("should repair");
        assert_eq!(v["command"], "echo one\ntwo");
    }

    #[test]
    fn repairs_trailing_comma() {
        let v = repair_json("{\"a\": 1,}").expect("should repair");
        assert_eq!(v["a"], 1);
    }

    #[test]
    fn accepts_openai_function_shape_with_stringified_args() {
        let calls = parse_text_tool_calls(
            "<tool_call>{\"function\":{\"name\":\"bash\",\"arguments\":\"{\\\"command\\\":\\\"ls\\\"}\"}}</tool_call>",
        );
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "bash");
        assert_eq!(calls[0].input, json!({"command": "ls"}));
    }

    #[test]
    fn ignores_prose_without_calls() {
        assert!(parse_text_tool_calls("The build passed and there are no TODOs.").is_empty());
    }

    #[test]
    fn ignores_json_without_a_name_field() {
        assert!(parse_text_tool_calls("Result was {\"count\": 3, \"ok\": true}").is_empty());
    }

    #[test]
    fn does_not_treat_brace_inside_string_as_structure() {
        let calls =
            parse_text_tool_calls("{\"name\":\"bash\",\"input\":{\"command\":\"awk '{print $1}' f\"}}");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].input, json!({"command": "awk '{print $1}' f"}));
    }

    #[test]
    fn multiple_tagged_calls_are_all_recovered() {
        let calls = parse_text_tool_calls(
            "<tool_call>{\"name\":\"bash\",\"input\":{\"command\":\"ls\"}}</tool_call>\
             <tool_call>{\"name\":\"bash\",\"input\":{\"command\":\"pwd\"}}</tool_call>",
        );
        assert_eq!(calls.len(), 2);
    }

    #[test]
    fn empty_text_yields_nothing() {
        assert!(parse_text_tool_calls("").is_empty());
        assert!(repair_json("   ").is_none());
    }
}
