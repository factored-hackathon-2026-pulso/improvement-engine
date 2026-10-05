//! Tolerant extraction of ONE JSON object from the text a model answered (BLD1).
//!
//! Real models wrap the object: a code fence, a sentence before it, a reasoning block, a trailing comma. The extractor removes
//! exactly that packaging and nothing else: it never invents a key, a value or a type. What comes out is parsed JSON that the
//! role parsers still validate STRICTLY (closed key set, enums, ranges), so tolerance here never widens what is accepted.
//!
//! Failure modes are typed (`ExtractError::code`), so a report can say WHY an answer was unusable: `empty`, `no_json` (prose only),
//! `truncated` (an object was opened and never closed: the output hit the token limit), `unparseable` (balanced braces that are
//! not JSON even after the trailing-comma repair), `not_object` (valid JSON that is not an object).
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtractError {
    Empty,
    NoJson,
    Truncated,
    Unparseable,
    NotObject,
}

impl ExtractError {
    pub fn code(&self) -> &'static str {
        match self {
            ExtractError::Empty => "empty",
            ExtractError::NoJson => "no_json",
            ExtractError::Truncated => "truncated",
            ExtractError::Unparseable => "unparseable",
            ExtractError::NotObject => "not_object",
        }
    }
}

/// Removes `<think>...</think>` blocks (and an unterminated `<think>` tail): a reasoning model's scratch text, never an answer.
fn strip_reasoning(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find("<think>") {
        out.push_str(&rest[..i]);
        match rest[i..].find("</think>") {
            Some(j) => rest = &rest[i + j + "</think>".len()..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// Drops commas that directly precede `}` or `]` (outside strings). The only repair applied.
pub fn repair_trailing_commas(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let (mut in_str, mut esc) = (false, false);
    for (i, &c) in chars.iter().enumerate() {
        if in_str {
            out.push(c);
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        if c == '"' {
            in_str = true;
            out.push(c);
            continue;
        }
        if c == ',' {
            let next = chars[i + 1..].iter().find(|x| !x.is_whitespace());
            if matches!(next, Some('}') | Some(']')) {
                continue;
            }
        }
        out.push(c);
    }
    out
}

/// End (exclusive, byte index) of the balanced object that starts at `start` (a `{`), string-aware; `None` when it never closes.
fn balanced_end(s: &str, start: usize) -> Option<usize> {
    let (mut depth, mut in_str, mut esc) = (0i32, false, false);
    for (i, c) in s[start..].char_indices() {
        if in_str {
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(start + i + 1);
                }
            }
            _ => {}
        }
    }
    None
}

fn parse_object(candidate: &str) -> Option<Value> {
    serde_json::from_str::<Value>(candidate).ok().or_else(|| serde_json::from_str::<Value>(&repair_trailing_commas(candidate)).ok()).filter(Value::is_object)
}

/// The first JSON object in `text` that parses (after the trailing-comma repair).
pub fn extract_json_object(text: &str) -> Result<Value, ExtractError> {
    let clean = strip_reasoning(text);
    let t = clean.trim();
    if t.is_empty() {
        return Err(ExtractError::Empty);
    }
    if let Ok(v) = serde_json::from_str::<Value>(t) {
        return if v.is_object() { Ok(v) } else { Err(ExtractError::NotObject) };
    }
    let (mut at, mut saw_open, mut saw_balanced) = (0usize, false, false);
    while let Some(i) = t[at..].find('{') {
        let start = at + i;
        saw_open = true;
        match balanced_end(t, start) {
            Some(end) => {
                saw_balanced = true;
                if let Some(v) = parse_object(&t[start..end]) {
                    return Ok(v);
                }
                at = end; // an unparseable balanced block (`{id}` in prose): skip it whole, never return its inner pieces
            }
            None => {
                // never closed. An answer that still ENDS with a closing brace was not cut off, its structure is broken (a key that lost
                // its opening quote shifts the string scan: BLD1, `],examples_pt":[`); only an answer that stops mid-way is truncated.
                let cut = !t.ends_with('}') && !saw_balanced;
                return Err(if cut { ExtractError::Truncated } else { ExtractError::Unparseable });
            }
        }
    }
    Err(if !saw_open {
        ExtractError::NoJson
    } else {
        ExtractError::Unparseable
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn plain_object() {
        assert_eq!(extract_json_object(r#"{"a": 1}"#), Ok(json!({"a": 1})));
    }

    #[test]
    fn code_fence_with_and_without_language() {
        assert_eq!(extract_json_object("```json\n{\"a\": 1}\n```"), Ok(json!({"a": 1})));
        assert_eq!(extract_json_object("```\n{\"a\": [1, 2]}\n```"), Ok(json!({"a": [1, 2]})));
    }

    #[test]
    fn prose_before_and_after() {
        assert_eq!(extract_json_object("Here is the answer:\n{\"a\": \"x\"}\nHope it helps."), Ok(json!({"a": "x"})));
    }

    #[test]
    fn reasoning_block_is_dropped_even_with_braces_inside() {
        let t = "<think>maybe {\"a\": 0} or not</think>\n{\"a\": 2}";
        assert_eq!(extract_json_object(t), Ok(json!({"a": 2})));
    }

    #[test]
    fn braces_inside_strings_do_not_unbalance() {
        assert_eq!(extract_json_object("x {\"a\": \"}{ \\\" }\", \"b\": {\"c\": 1}} y"), Ok(json!({"a": "}{ \" }", "b": {"c": 1}})));
    }

    #[test]
    fn trailing_commas_are_repaired_but_nothing_else() {
        assert_eq!(extract_json_object("{\"a\": [1, 2,], \"b\": {\"c\": 1,},}"), Ok(json!({"a": [1, 2], "b": {"c": 1}})));
        // a comma inside a string is data, not a trailing comma
        assert_eq!(extract_json_object("{\"a\": \"x,}\"}"), Ok(json!({"a": "x,}"})));
        // missing quotes are NOT repaired
        assert_eq!(extract_json_object("{a: 1}"), Err(ExtractError::Unparseable));
    }

    #[test]
    fn an_unparseable_prose_block_is_skipped_and_the_next_object_wins() {
        assert_eq!(extract_json_object("use {id} as key: {\"a\": 1}"), Ok(json!({"a": 1})));
    }

    #[test]
    fn typed_failure_modes() {
        assert_eq!(extract_json_object("   "), Err(ExtractError::Empty));
        assert_eq!(extract_json_object("<think>only thoughts"), Err(ExtractError::Empty));
        assert_eq!(extract_json_object("I cannot do that."), Err(ExtractError::NoJson));
        assert_eq!(extract_json_object("{\"a\": {\"b\": \"cut o"), Err(ExtractError::Truncated));
        assert_eq!(extract_json_object("[1, 2]"), Err(ExtractError::NotObject));
        assert_eq!(extract_json_object("\"hi\""), Err(ExtractError::NotObject));
        // a lost opening quote shifts the string scan: unbalanced, but the answer ends with a brace, so it is malformed, not cut off
        assert_eq!(extract_json_object("{\"a\": [\"x\"],b\": [\"y\"]}"), Err(ExtractError::Unparseable));
        assert_eq!(ExtractError::Truncated.code(), "truncated");
    }

    #[test]
    fn the_first_parseable_object_is_returned_not_the_last() {
        assert_eq!(extract_json_object("{\"a\": 1} then {\"a\": 2}"), Ok(json!({"a": 1})));
    }
}
