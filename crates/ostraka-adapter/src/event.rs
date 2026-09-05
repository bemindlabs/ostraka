//! Turning a vendor's output into the four event kinds the runtime understands.
//!
//! Anything richer than these four stays vendor-specific and is preserved only
//! as the raw payload, so that normalization never silently discards evidence.

use crate::profile::EventFormat;
use ostraka_core::record::Event;

/// Reads one line of adapter output into a normalized event.
///
/// A line that is not valid JSON under `Jsonl` is not an error: it is recorded
/// as a message, because a vendor writing a warning to stdout should not abort
/// a run or vanish from the record.
pub fn normalize_line(format: EventFormat, line: &str) -> Option<Event> {
    if line.trim().is_empty() {
        return None;
    }
    match format {
        EventFormat::None => Some(Event::Message {
            text: line.to_string(),
            raw: None,
        }),
        EventFormat::Jsonl => Some(normalize_json_line(line)),
    }
}

fn normalize_json_line(line: &str) -> Event {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
        return Event::Message {
            text: line.to_string(),
            raw: None,
        };
    };

    let kind = value.get("type").and_then(|v| v.as_str()).unwrap_or("");
    match kind {
        "tool_use" => Event::ToolUse {
            name: value
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string(),
            raw: Some(value),
        },
        "error" => Event::Error {
            message: value
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("unspecified error")
                .to_string(),
            raw: Some(value),
        },
        _ => {
            let text = value
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or(line)
                .to_string();
            Event::Message {
                text,
                raw: Some(value),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_lines_produce_nothing() {
        assert!(normalize_line(EventFormat::Jsonl, "   ").is_none());
    }

    #[test]
    fn a_tool_use_keeps_its_raw_payload() {
        let event = normalize_line(
            EventFormat::Jsonl,
            r#"{"type":"tool_use","name":"edit","extra":{"path":"a.rs"}}"#,
        )
        .expect("an event");
        match event {
            Event::ToolUse { name, raw } => {
                assert_eq!(name, "edit");
                let raw = raw.expect("raw payload preserved");
                assert_eq!(raw["extra"]["path"], "a.rs");
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn malformed_json_is_recorded_not_dropped() {
        let event = normalize_line(EventFormat::Jsonl, "warning: not json").expect("an event");
        match event {
            Event::Message { text, raw } => {
                assert_eq!(text, "warning: not json");
                assert!(raw.is_none());
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }
}
