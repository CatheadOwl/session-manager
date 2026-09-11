//! Settings core: a hand-editable, layered `settings.json`.
//!
//! The file lives at `~/.session-manager/settings.json` and is a sparse
//! override layer over in-code defaults. It is a public contract: users may
//! hand-edit it before any settings UI exists, so loading is lenient (warn,
//! never fail startup) and programmatic saves preserve unknown keys verbatim.
//! This module mirrors `MetadataManager`'s shape (Mutex store + PathBuf) and
//! is deliberately Tauri-free — event emission for `settings-changed` is the
//! caller's job (see `commands/settings.rs`).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::sync::Mutex;

/// Current on-disk schema version (migration chain anchor).
pub const SETTINGS_VERSION: u64 = 1;

/// One `sources[]` entry: a kind-discriminated union. `kind`
/// defaults to `"local"` when absent, so legacy files without a `kind`
/// field parse unchanged
/// (zero migration, no version bump). Serialization omits `kind` for local
/// entries — a local entry's minimal shape is `{ path, enabled }`;
/// ssh entries always carry `"kind": "ssh"`.
#[derive(Clone, Debug, PartialEq)]
pub enum SourceEntry {
    /// Local extra scan root:
    /// `path` points at an ALTERNATE HOME — every provider's standard
    /// root is discovered under it via the shared home-relative
    /// derivation (same model as a remote machine). There is NO
    /// per-entry provider field anymore; a legacy
    /// `provider` key from an old file is preserved in `extra`,
    /// ignored. `id` is optional for local entries.
    Local(LocalSource),
    /// SSH remote source. Consumed by the
    /// remote scan line; the local overlay skips it. Unknown fields are
    /// preserved verbatim through saves (forward compatibility).
    Ssh(SshSource),
}

impl SourceEntry {
    pub fn is_enabled(&self) -> bool {
        match self {
            SourceEntry::Local(l) => l.enabled,
            SourceEntry::Ssh(s) => s.enabled,
        }
    }

    /// Stable entry id when present (ssh: always; local: optional). Ids
    /// share one namespace across kinds within a file.
    pub fn id(&self) -> Option<&str> {
        match self {
            SourceEntry::Local(l) => l.id.as_deref(),
            SourceEntry::Ssh(s) => Some(s.id.as_str()),
        }
    }
}

/// Local `sources[]` payload: `{ path, enabled, id? }` — the
/// path is a HOME-shaped root, providers are auto-discovered under it,
/// so no provider field exists. Unknown fields are preserved verbatim
/// through saves (`extra`): a legacy `provider` key from an old
/// file round-trips instead of dropping data, and the loader warns that
/// the semantics changed (no file rewriting).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct LocalSource {
    pub path: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// SSH auth block: tagged by
/// `mode`, camelCase `keyPath`. Three modes:
/// - `agent` — ssh-agent identities only;
/// - `key` — explicit key file (fallback after the agent pass);
/// - `sshConfig` — live reference to a `~/.ssh/config` Host alias;
///   the entry's `host`/`user`/`port` fields are placeholders that
///   the connect layer overrides from the resolved `Host` block
///   (agent identities first, then the block's IdentityFile).
///
/// Wire note: the enum-level `rename_all = "lowercase"` would render
/// `SshConfig` as "sshconfig"; the explicit `#[serde(rename)]` below
/// pins the camelCase wire tag.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "mode", rename_all = "lowercase")]
pub enum SourceAuth {
    Agent,
    Key {
        #[serde(rename = "keyPath")]
        key_path: String,
    },
    #[serde(rename = "sshConfig")]
    SshConfig {
        alias: String,
    },
}

fn default_port() -> u16 {
    22
}

/// SSH `sources[]` payload ("remote source = another
/// machine" — minimal shape). `id`/`host` are required by the loader
/// (warn + skip when missing); `user` and `auth` are required by the
/// shape (a missing field fails entry parse → warn + skip, same net
/// behavior). There is NO `root`/`providerHint` field anymore: the
/// remote scan derives its roots from each provider's `roots()`, and
/// probing/heal is gone. `extra` preserves unknown fields verbatim
/// through saves — a legacy `root` or `providerHint` key in an old file
/// is swallowed here (forward compatibility).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SshSource {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    pub user: String,
    pub auth: SourceAuth,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl Serialize for SourceEntry {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::Error as _;
        match self {
            // No `kind` on the wire for local entries: keeps saved files in
            // the minimal historical shape (kind defaults to "local").
            SourceEntry::Local(l) => l.serialize(serializer),
            SourceEntry::Ssh(s) => {
                let mut value = serde_json::to_value(s).map_err(S::Error::custom)?;
                if let Value::Object(map) = &mut value {
                    map.insert("kind".to_string(), Value::String("ssh".to_string()));
                }
                value.serialize(serializer)
            }
        }
    }
}

