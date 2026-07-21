use std::fs::File;
use std::io::{self, BufRead, BufReader, Seek, SeekFrom};
use std::path::Path;

use chrono::{DateTime, FixedOffset};
use serde_json::Value;

use crate::session_manager::types::{ToolCallInfo, ToolResultInfo};

// ─── Shared provider helpers ────────────────────────────────────────────────

/// Move a single session file (no sidecar) to a destination directory.
/// Creates the destination directory if it does not exist.
/// Used by providers that store sessions as a single file (Codex, Gemini, Hermes, OpenClaw).
pub fn move_single_file(source_path: &Path, dest_dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dest_dir)
        .map_err(|e| format!("Failed to create destination directory: {e}"))?;
    let file_name = source_path
        .file_name()
        .ok_or_else(|| "Source path has no file name".to_string())?;
    let dest_path = dest_dir.join(file_name);
    std::fs::rename(source_path, &dest_path)
        .map_err(|e| format!("Failed to move session file: {e}"))?;
    Ok(())
}

/// Infer a session ID from the file stem (e.g. "abc-123.jsonl" -> "abc-123").
pub fn infer_session_id_from_filename(path: &Path) -> Option<String> {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .map(|stem| stem.to_string())
}

/// Push a trimmed, non-empty, deduplicated chunk onto a Vec.
pub fn push_raw_chunk(chunks: &mut Vec<String>, value: Option<&str>) {
    let Some(value) = value else {
        return;
    };
    let trimmed = value.trim();
    if trimmed.is_empty() || chunks.iter().any(|chunk| chunk == trimmed) {
        return;
    }
    chunks.push(trimmed.to_string());
}

pub const TITLE_MAX_CHARS: usize = 80;

pub fn read_head_tail_lines(
    path: &Path,
    head_n: usize,
    tail_n: usize,
) -> io::Result<(Vec<String>, Vec<String>)> {
    let file = File::open(path)?;
    let file_len = file.metadata()?.len();

    if file_len < 16_384 {
        let reader = BufReader::new(file);
        let all: Vec<String> = reader.lines().map_while(Result::ok).collect();
        let head = all.iter().take(head_n).cloned().collect();
        let skip = all.len().saturating_sub(tail_n);
        let tail = all.into_iter().skip(skip).collect();
        return Ok((head, tail));
    }

    let reader = BufReader::new(file);
    let head: Vec<String> = reader.lines().take(head_n).map_while(Result::ok).collect();

    let seek_pos = file_len.saturating_sub(16_384);
    let mut file2 = File::open(path)?;
    file2.seek(SeekFrom::Start(seek_pos))?;
    let tail_reader = BufReader::new(file2);
    let all_tail: Vec<String> = tail_reader.lines().map_while(Result::ok).collect();

    let skip_first = if seek_pos > 0 { 1 } else { 0 };
    let usable: Vec<String> = all_tail.into_iter().skip(skip_first).collect();
    let skip = usable.len().saturating_sub(tail_n);
    let tail = usable.into_iter().skip(skip).collect();

    Ok((head, tail))
}

pub fn parse_timestamp_to_ms(value: &Value) -> Option<i64> {
    if let Some(n) = value.as_i64() {
        return Some(if n > 1_000_000_000_000 { n } else { n * 1000 });
    }
    if let Some(n) = value.as_f64() {
        let n = n as i64;
        return Some(if n > 1_000_000_000_000 { n } else { n * 1000 });
    }

    let raw = value.as_str()?;
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|dt: DateTime<FixedOffset>| dt.timestamp_millis())
}

pub fn extract_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.to_string(),
        Value::Array(items) => items
            .iter()
            .filter_map(extract_text_from_item)
            .filter(|text| !text.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Object(map) => map
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        _ => String::new(),
    }
}

fn extract_text_from_item(item: &Value) -> Option<String> {
    let item_type = item.get("type").and_then(Value::as_str).unwrap_or("");

    if item_type == "tool_use" {
        let name = item
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        return Some(format!("[Tool: {name}]"));
    }

    if item_type == "tool_result" {
        // Tool result content is extracted separately via extract_tool_results;
        // only a placeholder remains in the main text content.
        return Some("[Tool Result]".to_string());
    }

    if let Some(text) = item.get("text").and_then(|v| v.as_str()) {
        return Some(text.to_string());
    }

    if let Some(text) = item.get("input_text").and_then(|v| v.as_str()) {
        return Some(text.to_string());
    }

    if let Some(text) = item.get("output_text").and_then(|v| v.as_str()) {
        return Some(text.to_string());
    }

    if let Some(content) = item.get("content") {
        let text = extract_text(content);
        if !text.is_empty() {
            return Some(text);
        }
    }

    None
}

