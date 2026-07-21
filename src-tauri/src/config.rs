use std::path::PathBuf;

pub fn get_home_dir() -> PathBuf {
    if let Ok(home) = std::env::var("SESSION_MANAGER_TEST_HOME") {
        let trimmed = home.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }

    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

pub fn get_claude_projects_dir() -> PathBuf {
    get_claude_config_dir().join("projects")
}

pub fn get_claude_projects_archived_dir() -> PathBuf {
    get_claude_config_dir().join("projects_archived")
}

pub fn get_gemini_dir() -> PathBuf {
    get_home_dir().join(".gemini")
}

pub fn get_gemini_archive_dir() -> PathBuf {
    get_gemini_dir().join("tmp_archived")
}

pub fn get_openclaw_dir() -> PathBuf {
    get_home_dir().join(".openclaw")
}

pub fn get_openclaw_agents_dir() -> PathBuf {
    get_openclaw_dir().join("agents")
}

pub fn get_openclaw_archive_dir() -> PathBuf {
    get_openclaw_dir().join("agents_archived")
}

pub fn get_codex_dir() -> PathBuf {
    get_home_dir().join(".codex")
}

pub fn get_codex_sessions_dir() -> PathBuf {
    get_codex_dir().join("sessions")
}

pub fn get_codex_archive_dir() -> PathBuf {
    get_codex_dir().join("archived_sessions")
}

pub fn get_hermes_dir() -> PathBuf {
    get_home_dir().join(".config").join("hermes")
}

pub fn get_hermes_sessions_dir() -> PathBuf {
    get_hermes_dir().join("sessions")
}

pub fn get_hermes_archive_dir() -> PathBuf {
    get_hermes_dir().join("archived_sessions")
}

pub fn get_qoder_cn_projects_dir() -> PathBuf {
    get_home_dir().join(".qoder-cn").join("projects")
}

pub fn get_qoder_projects_dir() -> PathBuf {
    get_home_dir().join(".qoder").join("projects")
}

pub fn get_opencode_base_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg).join("opencode");
        }
    }
    get_home_dir().join(".local/share/opencode")
}

pub fn get_opencode_archive_dir() -> PathBuf {
    get_opencode_base_dir().join("storage_archived")
}

pub fn get_claude_config_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("CLAUDE_CONFIG_DIR") {
        let trimmed = dir.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }

    get_home_dir().join(".claude")
}

pub fn get_app_metadata_path() -> Result<PathBuf, String> {
    Ok(get_home_dir().join(".session-manager").join("metadata.json"))
}

pub fn get_fork_tree_cache_path() -> Result<PathBuf, String> {
    Ok(get_home_dir()
        .join(".session-manager")
        .join("fork-tree.json"))
}

/// Global mutex for tests that modify environment variables.
/// All test modules must share this single lock to prevent parallel tests
/// from racing on `SESSION_MANAGER_TEST_HOME` / `CLAUDE_CONFIG_DIR`.
#[cfg(test)]
pub static TEST_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_config_dir_uses_override() {
        let _guard = TEST_ENV_LOCK.lock().expect("lock");
        std::env::set_var("CLAUDE_CONFIG_DIR", "/tmp/custom-claude");
        assert_eq!(get_claude_config_dir(), PathBuf::from("/tmp/custom-claude"));
        std::env::remove_var("CLAUDE_CONFIG_DIR");
    }
}
