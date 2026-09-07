//! Reading a vendor's own account of what a run cost.
//!
//! Nothing here estimates. Every number comes from the vendor saying it, and a
//! vendor that says nothing produces nothing — a status line reporting a figure
//! we invented would be worse than one reporting a dash.
//!
//! The three CLIs shipped today report in three different shapes, on two
//! different streams, and one of them rounds. That is why extraction is
//! configuration rather than code: no vendor is named in this crate, and adding
//! a fourth shape is a file.

use ostraka_core::record::TokenUsage;
use serde::{Deserialize, Serialize};

/// Which stream a vendor writes its accounting to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Stream {
    #[default]
    Stdout,
    Stderr,
}

/// Which side of the marker the number sits on.
///
/// Both shapes are in the wild: `tokens used / 3,303` puts the label first, and
/// `7.5k input, 4 output` puts it last. One setting per profile is enough — a
/// vendor is consistent with itself even when vendors are not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    #[default]
    After,
    Before,
}

/// How to read the numbers out of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Shape {
    /// Locate a marker; the number is the first one at or after it.
    #[default]
    Text,
    /// Parse the stream as one JSON document and follow RFC 6901 pointers.
    Json,
}

/// One counter, or several that add up to one.
///
/// Several is the common case rather than the exotic one: a vendor that splits
/// input into fresh, cached and cache-creating tokens is reporting three parts
/// of one number, and showing only the first would understate a run by orders
/// of magnitude.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Keys {
    One(String),
    Several(Vec<String>),
}

impl Keys {
    fn iter(&self) -> std::slice::Iter<'_, String> {
        match self {
            Self::One(key) => std::slice::from_ref(key).iter(),
            Self::Several(keys) => keys.iter(),
        }
    }
}

/// Where a profile's vendor reports what it spent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageSpec {
    #[serde(default)]
    pub stream: Stream,
    #[serde(default)]
    pub shape: Shape,
    #[serde(default)]
    pub number: Side,
    /// Marker text, or a JSON pointer, depending on `shape`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<Keys>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<Keys>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<Keys>,
    /// This vendor rounds before reporting, so what it says is an estimate.
    #[serde(default)]
    pub approximate: bool,
}

impl UsageSpec {
    /// Reads the counts a vendor reported, or `None` if it reported none.
    pub fn read(
        &self,
        adapter: &str,
        role: &str,
        stdout: &str,
        stderr: &str,
    ) -> Option<TokenUsage> {
        let text = match self.stream {
            Stream::Stdout => stdout,
            Stream::Stderr => stderr,
        };
        let find = |keys: &Option<Keys>| -> Option<u64> {
            let mut found: Option<u64> = None;
            for key in keys.as_ref()?.iter() {
                let one = match (self.shape, self.number) {
                    (Shape::Text, Side::After) => after_marker(text, key),
                    (Shape::Text, Side::Before) => before_marker(text, key),
                    (Shape::Json, _) => at_pointer(text, key),
                };
                // A counter the vendor omitted contributes nothing; it does not
                // make the others unreadable.
                if let Some(n) = one {
                    found = Some(found.unwrap_or(0) + n);
                }
            }
            found
        };

        let usage = TokenUsage {
            adapter: adapter.to_string(),
            role: role.to_string(),
            input: find(&self.input),
            output: find(&self.output),
            total: find(&self.total),
            approximate: self.approximate,
        };
        // A row of nothing is not a measurement. Recording it would make a
        // vendor that reports nothing indistinguishable from one that ran and
        // spent nothing.
        (usage.input.is_some() || usage.output.is_some() || usage.total.is_some()).then_some(usage)
    }
}

/// The first number at or after `marker`, on its line or the next non-empty one.
///
/// Covers both shapes seen in the wild: a label with the number beside it, and a
/// label with the number on the line below.
fn after_marker(text: &str, marker: &str) -> Option<u64> {
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        let Some(rest) = line.split_once(marker).map(|(_, rest)| rest) else {
            continue;
        };
        if let Some(n) = first_number(rest) {
            return Some(n);
        }
        for next in lines.by_ref() {
            if next.trim().is_empty() {
                continue;
            }
            return first_number(next);
        }
    }
    None
}

/// The last number before `marker`, on the line the marker appears on.
///
/// Scans the whole prefix rather than the adjacent token, so a model name with
/// digits in it does not become the count: the number wanted is the one nearest
/// the label, which is the last one.
fn before_marker(text: &str, marker: &str) -> Option<u64> {
    text.lines().find_map(|line| {
        line.split_once(marker)
            .and_then(|(before, _)| last_number(before))
    })
}

/// The last number in some text, by the same rules as [`first_number`].
fn last_number(text: &str) -> Option<u64> {
    let bytes = text.as_bytes();
    let last_digit = (0..bytes.len())
        .rev()
        .find(|&i| bytes[i].is_ascii_digit())?;

    let mut start = last_digit;
    while start > 0
        && (bytes[start - 1].is_ascii_digit() || matches!(bytes[start - 1], b',' | b'_' | b'.'))
    {
        start -= 1;
    }
    // A `k` or `m` belongs to the number in front of it, and dropping it would
    // turn 7.5k into 7.
    let mut end = last_digit + 1;
    if end < bytes.len() && matches!(bytes[end], b'k' | b'K' | b'm' | b'M') {
        end += 1;
    }
    first_number(&text[start..end])
}