pub fn truncate_summary(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }

    let mut result = trimmed.chars().take(max_chars).collect::<String>();
    result.push_str("...");
    result
}

pub fn path_basename(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let normalized = trimmed.trim_end_matches(['/', '\\']);
    let last = normalized
        .split(['/', '\\'])
        .next_back()
        .filter(|segment| !segment.is_empty())?;
    Some(last.to_string())
}

pub const TOOL_CALL_INPUT_MAX_CHARS: usize = 1000;

/// Serialize tool `input` value for human-readable preview.
/// Returns empty string if the JSON is too long (not useful as a compact preview).
pub(crate) fn truncate_tool_input(value: &Value) -> String {
    let json = serde_json::to_string(value).unwrap_or_default();
    let len = json.chars().count();
    if len <= TOOL_CALL_INPUT_MAX_CHARS {
        return json;
    }
    format!("[exceed limit {} chars]", len)
}

/// Extract tool call info from a JSON content block (typically `message.content` array).
/// Returns empty Vec if content is not an array or contains no tool_use items.
pub fn extract_tool_calls(content: &Value) -> Vec<ToolCallInfo> {
    let items = match content {
        Value::Array(items) => items,
        _ => return Vec::new(),
    };
    items
        .iter()
        .filter_map(|item| {
            let item_type = item.get("type").and_then(Value::as_str)?;
            if item_type != "tool_use" {
                return None;
            }
            let name = item
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string();
            let input = item
                .get("input")
                .map(truncate_tool_input)
                .unwrap_or_default();
            let call_id = item
                .get("id")
                .and_then(Value::as_str)
                .map(|s| s.to_string());
            Some(ToolCallInfo {
                name,
                input,
                call_id,
            })
        })
        .collect()
}

pub const TOOL_RESULT_MAX_CHARS: usize = 5000;

