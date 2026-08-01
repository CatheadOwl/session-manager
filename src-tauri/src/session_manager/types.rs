use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMeta {
    pub provider_id: String,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_active_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locator: Option<SessionLocator>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resume_command: Option<String>,
    /// Provider-specific session ID this session forked from (e.g. Codex's forked_from_id).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forked_from_id: Option<String>,
}

impl SessionMeta {
    pub fn debug_assert_file_locator_matches_source_path(&self) {
        if let (Some(source_path), Some(SessionLocator::File { path })) =
            (&self.source_path, &self.locator)
        {
            debug_assert_eq!(
                source_path, path,
                "file-backed SessionMeta locator must match source_path"
            );
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub cache_read_input_tokens: u64,
    pub output_tokens: u64,
}

impl TokenUsage {
    pub fn input_total(&self) -> u64 {
        self.input_tokens
            .saturating_add(self.cache_creation_input_tokens)
            .saturating_add(self.cache_read_input_tokens)
    }

    pub fn total(&self) -> u64 {
        self.input_total().saturating_add(self.output_tokens)
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CumulativeTokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
}

impl CumulativeTokenUsage {
    pub fn add_usage(&mut self, usage: TokenUsage) {
        self.input_tokens = self.input_tokens.saturating_add(usage.input_total());
        self.output_tokens = self.output_tokens.saturating_add(usage.output_tokens);
        self.total_tokens = self.input_tokens.saturating_add(self.output_tokens);
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallInfo {
    pub name: String,
    pub input: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolResultInfo {
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMessage {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ts: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cumulative_usage: Option<CumulativeTokenUsage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCallInfo>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_result: Option<ToolResultInfo>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QaPair {
    pub question_idx: usize,
    pub answer_idx: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionDetail {
    pub messages: Vec<SessionMessage>,
    pub qa_pairs: Vec<QaPair>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum SessionLocator {
    File {
        path: String,
    },
    Database {
        path: String,
        #[serde(rename = "recordId", alias = "record_id")]
        record_id: String,
    },
}

impl SessionLocator {
    pub fn file_path(&self) -> Result<&str, String> {
        match self {
            SessionLocator::File { path } => Ok(path),
            SessionLocator::Database { .. } => {
                Err("Database-backed sessions cannot be treated as filesystem paths".to_string())
            }
        }
    }

    #[cfg(test)]
    pub fn display_source_path(&self) -> &str {
        match self {
            SessionLocator::File { path } | SessionLocator::Database { path, .. } => path,
        }
    }

    #[cfg(test)]
    pub fn detail_key_part(&self) -> String {
        match self {
            SessionLocator::File { path } => format!("file:{path}"),
            SessionLocator::Database { path, record_id } => {
                format!("database:{path}:{record_id}")
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionHandle {
    pub provider_id: String,
    pub session_id: String,
    pub locator: SessionLocator,
}

impl SessionHandle {
    pub fn file_path(&self) -> Result<&str, String> {
        self.locator.file_path()
    }

    #[cfg(test)]
    pub fn display_source_path(&self) -> &str {
        self.locator.display_source_path()
    }

    #[cfg(test)]
    pub fn detail_key(&self) -> String {
        format!(
            "{}:{}:{}",
            self.provider_id,
            self.session_id,
            self.locator.detail_key_part()
        )
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionHandleRequest {
    pub provider_id: String,
    pub session_id: String,
    #[serde(default)]
    pub source_path: Option<String>,
    #[serde(default)]
    pub locator: Option<SessionLocator>,
}

impl SessionHandleRequest {
    pub fn into_handle(self) -> Result<SessionHandle, String> {
        let locator = match (self.locator, self.source_path) {
            (Some(locator), _) => locator,
            (None, Some(path)) => SessionLocator::File { path },
            (None, None) => {
                return Err("Session request requires either locator or sourcePath".to_string())
            }
        };

        Ok(SessionHandle {
            provider_id: self.provider_id,
            session_id: self.session_id,
            locator,
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteSessionRequest {
    pub provider_id: String,
    pub session_id: String,
    pub source_path: String,
    #[serde(default)]
    pub locator: Option<SessionLocator>,
}

impl DeleteSessionRequest {
    pub fn to_handle(&self) -> SessionHandle {
        SessionHandle {
            provider_id: self.provider_id.clone(),
            session_id: self.session_id.clone(),
            locator: self
                .locator
                .clone()
                .unwrap_or_else(|| SessionLocator::File {
                    path: self.source_path.clone(),
                }),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteSessionOutcome {
    pub provider_id: String,
    pub session_id: String,
    pub source_path: String,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub enum SessionScope {
    Active,
    Archived,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_meta(source_path: Option<String>, locator: Option<SessionLocator>) -> SessionMeta {
        SessionMeta {
            provider_id: "claude".to_string(),
            session_id: "session-1".to_string(),
            title: None,
            summary: None,
            project_dir: None,
            created_at: None,
            last_active_at: None,
            source_path,
            locator,
            resume_command: None,
            forked_from_id: None,
        }
    }

    #[test]
    fn file_locator_can_match_source_path() {
        base_meta(
            Some("/data/session.jsonl".to_string()),
            Some(SessionLocator::File {
                path: "/data/session.jsonl".to_string(),
            }),
        )
        .debug_assert_file_locator_matches_source_path();
    }

    #[test]
    fn database_locator_uses_camel_case_record_id_on_the_wire() {
        let locator = SessionLocator::Database {
            path: "/data/opencode.db".to_string(),
            record_id: "row-a".to_string(),
        };

        let value = serde_json::to_value(&locator).expect("serialize locator");

        assert_eq!(
            value,
            serde_json::json!({
                "kind": "database",
                "path": "/data/opencode.db",
                "recordId": "row-a",
            })
        );
    }

    #[test]
    fn database_locator_accepts_legacy_snake_case_record_id() {
        let locator: SessionLocator = serde_json::from_value(serde_json::json!({
            "kind": "database",
            "path": "/data/opencode.db",
            "record_id": "row-a",
        }))
        .expect("deserialize legacy locator");

        assert_eq!(
            locator,
            SessionLocator::Database {
                path: "/data/opencode.db".to_string(),
                record_id: "row-a".to_string(),
            }
        );
    }

    #[test]
    #[should_panic(expected = "file-backed SessionMeta locator must match source_path")]
    fn file_locator_mismatch_panics_in_debug_assertions() {
        base_meta(
            Some("/data/session.jsonl".to_string()),
            Some(SessionLocator::File {
                path: "/other/session.jsonl".to_string(),
            }),
        )
        .debug_assert_file_locator_matches_source_path();
    }
}