impl<'de> Deserialize<'de> for SourceEntry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error as _;
        let value = Value::deserialize(deserializer)?;
        // Kind-first discrimination; absent kind = "local".
        let kind = value.get("kind").and_then(Value::as_str).unwrap_or("local");
        match kind {
            "local" => serde_json::from_value(value)
                .map(SourceEntry::Local)
                .map_err(D::Error::custom),
            "ssh" => {
                // Strip the tag so the flatten catch-all doesn't capture it
                // (it is re-emitted by Serialize, not stored in `extra`).
                let mut value = value;
                if let Value::Object(map) = &mut value {
                    map.remove("kind");
                }
                serde_json::from_value(value)
                    .map(SourceEntry::Ssh)
                    .map_err(D::Error::custom)
            }
            other => Err(D::Error::custom(format!(
                "unknown sources entry kind `{other}`"
            ))),
        }
    }
}

fn default_true() -> bool {
    true
}

/// Externally-tagged setting value: `{"bool": true}`, `{"stringList": [...]}`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum SettingValue {
    #[serde(rename = "bool")]
    Bool(bool),
    #[serde(rename = "string")]
    Str(String),
    #[serde(rename = "stringList")]
    StringList(Vec<String>),
    #[serde(rename = "sourceList")]
    SourceList(Vec<SourceEntry>),
}

impl SettingValue {
    pub fn type_name(&self) -> &'static str {
        match self {
            SettingValue::Bool(_) => "bool",
            SettingValue::Str(_) => "string",
            SettingValue::StringList(_) => "stringList",
            SettingValue::SourceList(_) => "sourceList",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingType {
    #[serde(rename = "bool")]
    Bool,
    #[serde(rename = "string")]
    Str,
    #[serde(rename = "stringList")]
    StringList,
    #[serde(rename = "sourceList")]
    SourceList,
}

impl SettingType {
    fn matches(&self, value: &SettingValue) -> bool {
        self.as_str() == value.type_name()
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            SettingType::Bool => "bool",
            SettingType::Str => "string",
            SettingType::StringList => "stringList",
            SettingType::SourceList => "sourceList",
        }
    }
}

/// Descriptor for the future settings UI. Presentation (labels/order/i18n)
/// stays on the TS side; this carries only key/type/default/group.
#[derive(Serialize, Clone, Debug)]
pub struct SettingDefinition {
    pub key: &'static str,
    #[serde(rename = "type")]
    pub setting_type: SettingType,
    pub default: SettingValue,
    pub group: &'static str,
}

/// Defaults SSOT is this table, never the file.
pub const SETTINGS: &[SettingDefinition] = &[
    SettingDefinition {
        key: "update.autoCheck",
        setting_type: SettingType::Bool,
        default: SettingValue::Bool(true),
        group: "update",
    },
    SettingDefinition {
        key: "sources",
        setting_type: SettingType::SourceList,
        default: SettingValue::SourceList(Vec::new()),
        group: "sources",
    },
];

/// Merged view served over IPC: descriptors + effective values.
#[derive(Serialize, Clone, Debug)]
pub struct SettingsSnapshot {
    pub version: u64,
    pub descriptors: Vec<SettingDefinition>,
    pub values: HashMap<String, SettingValue>,
}

/// Versioned migration chain. Currently a no-op v1→v1; future steps read
/// `raw["version"]`, mutate `raw` in place, and return applied step names.
pub fn migrate(raw: &mut Value) -> Vec<String> {
    // No migrations exist yet — v1 is the current version.
    let _ = raw;
    Vec::new()
}

#[derive(Default)]
struct SettingsStore {
    /// Non-default known-key values (file overrides + programmatic sets).
    overrides: HashMap<String, SettingValue>,
    /// Unknown top-level keys, preserved verbatim on save.
    unknown_top: Map<String, Value>,
    /// Unknown fields inside the `update` section, preserved verbatim on save.
    unknown_update: Map<String, Value>,
    /// D7: the loaded file contained comments (strip produced a diff).
    had_comments: bool,
    /// D7: the one-time "comments will be dropped" warning has been logged.
    comments_warned: bool,
}

/// Mutex in-memory store + path, mirroring `MetadataManager`. No Tauri dep —
/// the CLI adapter can consume it too.
pub struct SettingsManager {
    store: Mutex<SettingsStore>,
    path: PathBuf,
}

impl SettingsManager {
    /// Lenient load (see the lenient-load invariants below). Never fails and
    /// never writes: a missing or broken file yields defaults, and the broken
    /// file is left untouched until an explicit `set_value`.
    pub fn new(path: PathBuf) -> Self {
        let mut store = SettingsStore::default();

        if let Ok(raw) = fs::read_to_string(&path) {
            let stripped = {
                let mut out = String::new();
                let mut reader = json_comments::StripComments::new(raw.as_bytes());
                match reader.read_to_string(&mut out) {
                    Ok(_) => Some(out),
                    Err(_) => None,
                }
            };
            match stripped {
                None => log::warn!(
                    "settings: unparseable file at {} — using defaults; file left untouched",
                    path.display()
                ),
                Some(stripped) => {
                    store.had_comments = stripped != raw;
                    match serde_json::from_str::<Value>(&stripped) {
                        Ok(mut value) => {
                            let migrations = migrate(&mut value);
                            for step in migrations {
                                log::info!("settings: applied migration step {step}");
                            }
                            match value {
                                Value::Object(map) => Self::extract(map, &mut store),
                                other => log::warn!(
                                    "settings: root is {} (expected object) at {} — using defaults; file left untouched",
                                    json_kind(&other),
                                    path.display()
                                ),
                            }
                        }
                        Err(e) => log::warn!(
                            "settings: failed to parse {} ({e}) — using defaults; file left untouched",
                            path.display()
                        ),
                    }
                }
            }
        }

        Self {
            store: Mutex::new(store),
            path,
        }
    }