/// Extract tool result content from a `message.content` array.
/// Returns `None` if no `tool_result` items are found or their content is empty.
pub fn extract_tool_results(content: &Value) -> Option<ToolResultInfo> {
    let items = match content {
        Value::Array(items) => items,
        _ => return None,
    };
    let mut parts: Vec<String> = Vec::new();
    let mut call_id: Option<String> = None;
    for item in items {
        if item.get("type").and_then(Value::as_str) != Some("tool_result") {
            continue;
        }
        // capture tool_use_id from the first tool_result item
        if call_id.is_none() {
            call_id = item
                .get("tool_use_id")
                .and_then(Value::as_str)
                .map(|s| s.to_string());
        }
        if let Some(inner) = item.get("content") {
            let text = extract_text(inner);
            if !text.trim().is_empty() {
                parts.push(text);
            }
        }
    }
    if parts.is_empty() {
        return None;
    }
    let full = parts.join("\n");
    let len = full.chars().count();
    if len > TOOL_RESULT_MAX_CHARS {
        // Content too long for this compact view — return indicator instead of
        // truncated/incomplete data (e.g. broken JSON).
        return Some(ToolResultInfo {
            content: format!("[exceed limit {} chars]", len),
            call_id,
        });
    }
    Some(ToolResultInfo {
        content: full,
        call_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_timestamp_to_ms_supports_integers_and_rfc3339() {
        assert_eq!(
            parse_timestamp_to_ms(&json!(1_771_061_953_033_i64)),
            Some(1_771_061_953_033)
        );
        assert_eq!(
            parse_timestamp_to_ms(&json!(1_771_061_953_i64)),
            Some(1_771_061_953_000)
        );
        assert_eq!(
            parse_timestamp_to_ms(&json!("1970-01-01T00:00:01Z")),
            Some(1_000)
        );
    }

    #[test]
    fn extract_text_supports_tool_use_and_tool_result() {
        let value = json!([
            {"type": "text", "text": "hello"},
            {"type": "tool_use", "name": "Read"},
            {"type": "tool_result", "content": "done"}
        ]);

        let text = extract_text(&value);
        assert!(text.contains("hello"));
        assert!(text.contains("[Tool: Read]"));
        assert!(text.contains("[Tool Result]"));
    }

    #[test]
    fn extract_tool_calls_parses_tool_use_blocks() {
        let value = json!([
            {"type": "text", "text": "hello"},
            {"type": "tool_use", "name": "Read", "input": {"file_path": "/tmp/test.txt"}},
            {"type": "tool_use", "name": "Edit", "input": {"old_string": "a", "new_string": "b"}, "id": "call-1"},
            {"type": "tool_result", "content": "done"}
        ]);

        let calls = extract_tool_calls(&value);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "Read");
        assert!(calls[0].input.contains("file_path"));
        assert_eq!(calls[0].call_id, None);
        assert_eq!(calls[1].name, "Edit");
        assert!(calls[1].input.contains("old_string"));
        assert_eq!(calls[1].call_id.as_deref(), Some("call-1"));
    }

    #[test]
    fn extract_tool_calls_handles_empty_and_non_array() {
        // Non-array (object) → empty
        let calls = extract_tool_calls(&json!({"type": "text", "text": "hi"}));
        assert!(calls.is_empty());

        // Empty array → empty
        let calls = extract_tool_calls(&json!([]));
        assert!(calls.is_empty());

        // Null → empty (match arm falls through to non-array)
        let calls = extract_tool_calls(&serde_json::Value::Null);
        assert!(calls.is_empty());
    }

    #[test]
    fn extract_tool_calls_skips_non_tool_items() {
        let value = json!([
            {"type": "text", "text": "hello"},
            {"type": "tool_result", "content": "done"}
        ]);

        let calls = extract_tool_calls(&value);
        assert!(calls.is_empty());
    }

    #[test]
    fn truncate_tool_input_skips_long_input() {
        let long_str = "x".repeat(1200); // > TOOL_CALL_INPUT_MAX_CHARS (1000)
        let value = json!({"data": long_str});
        let result = truncate_tool_input(&value);
        assert!(result.starts_with("[exceed limit"), "got: {result}");
    }

    #[test]
    fn truncate_tool_input_keeps_short_json() {
        let value = json!({"key": "short"});
        let result = truncate_tool_input(&value);
        assert!(result.contains("short"));
        assert!(!result.ends_with("..."));
    }

    #[test]
    fn extract_tool_results_parses_tool_result_blocks() {
        let value = json!([
            {"type": "text", "text": "hello"},
            {"type": "tool_result", "content": [{"type": "text", "text": "file content\nline 2"}]},
            {"type": "tool_use", "name": "Read"}
        ]);

        let result = extract_tool_results(&value);
        assert!(result.is_some());
        let info = result.unwrap();
        assert!(info.content.contains("file content"));
        assert!(info.content.contains("line 2"));
    }

    #[test]
    fn extract_tool_results_returns_none_for_non_array() {
        assert!(extract_tool_results(&json!("string")).is_none());
        assert!(extract_tool_results(&json!({"key": "val"})).is_none());
        assert!(extract_tool_results(&serde_json::Value::Null).is_none());
    }

    #[test]
    fn extract_tool_results_returns_none_when_no_tool_result_items() {
        let value = json!([
            {"type": "text", "text": "hello"},
            {"type": "tool_use", "name": "Read"}
        ]);
        assert!(extract_tool_results(&value).is_none());
    }

    #[test]
    fn extract_tool_results_skips_long_content() {
        let long = "x".repeat(5500); // > TOOL_RESULT_MAX_CHARS (5000)
        let value = json!([
            {"type": "tool_result", "content": [{"type": "text", "text": long}]}
        ]);
        let result = extract_tool_results(&value);
        assert!(result.is_some());
        assert!(result.unwrap().content.starts_with("[exceed limit"));
    }

    #[test]
    fn extract_text_replaces_tool_result_with_placeholder() {
        let value = json!([
            {"type": "text", "text": "hello"},
            {"type": "tool_result", "content": [{"type": "text", "text": "file content"}]}
        ]);

        let text = extract_text(&value);
        assert!(text.contains("hello"));
        assert!(text.contains("[Tool Result]"));
        assert!(
            !text.contains("file content"),
            "actual content should not be in extracted text"
        );
    }
}