/// A number from the head of some text, tolerating separators and `k`/`m`.
///
/// A suffix means the vendor rounded. The profile says so with `approximate`,
/// because a value that is sometimes rounded is always an estimate.
fn first_number(text: &str) -> Option<u64> {
    let trimmed = text.trim_start_matches(|c: char| !c.is_ascii_digit());
    let digits: String = trimmed
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == ',' || *c == '_' || *c == '.')
        .collect();
    if digits.is_empty() {
        return None;
    }
    let cleaned: String = digits.chars().filter(|c| *c != ',' && *c != '_').collect();
    let value: f64 = cleaned.parse().ok()?;
    let scale = match trimmed[digits.len()..].chars().next() {
        Some('k') | Some('K') => 1_000.0,
        Some('m') | Some('M') => 1_000_000.0,
        _ => 1.0,
    };
    Some((value * scale).round() as u64)
}

fn at_pointer(text: &str, pointer: &str) -> Option<u64> {
    let document: serde_json::Value = serde_json::from_str(text.trim()).ok()?;
    document.pointer(pointer)?.as_u64()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(shape: Shape, stream: Stream) -> UsageSpec {
        UsageSpec {
            stream,
            shape,
            number: Side::After,
            total: None,
            input: None,
            output: None,
            approximate: false,
        }
    }

    #[test]
    fn a_label_with_the_number_on_the_next_line_is_read() {
        // One vendor prints the count under its label.
        let mut s = spec(Shape::Text, Stream::Stderr);
        s.total = Some(Keys::One("tokens used".into()));
        let usage = s
            .read("v", "author", "", "codex\nOK\ntokens used\n3,303\n")
            .expect("reads");
        assert_eq!(usage.total, Some(3303));
        assert!(!usage.approximate);
    }

    #[test]
    fn a_label_with_the_number_beside_it_is_read() {
        let mut s = spec(Shape::Text, Stream::Stderr);
        s.input = Some(Keys::One("input:".into()));
        s.output = Some(Keys::One("output:".into()));
        let usage = s
            .read("v", "author", "", "input: 1,024  output: 55\n")
            .expect("reads");
        assert_eq!((usage.input, usage.output), (Some(1024), Some(55)));
    }

    #[test]
    fn a_rounded_figure_is_expanded_and_flagged() {
        // "7.5k input" is not 7500 tokens, it is however many rounded to 7.5k.
        // Expanding it is fine; pretending it is exact is not.
        let mut s = spec(Shape::Text, Stream::Stderr);
        s.number = Side::Before;
        s.approximate = true;
        s.input = Some(Keys::One("input".into()));
        s.output = Some(Keys::One("output".into()));
        // A model name with digits in it must not become the count.
        let usage = s
            .read(
                "v",
                "author",
                "",
                "  claude-haiku-4.5   7.5k input, 4 output, 0 cache\n",
            )
            .expect("reads");
        assert_eq!(usage.input, Some(7500));
        assert_eq!(usage.output, Some(4));
        assert!(usage.approximate);
    }

    #[test]
    fn json_counts_are_read_by_pointer() {
        let mut s = spec(Shape::Json, Stream::Stdout);
        s.input = Some(Keys::One("/usage/input_tokens".into()));
        s.output = Some(Keys::One("/usage/output_tokens".into()));
        let body = r#"{"result":"OK","usage":{"input_tokens":2,"output_tokens":4}}"#;
        let usage = s.read("v", "author", body, "").expect("reads");
        assert_eq!((usage.input, usage.output), (Some(2), Some(4)));
    }

    #[test]
    fn a_vendor_that_reports_nothing_produces_no_row() {
        // Distinguishable from a vendor that ran and spent nothing, which is
        // the difference a status line has to be able to show.
        let mut s = spec(Shape::Text, Stream::Stderr);
        s.total = Some(Keys::One("tokens used".into()));
        assert!(s.read("v", "author", "", "no accounting here\n").is_none());
    }

    #[test]
    fn a_marker_with_no_number_after_it_anywhere_is_not_a_zero() {
        let mut s = spec(Shape::Text, Stream::Stderr);
        s.total = Some(Keys::One("tokens used".into()));
        assert!(s.read("v", "author", "", "tokens used\n").is_none());
    }

    #[test]
    fn unparseable_json_is_absence_rather_than_a_crash() {
        let mut s = spec(Shape::Json, Stream::Stdout);
        s.total = Some(Keys::One("/usage/total".into()));
        assert!(s.read("v", "author", "not json at all", "").is_none());
    }

    #[test]
    fn several_counters_add_up_to_one_field() {
        // A vendor that reports fresh, cache-creating and cache-read input
        // separately is reporting three parts of one number.
        let mut s = spec(Shape::Json, Stream::Stdout);
        s.input = Some(Keys::Several(vec![
            "/usage/input_tokens".into(),
            "/usage/cache_read_input_tokens".into(),
            "/usage/missing_entirely".into(),
        ]));
        let body = r#"{"usage":{"input_tokens":10,"cache_read_input_tokens":10126}}"#;
        let usage = s.read("v", "author", body, "").expect("reads");
        assert_eq!(usage.input, Some(10_136));
    }

    #[test]
    fn counted_prefers_a_total_and_falls_back_to_the_split() {
        let split = TokenUsage {
            adapter: "v".into(),
            role: "author".into(),
            input: Some(10),
            output: Some(5),
            total: None,
            approximate: false,
        };
        assert_eq!(split.counted(), 15);
        let total = TokenUsage {
            total: Some(99),
            ..split.clone()
        };
        assert_eq!(total.counted(), 99);
    }
}