    /// Split parsed top-level fields into known overrides, preserved unknown
    /// keys, and dropped wrong-typed known keys (warn + fallback to default).
    fn extract(map: Map<String, Value>, store: &mut SettingsStore) {
        for (key, value) in map {
            match key.as_str() {
                "version" => {
                    if !value.is_u64() {
                        log::warn!(
                            "settings: wrong-typed `version` ({}) — dropped",
                            json_kind(&value)
                        );
                    }
                }
                "update" => match value {
                    Value::Object(fields) => {
                        for (field, field_value) in fields {
                            if field == "autoCheck" {
                                match field_value.as_bool() {
                                    Some(b) => {
                                        // Sparse: only remember non-default values.
                                        if SettingValue::Bool(b) != default_of("update.autoCheck") {
                                            store
                                                .overrides
                                                .insert("update.autoCheck".to_string(), SettingValue::Bool(b));
                                        }
                                    }
                                    None => log::warn!(
                                        "settings: wrong-typed `update.autoCheck` ({}) — dropped, falling back to default",
                                        json_kind(&field_value)
                                    ),
                                }
                            } else {
                                log::warn!(
                                    "settings: unknown field `update.{field}` — preserved for round-trip"
                                );
                                store.unknown_update.insert(field, field_value);
                            }
                        }
                    }
                    other => log::warn!(
                        "settings: wrong-typed `update` ({}) — dropped, falling back to defaults",
                        json_kind(&other)
                    ),
                },
                "sources" => match parse_sources(&value) {
                    Some(list) => {
                        if SettingValue::SourceList(list.clone()) != default_of("sources") {
                            store
                                .overrides
                                .insert("sources".to_string(), SettingValue::SourceList(list));
                        }
                    }
                    None => log::warn!(
                        "settings: wrong-typed `sources` ({}) — dropped, falling back to default",
                        json_kind(&value)
                    ),
                },
                _ => {
                    store.unknown_top.insert(key.clone(), value.clone());
                    log::warn!("settings: unknown key `{key}` — preserved for round-trip");
                }
            }
        }
    }

    /// Merged view: defaults overlaid by non-default overrides.
    pub fn get(&self) -> SettingsSnapshot {
        let store = self.store.lock().unwrap();
        SettingsSnapshot {
            version: SETTINGS_VERSION,
            descriptors: SETTINGS.to_vec(),
            values: merged_values(&store),
        }
    }

    /// Per-key read for consumers. `None` for unknown keys.
    // No production caller yet: the IPC layer reads the full `get()` snapshot,
    // so today only tests call this. Kept as the per-key contract for future
    // consumers (CLI adapter).
    #[allow(dead_code)]
    pub fn get_value(&self, key: &str) -> Option<SettingValue> {
        let store = self.store.lock().unwrap();
        merged_values(&store).remove(key)
    }

    /// Per-key write. Validates the key exists and the type matches; performs
    /// a sparse atomic save; returns the changed-key list so the caller (IPC
    /// command layer) can emit `settings-changed` — this manager stays
    /// Tauri-free.
    pub fn set_value(&self, key: &str, value: SettingValue) -> Result<Vec<String>, String> {
        let def = SETTINGS
            .iter()
            .find(|d| d.key == key)
            .ok_or_else(|| format!("Unknown setting key: {key}"))?;
        if !def.setting_type.matches(&value) {
            return Err(format!(
                "Type mismatch for `{key}`: expected {}, got {}",
                def.setting_type.as_str(),
                value.type_name()
            ));
        }

        let mut store = self.store.lock().unwrap();
        if value == def.default {
            // Sparse: setting back to the default removes the override.
            store.overrides.remove(key);
        } else {
            store.overrides.insert(key.to_string(), value);
        }
        self.save(&mut store)?;
        Ok(vec![key.to_string()])
    }

    /// Scan-layer accessor (Phase B): extra scan roots, disabled entries skipped.
    pub fn enabled_sources(&self) -> Vec<SourceEntry> {
        let store = self.store.lock().unwrap();
        match store
            .overrides
            .get("sources")
            .cloned()
            .unwrap_or_else(|| default_of("sources"))
        {
            SettingValue::SourceList(list) => list.into_iter().filter(|s| s.is_enabled()).collect(),
            _ => Vec::new(),
        }
    }

