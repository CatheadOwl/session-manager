use serde::{Deserialize, Serialize};

/// Result returned to the frontend.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForkTreeResult {
    pub roots: Vec<TreeNodeData>,
    pub total_sessions: u32,
    pub computed_from_cache: bool,
    pub duration_ms: u64,
}

/// A node in the fork tree, with nested children.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeNodeData {
    pub session_key: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_active_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_dir: Option<String>,
    pub user_hash_chain: Vec<String>,
    pub depth: u32,
    /// Index (0-based) into user_hash_chain where this node forks from its parent.
    /// For root nodes (depth=0), this is 0.
    pub forked_at_user: u32,
    /// The actual user text at the fork point (first differing user input).
    /// Only set when depth > 0 (non-root nodes).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fork_user_text: Option<String>,
    pub children: Vec<TreeNodeData>,
}

/// Per-file cached data inside fork-tree.json.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CachedFileData {
    pub(crate) session_key: String,
    pub(crate) source_path: String,
    pub(crate) title: String,
    pub(crate) summary: Option<String>,
    pub(crate) last_active_at: Option<i64>,
    pub(crate) project_dir: Option<String>,
    pub(crate) hash_chain: Vec<String>,
    /// Short user event previews, parallel to hash_chain.
    /// Stored so the fork point can be hinted in the tree without caching full prompts.
    #[serde(default)]
    pub(crate) user_texts: Vec<String>,
    /// Original (unfiltered) indices of events that passed the hash_events filter.
    /// `kept_indices[i]` = position of chain entry `i` in the raw user-event sequence.
    /// Used to map chain-space LCP positions back to message-level indices for UI jump.
    #[serde(default)]
    pub(crate) kept_indices: Vec<usize>,
    /// Provider-specific session ID this session forked from (e.g. Codex's forked_from_id).
    /// When set, this takes priority over heuristic hash-chain matching in parent detection.
    #[serde(default)]
    pub(crate) forked_from_id: Option<String>,
    /// UUID chain from user events, for stronger cross-session matching.
    /// Extracted from JSONL lines with a `uuid` field (e.g. Claude sessions).
    /// Empty for providers that don't carry UUIDs.
    #[serde(default)]
    pub(crate) uuid_chain: Vec<String>,
}

/// On-disk cache structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ForkTreeCache {
    pub(crate) version: u32,
    pub(crate) files: Vec<CachedFileData>,
}

pub(crate) const CACHE_VERSION: u32 = 7;
