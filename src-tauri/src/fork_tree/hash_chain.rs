use std::path::Path;

use sha2::{Digest, Sha256};

use super::types::CachedFileData;
use crate::session_manager;
use crate::session_manager::providers::ProviderRegistry;

/// Stored user-text previews are intentionally short: enough to hint at the fork
/// point in the tree view without keeping full prompts in the cache.
pub(crate) const USER_TEXT_PREVIEW_CHARS: usize = 64;

#[allow(dead_code)]
pub(crate) fn compute_file_data(registry: &ProviderRegistry, path: &Path) -> Result<CachedFileData, String> {
    // First parse session metadata — dispatches to the correct provider
    let meta = session_manager::parse_session_meta(registry, path)
        .ok_or_else(|| format!("Failed to parse session: {}", path.display()))?;

    // Get provider-specific user events (with UUIDs) for fork detection
    let provider = registry
        .get(&meta.provider_id)
        .map_err(|e| format!("{}: {}", e, path.display()))?;
    let events_with_uuid = provider.user_events_with_uuid(path)?;

    // Destructure into texts (for hashing) and uuids (for uuid-chain matching)
    let events: Vec<String> = events_with_uuid.iter().map(|(t, _)| t.clone()).collect();
    let uuid_chain: Vec<String> = events_with_uuid.into_iter().map(|(_, u)| u).collect();

    // If no actual UUIDs were extracted (all empty strings), clear the chain so
    // tree_builder can reliably detect uuid_chain availability vs. empty fallback.
    let uuid_chain = if uuid_chain.iter().any(|u| !u.is_empty()) {
        uuid_chain
    } else {
        vec![]
    };

    // Compute hash chain from user event texts
    let (hash_chain, user_texts, kept_indices) = hash_events(&events);

    Ok(CachedFileData {
        session_key: format!(
            "{}:{}:{}",
            meta.provider_id,
            meta.session_id,
            path.to_string_lossy()
        ),
        source_path: path.to_string_lossy().to_string(),
        title: meta.title.unwrap_or_else(|| {
            meta.session_id.chars().take(8).collect()
        }),
        summary: meta.summary,
        last_active_at: meta.last_active_at,
        project_dir: meta.project_dir,
        hash_chain,
        user_texts,
        kept_indices,
        forked_from_id: meta.forked_from_id,
        uuid_chain,
    })
}

/// Hash a list of user event texts into parallel hash chain and preview vectors.
/// Each text is truncated to 120 characters (char-based) before hashing,
/// matching the reference analysis.ps1 Substring(0, 120) behavior.
///
/// Messages matching filter heuristics (greetings, system instructions) are
/// excluded from the chain — they don't carry user intent for fork detection
/// and cause false positive LCP matches between unrelated sessions.
///
/// Returns (hash_chain, user_texts, kept_original_indices) with 1:1 positional
/// correspondence. `kept_original_indices[i]` is the position of chain entry `i`
/// in the original (unfiltered) event sequence, enabling downstream code to map
/// chain-space positions back to message-level indices for UI navigation.
pub(crate) fn hash_events(events: &[String]) -> (Vec<String>, Vec<String>, Vec<usize>) {
    let mut hashes = Vec::with_capacity(events.len());
    let mut texts = Vec::with_capacity(events.len());
    let mut kept_indices = Vec::with_capacity(events.len());
    for (idx, text) in events.iter().enumerate() {
        if is_filtered_text(text) {
            continue;
        }
        let key: String = text.chars().take(120).collect();
        hashes.push(sha256_first_8(&key));
        texts.push(preview_user_text(text));
        kept_indices.push(idx);
    }
    (hashes, texts, kept_indices)
}

fn preview_user_text(text: &str) -> String {
    let mut chars = text.chars();
    let preview: String = chars.by_ref().take(USER_TEXT_PREVIEW_CHARS).collect();
    if chars.next().is_some() {
        format!("{preview}...")
    } else {
        preview
    }
}

/// Check if a user event text should be excluded from hash chain computation.
/// Filtered messages are system instructions or greetings that don't represent
/// meaningful user intent for fork detection.
fn is_filtered_text(text: &str) -> bool {
    is_system_instruction(text) || is_short_greeting(text)
}

/// Detect messages that are purely XML-like system-generated instructions,
/// e.g. `<command-name>mcp</command-name>`.
/// Heuristic: starts with `<`, ends with `>`, and contains a closing tag.
fn is_system_instruction(text: &str) -> bool {
    let t = text.trim();
    t.starts_with('<') && t.ends_with('>') && t.contains("</")
}

/// Detect very short greeting-like messages that commonly cause
/// false positive LCP matches between unrelated sessions.
/// Only matches messages that are a greeting and little else (≤15 chars).
fn is_short_greeting(text: &str) -> bool {
    let t = text.trim().to_lowercase();
    if t.len() > 15 {
        return false;
    }
    matches!(
        t.as_str(),
        "hello" | "hi" | "hey" | "yo" | "你好" | "嗨" | "喂" | "您好"
    )
}

/// Compute an 8-character hex string from the first 4 bytes of SHA-256(input).
pub(crate) fn sha256_first_8(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    let result = hasher.finalize();
    format!(
        "{:02x}{:02x}{:02x}{:02x}",
        result[0], result[1], result[2], result[3]
    )
}