    /// Sparse pretty-JSON atomic write: keys equal to defaults are omitted
    /// (except `version`), unknown keys are re-serialized verbatim.
    fn save(&self, store: &mut SettingsStore) -> Result<(), String> {
        // D7: one-time warning before the first programmatic save of a file
        // that contained comments (they will be dropped).
        if store.had_comments && !store.comments_warned {
            log::warn!(
                "settings: {} contains comments; this programmatic save will drop them",
                self.path.display()
            );
            store.comments_warned = true;
        }

        let mut root = Map::new();
        root.insert("version".to_string(), Value::from(SETTINGS_VERSION));

        // Unknown top-level keys round-trip verbatim.
        for (key, value) in &store.unknown_top {
            root.insert(key.clone(), value.clone());
        }

        // Known dotted keys nest (e.g. `update.autoCheck` → update.autoCheck);
        // a section is emitted when it has an override or preserved unknowns.
        let mut sections: HashMap<&str, Map<String, Value>> = HashMap::new();
        for def in SETTINGS {
            let (section, field) = match def.key.split_once('.') {
                Some((s, f)) => (s, f),
                None => {
                    if let Some(value) = store.overrides.get(def.key) {
                        root.insert(def.key.to_string(), setting_value_to_json(value));
                    }
                    continue;
                }
            };
            let entry = sections.entry(section).or_default();
            if let Some(value) = store.overrides.get(def.key) {
                entry.insert(field.to_string(), setting_value_to_json(value));
            }
        }
        for (field, value) in &store.unknown_update {
            sections
                .entry("update")
                .or_default()
                .insert(field.clone(), value.clone());
        }
        for (section, mut fields) in sections {
            if !fields.is_empty() {
                root.insert(
                    section.to_string(),
                    Value::Object(std::mem::take(&mut fields)),
                );
            }
        }

        let json = serde_json::to_string_pretty(&Value::Object(root))
            .map_err(|e| format!("Failed to serialize settings: {e}"))?;

        // Atomic write, MetadataManager-invariant style: parent dir ensured,
        // NamedTempFile in the same dir + persist.
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create settings dir: {e}"))?;
        }
        let dir = self.path.parent().unwrap_or(&self.path);
        let mut tmp = tempfile::NamedTempFile::new_in(dir)
            .map_err(|e| format!("Failed to create temp file: {e}"))?;
        use std::io::Write;
        tmp.write_all(json.as_bytes())
            .map_err(|e| format!("Failed to write settings: {e}"))?;
        tmp.persist(&self.path)
            .map_err(|e| format!("Failed to persist settings: {e}"))?;
        Ok(())
    }
}

fn merged_values(store: &SettingsStore) -> HashMap<String, SettingValue> {
    SETTINGS
        .iter()
        .map(|def| {
            (
                def.key.to_string(),
                store
                    .overrides
                    .get(def.key)
                    .cloned()
                    .unwrap_or_else(|| def.default.clone()),
            )
        })
        .collect()
}

fn default_of(key: &str) -> SettingValue {
    SETTINGS
        .iter()
        .find(|d| d.key == key)
        .map(|d| d.default.clone())
        .expect("known setting key")
}

/// Kind-first lenient loader: each entry is parsed
/// independently and a bad entry is warned + skipped WITHOUT dropping the
/// rest of the list. Per-kind rules fall out of the typed payload parse:
/// - unknown `kind` → error → warn + skip that entry only;
/// - `local` missing `path` (or wrong-typed path/enabled) → skip;
/// - a local entry still carrying the LEGACY `provider` key (no
///   longer part of the contract) → tolerated, preserved in `extra`,
///   and WARNED: the semantics changed (path must now point at a
///   home-shaped root) but the file is never rewritten;
/// - `ssh` missing `id`/`host` (or `user`/`auth`) → skip;
/// - ssh extra unknown fields (e.g. a stray `provider`, or the legacy
///   `root`/`providerHint` keys no longer in the contract) → tolerated
///   and preserved;
/// - duplicate `id` across the file (ssh AND local share the namespace)
///   → warn + skip the LATER entry.
fn parse_sources(value: &Value) -> Option<Vec<SourceEntry>> {
    let array = value.as_array()?;
    let mut list = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::new();
    for entry in array {
        match serde_json::from_value::<SourceEntry>(entry.clone()) {
            Ok(parsed) => {
                if let SourceEntry::Local(l) = &parsed {
                    if l.extra.contains_key("provider") {
                        log::warn!(
                            "settings: local source `{}` carries a legacy `provider` key — the path must point at a HOME-shaped root and providers are auto-discovered; the key is preserved but ignored",
                            l.path
                        );
                    }
                }
                if let Some(id) = parsed.id() {
                    if !seen_ids.insert(id.to_string()) {
                        log::warn!(
                            "settings: duplicate sources id `{id}` — later entry skipped: {entry}"
                        );
                        continue;
                    }
                }
                list.push(parsed);
            }
            Err(err) => {
                log::warn!("settings: sources entry skipped ({err}): {entry}");
            }
        }
    }
    Some(list)
}

fn setting_value_to_json(value: &SettingValue) -> Value {
    match value {
        SettingValue::Bool(b) => Value::from(*b),
        SettingValue::Str(s) => Value::from(s.clone()),
        SettingValue::StringList(list) => serde_json::to_value(list).expect("string list"),
        SettingValue::SourceList(list) => serde_json::to_value(list).expect("source list"),
    }
}

fn json_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn settings_path(dir: &std::path::Path) -> PathBuf {
        dir.join(".session-manager").join("settings.json")
    }

    fn write_settings(dir: &std::path::Path, content: &str) -> PathBuf {
        let path = settings_path(dir);
        std::fs::create_dir_all(path.parent().unwrap()).expect("create dir");
        std::fs::write(&path, content).expect("write settings");
        path
    }

    #[test]
    fn missing_file_loads_defaults() {
        let dir = tempdir().expect("tempdir");
        let manager = SettingsManager::new(settings_path(dir.path()));
        let snapshot = manager.get();
        assert_eq!(
            snapshot.values["update.autoCheck"],
            SettingValue::Bool(true)
        );
        assert_eq!(
            snapshot.values["sources"],
            SettingValue::SourceList(Vec::new())
        );
        // Never written automatically at startup.
        assert!(!settings_path(dir.path()).exists());
    }

    #[test]
    fn commented_file_parses() {
        let dir = tempdir().expect("tempdir");
        let path = write_settings(
            dir.path(),
            "{\n  // disable auto update\n  \"version\": 1,\n  \"update\": { \"autoCheck\": false } /* block too */\n}\n",
        );
        let manager = SettingsManager::new(path);
        assert_eq!(
            manager.get_value("update.autoCheck"),
            Some(SettingValue::Bool(false))
        );
    }

    #[test]
    fn wrong_typed_known_key_dropped_with_default_fallback() {
        let dir = tempdir().expect("tempdir");
        let path = write_settings(
            dir.path(),
            "{\"update\": { \"autoCheck\": \"yes\" }, \"sources\": \"not-a-list\"}",
        );
        let manager = SettingsManager::new(path);
        assert_eq!(
            manager.get_value("update.autoCheck"),
            Some(SettingValue::Bool(true))
        );
        assert_eq!(
            manager.get_value("sources"),
            Some(SettingValue::SourceList(Vec::new()))
        );
    }

    #[test]
    fn unparseable_file_tolerated_and_left_untouched() {
        let dir = tempdir().expect("tempdir");
        let broken = "{ this is not json !!";
        let path = write_settings(dir.path(), broken);
        let manager = SettingsManager::new(path.clone());
        assert_eq!(
            manager.get_value("update.autoCheck"),
            Some(SettingValue::Bool(true))
        );
        // Not deleted, not rewritten before an explicit set.
        assert_eq!(std::fs::read_to_string(&path).expect("read"), broken);
    }

    #[test]
    fn unknown_keys_preserved_through_save() {
        let dir = tempdir().expect("tempdir");
        let path = write_settings(
            dir.path(),
            "{\"futureKey\": {\"nested\": [1, 2, {\"deep\": true}]}, \"update\": {\"autoCheck\": false, \"futureField\": 42}}",
        );
        let manager = SettingsManager::new(path.clone());
        manager
            .set_value("update.autoCheck", SettingValue::Bool(false))
            .expect("set");

        let saved: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).expect("read")).expect("parse");
        // Unknown top-level key survives byte-for-byte (exact value subtree).
        assert_eq!(
            saved["futureKey"],
            serde_json::json!({"nested": [1, 2, {"deep": true}]})
        );
        // Unknown field inside a known section survives too.
        assert_eq!(saved["update"]["futureField"], serde_json::json!(42));
        // And the set value is there.
        assert_eq!(saved["update"]["autoCheck"], serde_json::json!(false));
    }

    #[test]
    fn unparseable_file_rewritten_only_on_explicit_set() {
        let dir = tempdir().expect("tempdir");
        let path = write_settings(dir.path(), "{ broken");
        let manager = SettingsManager::new(path.clone());
        manager
            .set_value("update.autoCheck", SettingValue::Bool(false))
            .expect("set");
        let saved: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).expect("read")).expect("parse");
        assert_eq!(saved["version"], serde_json::json!(1));
        assert_eq!(saved["update"]["autoCheck"], serde_json::json!(false));
    }

    #[test]
    fn sparse_write_omits_default_equal_keys() {
        let dir = tempdir().expect("tempdir");
        let path = write_settings(dir.path(), "{\"update\": {\"autoCheck\": false}}");
        let manager = SettingsManager::new(path.clone());

        // Setting back to the default removes the override from the file.
        manager
            .set_value("update.autoCheck", SettingValue::Bool(true))
            .expect("set");
        let saved: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).expect("read")).expect("parse");
        assert!(
            saved.get("update").is_none(),
            "default-equal key must be omitted"
        );
    }

    #[test]
    fn version_always_written() {
        let dir = tempdir().expect("tempdir");
        let path = settings_path(dir.path());
        let manager = SettingsManager::new(path.clone());
        manager
            .set_value("update.autoCheck", SettingValue::Bool(false))
            .expect("set");
        let saved: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).expect("read")).expect("parse");
        assert_eq!(saved["version"], serde_json::json!(SETTINGS_VERSION));
    }

    #[test]
    fn set_value_validates_key_and_type() {
        let dir = tempdir().expect("tempdir");
        let manager = SettingsManager::new(settings_path(dir.path()));

        assert_eq!(
            manager.set_value("nope", SettingValue::Bool(false)),
            Err("Unknown setting key: nope".to_string())
        );
        assert_eq!(
            manager.set_value("update.autoCheck", SettingValue::Str("yes".to_string())),
            Err("Type mismatch for `update.autoCheck`: expected bool, got string".to_string())
        );
    }

    #[test]
    fn set_value_returns_changed_keys() {
        let dir = tempdir().expect("tempdir");
        let manager = SettingsManager::new(settings_path(dir.path()));
        assert_eq!(
            manager
                .set_value("update.autoCheck", SettingValue::Bool(false))
                .expect("set"),
            vec!["update.autoCheck".to_string()]
        );
    }

    /// `provider` is gone from the local contract — a legacy
    /// key is tolerated (preserved in `extra`, warned), never skips the
    /// entry, and a plain `{ path }` entry loads as before.
    #[test]
    fn enabled_sources_skips_disabled_and_tolerates_legacy_provider() {
        let dir = tempdir().expect("tempdir");
        let path = write_settings(
            dir.path(),
            "{\"sources\": [\
                {\"path\": \"D:/home-backup\", \"provider\": \"codex\"},\
                {\"path\": \"D:/other\", \"provider\": \"claude\", \"enabled\": false},\
                {\"path\": \"D:/plain\"}\
            ]}",
        );
        let manager = SettingsManager::new(path);
        assert_eq!(
            manager.enabled_sources(),
            vec![
                local_with_extra("D:/home-backup", true, "provider", "codex"),
                local("D:/plain", true),
            ]
        );
        // The disabled entry is kept in the merged view (with its legacy
        // key preserved)...
        assert_eq!(
            manager.get_value("sources"),
            Some(SettingValue::SourceList(vec![
                local_with_extra("D:/home-backup", true, "provider", "codex"),
                local_with_extra("D:/other", false, "provider", "claude"),
                local("D:/plain", true),
            ]))
        );
    }

    fn local(path: &str, enabled: bool) -> SourceEntry {
        SourceEntry::Local(LocalSource {
            path: path.to_string(),
            enabled,
            id: None,
            extra: BTreeMap::new(),
        })
    }

    fn local_with_extra(
        path: &str,
        enabled: bool,
        key: &str,
        value: &str,
    ) -> SourceEntry {
        SourceEntry::Local(LocalSource {
            path: path.to_string(),
            enabled,
            id: None,
            extra: BTreeMap::from([(
                key.to_string(),
                Value::String(value.to_string()),
            )]),
        })
    }

    fn ssh(id: &str, host: &str) -> SourceEntry {
        SourceEntry::Ssh(SshSource {
            id: id.to_string(),
            label: None,
            host: host.to_string(),
            port: 22,
            user: "u".to_string(),
            auth: SourceAuth::Agent,
            enabled: true,
            extra: BTreeMap::new(),
        })
    }

    #[test]
    fn zero_migration_old_file_parses_unchanged() {
        // An old file with no `kind` field anywhere parses as-is; the
        // legacy `provider` keys land in `extra` (preserved, ignored).
        let dir = tempdir().expect("tempdir");
        let path = write_settings(
            dir.path(),
            "{\"sources\": [{\"path\": \"D:/a\", \"provider\": \"claude\", \"enabled\": true},\
                {\"path\": \"D:/b\", \"provider\": \"codex\"}]}",
        );
        let manager = SettingsManager::new(path);
        assert_eq!(
            manager.get_value("sources"),
            Some(SettingValue::SourceList(vec![
                local_with_extra("D:/a", true, "provider", "claude"),
                local_with_extra("D:/b", true, "provider", "codex"),
            ]))
        );
    }

    #[test]
    fn loader_unknown_kind_skips_entry_only() {
        let dir = tempdir().expect("tempdir");
        let path = write_settings(
            dir.path(),
            "{\"sources\": [\
                {\"kind\": \"warp\", \"path\": \"D:/w\"},\
                {\"path\": \"D:/a\", \"provider\": \"claude\"}\
            ]}",
        );
        let manager = SettingsManager::new(path);
        // The rest of the list survives the unknown-kind entry.
        assert_eq!(
            manager.get_value("sources"),
            Some(SettingValue::SourceList(vec![local_with_extra(
                "D:/a", true, "provider", "claude"
            )]))
        );
    }

    #[test]
    fn loader_ssh_missing_required_fields_skipped() {
        let dir = tempdir().expect("tempdir");
        let path = write_settings(
            dir.path(),
            "{\"sources\": [\
                {\"kind\": \"ssh\", \"host\": \"h\", \"user\": \"u\", \"auth\": {\"mode\": \"agent\"}},\
                {\"kind\": \"ssh\", \"id\": \"i\", \"user\": \"u\", \"auth\": {\"mode\": \"agent\"}}\
            ]}",
        );
        let manager = SettingsManager::new(path);
        assert_eq!(
            manager.get_value("sources"),
            Some(SettingValue::SourceList(Vec::new()))
        );
    }

    #[test]
    fn loader_duplicate_id_skips_later_entry_across_kinds() {
        let dir = tempdir().expect("tempdir");
        let path = write_settings(
            dir.path(),
            "{\"sources\": [\
                {\"kind\": \"ssh\", \"id\": \"dup\", \"host\": \"h1\", \"user\": \"u\", \"auth\": {\"mode\": \"agent\"}},\
                {\"kind\": \"ssh\", \"id\": \"dup\", \"host\": \"h2\", \"user\": \"u\", \"auth\": {\"mode\": \"agent\"}},\
                {\"path\": \"D:/a\", \"provider\": \"claude\", \"id\": \"dup\"},\
                {\"path\": \"D:/b\", \"provider\": \"codex\", \"id\": \"other\"}\
            ]}",
        );
        let manager = SettingsManager::new(path);
        let first = ssh("dup", "h1");
        assert_eq!(
            manager.get_value("sources"),
            Some(SettingValue::SourceList(vec![
                first,
                SourceEntry::Local(LocalSource {
                    path: "D:/b".to_string(),
                    enabled: true,
                    id: Some("other".to_string()),
                    extra: BTreeMap::from([(
                        "provider".to_string(),
                        Value::String("codex".to_string())
                    )]),
                }),
            ]))
        );
    }

    #[test]
    fn loader_ssh_parses_and_preserves_unknown_fields() {
        let dir = tempdir().expect("tempdir");
        let path = write_settings(
            dir.path(),
            "{\"sources\": [{\
                \"kind\": \"ssh\", \"id\": \"ali\", \"label\": \"Aliyun dev\",\
                \"host\": \"192.0.2.10\", \"user\": \"admin\",\
                \"auth\": {\"mode\": \"key\", \"keyPath\": \"~/.ssh/id_ed25519\"},\
                \"provider\": \"stray-local-field\"\
            }]}",
        );
        let manager = SettingsManager::new(path);
        let entry = match manager.get_value("sources") {
            Some(SettingValue::SourceList(mut list)) => list.remove(0),
            _ => panic!("expected a source list"),
        };
        let SourceEntry::Ssh(s) = entry else {
            panic!("expected an ssh entry")
        };
        assert_eq!(s.id, "ali");
        assert_eq!(s.label.as_deref(), Some("Aliyun dev"));
        assert_eq!(s.port, 22); // defaulted
        assert_eq!(
            s.auth,
            SourceAuth::Key {
                key_path: "~/.ssh/id_ed25519".to_string()
            }
        );
        // Forward compat: the stray local field is tolerated AND preserved.
        assert_eq!(
            s.extra.get("provider"),
            Some(&serde_json::json!("stray-local-field"))
        );
    }

    /// Compatibility pin: a file written by the OLD
    /// model (ssh entries carrying `root` + `providerHint`) must still
    /// load, with both keys swallowed into `extra` — no skip, no error.
    #[test]
    fn loader_ssh_legacy_root_and_provider_hint_load_into_extra() {
        let dir = tempdir().expect("tempdir");
        let path = write_settings(
            dir.path(),
            "{\"sources\": [{\
                \"kind\": \"ssh\", \"id\": \"ali\", \"host\": \"192.0.2.10\", \"user\": \"admin\",\
                \"root\": \"~/.claude/projects\", \"providerHint\": \"claude\",\
                \"auth\": {\"mode\": \"agent\"}\
            }]}",
        );
        let manager = SettingsManager::new(path);
        let entry = match manager.get_value("sources") {
            Some(SettingValue::SourceList(mut list)) => list.remove(0),
            _ => panic!("expected a source list"),
        };
        let SourceEntry::Ssh(s) = entry else {
            panic!("expected an ssh entry (legacy fields must not skip it)");
        };
        assert_eq!(s.id, "ali");
        // The removed schema fields are preserved verbatim as unknowns —
        // a save round-trips them instead of dropping data.
        assert_eq!(
            s.extra.get("root"),
            Some(&serde_json::json!("~/.claude/projects"))
        );
        assert_eq!(
            s.extra.get("providerHint"),
            Some(&serde_json::json!("claude"))
        );
    }

    #[test]
    fn local_edit_round_trip_preserves_ssh_entries() {
        let dir = tempdir().expect("tempdir");
        let path = write_settings(
            dir.path(),
            "{\"sources\": [\
                {\"path\": \"D:/a\", \"provider\": \"claude\"},\
                {\"kind\": \"ssh\", \"id\": \"ali\", \"label\": \"Aliyun dev\",\
                    \"host\": \"192.0.2.10\", \"port\": 2222, \"user\": \"admin\",\
                    \"root\": \"~/.claude/projects\", \"auth\": {\"mode\": \"key\", \"keyPath\": \"~/.ssh/k\"},\
                    \"providerHint\": \"claude\", \"enabled\": true}\
            ]}",
        );
        let manager = SettingsManager::new(path.clone());

        // The UI commit shape (documented): set_value("sources", …) carries
        // the FULL list — the edited local entries plus the ssh entries
        // verbatim, exactly as get_value served them.
        let SettingValue::SourceList(mut current) = manager.get_value("sources").expect("sources")
        else {
            panic!("expected source list")
        };
        let ssh_entry = current.remove(1);
        current[0] = local("D:/renamed", true);
        let next = vec![current[0].clone(), ssh_entry.clone()];
        manager
            .set_value("sources", SettingValue::SourceList(next))
            .expect("set");

        // Reload from disk: both entries round-trip faithfully.
        let reloaded = SettingsManager::new(path.clone());
        assert_eq!(
            reloaded.get_value("sources"),
            Some(SettingValue::SourceList(vec![
                local("D:/renamed", true),
                ssh_entry,
            ]))
        );
        // Spot-check the serialized ssh JSON shape (kind tag, camelCase).
        let text = std::fs::read_to_string(&path).expect("read");
        assert!(text.contains("\"kind\": \"ssh\""));
        assert!(text.contains("\"keyPath\": \"~/.ssh/k\""));
        assert!(text.contains("\"providerHint\": \"claude\""));
    }

    #[test]
    fn enabled_sources_includes_enabled_ssh_entries() {
        // enabled_sources() is kind-agnostic (the scan overlay, not the
        // settings core, filters to Local).
        let dir = tempdir().expect("tempdir");
        let path = write_settings(
            dir.path(),
            "{\"sources\": [\
                {\"kind\": \"ssh\", \"id\": \"a\", \"host\": \"h\", \"user\": \"u\", \"auth\": {\"mode\": \"agent\"}},\
                {\"kind\": \"ssh\", \"id\": \"b\", \"host\": \"h\", \"user\": \"u\", \"auth\": {\"mode\": \"agent\"}, \"enabled\": false}\
            ]}",
        );
        let manager = SettingsManager::new(path);
        assert_eq!(manager.enabled_sources(), vec![ssh("a", "h")]);
    }

    /// Wire pin: the sshConfig variant must serialize with the
    /// camelCase tag "sshConfig" (the enum-level lowercase rename would
    /// produce "sshconfig"), and parse back symmetrically.
    #[test]
    fn ssh_config_auth_wire_shape_is_camel_case() {
        let wire = serde_json::to_value(SourceAuth::SshConfig {
            alias: "ali".to_string(),
        })
        .expect("serialize");
        assert_eq!(
            wire,
            serde_json::json!({"mode": "sshConfig", "alias": "ali"})
        );
        let back: SourceAuth = serde_json::from_value(wire).expect("deserialize");
        assert_eq!(
            back,
            SourceAuth::SshConfig {
                alias: "ali".to_string()
            }
        );
        // The lowercase forms of the other modes are unchanged.
        assert_eq!(
            serde_json::to_value(SourceAuth::Agent).unwrap(),
            serde_json::json!({"mode": "agent"})
        );
    }

    /// An ssh entry using the sshConfig auth mode loads through the
    /// lenient kind-first loader like any other.
    #[test]
    fn loader_ssh_config_auth_entry_loads() {
        let dir = tempdir().expect("tempdir");
        let path = write_settings(
            dir.path(),
            "{\"sources\": [{\
                \"kind\": \"ssh\", \"id\": \"ali\", \"host\": \"placeholder\", \"user\": \"placeholder\",\
                \"auth\": {\"mode\": \"sshConfig\", \"alias\": \"ali\"}\
            }]}",
        );
        let manager = SettingsManager::new(path);
        let entry = match manager.get_value("sources") {
            Some(SettingValue::SourceList(mut list)) => list.remove(0),
            _ => panic!("expected a source list"),
        };
        let SourceEntry::Ssh(s) = entry else {
            panic!("expected an ssh entry")
        };
        assert_eq!(
            s.auth,
            SourceAuth::SshConfig {
                alias: "ali".to_string()
            }
        );
    }

    #[test]
    fn migrate_is_noop_at_v1() {
        let mut raw = serde_json::json!({"version": 1, "update": {"autoCheck": false}});
        assert!(migrate(&mut raw).is_empty());
        assert_eq!(raw["update"]["autoCheck"], serde_json::json!(false));
    }

    #[test]
    fn empty_sources_list_equals_default_and_is_sparse() {
        let dir = tempdir().expect("tempdir");
        let path = write_settings(
            dir.path(),
            "{\"sources\": [{\"path\": \"D:/x\", \"provider\": \"codex\"}]}",
        );
        let manager = SettingsManager::new(path.clone());
        manager
            .set_value("sources", SettingValue::SourceList(Vec::new()))
            .expect("set");
        let text = std::fs::read_to_string(&path).expect("read");
        assert!(
            !text.contains("sources"),
            "empty list is default-equal and must be omitted"
        );
    }

    #[test]
    fn snapshot_carries_descriptors_and_merged_values() {
        let dir = tempdir().expect("tempdir");
        let path = write_settings(dir.path(), "{\"update\": {\"autoCheck\": false}}");
        let manager = SettingsManager::new(path);
        let snapshot = manager.get();
        assert_eq!(snapshot.version, 1);
        assert_eq!(snapshot.descriptors.len(), SETTINGS.len());
        assert_eq!(
            snapshot.values["update.autoCheck"],
            SettingValue::Bool(false)
        );
        assert_eq!(
            snapshot.values["sources"],
            SettingValue::SourceList(Vec::new())
        );
    }
}
