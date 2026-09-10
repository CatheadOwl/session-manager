//! Settings core (ADR 0006): a hand-editable, layered `settings.json`.
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

/// One `sources[]` entry (ADR 0008): a kind-discriminated union. `kind`
/// defaults to `"local"` when absent, so pre-ADR-0008 files parse unchanged
/// (zero migration, no version bump). Serialization omits `kind` for local
/// entries — a local entry is the file's minimal historical shape
/// `{ path, provider, enabled }`; ssh entries always carry `"kind": "ssh"`.
#[derive(Clone, Debug, PartialEq)]
pub enum SourceEntry {
    /// Local extra scan root (ADR 0006 D2 overlay). `provider` is required
    /// (D5): a guessed parser risks wrong session semantics. `id` is
    /// optional for local entries (ADR 0008 §1).
    Local(LocalSource),
    /// SSH remote source (ADR 0008 / ADR 0007 remote v1). Consumed by the
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
    /// share one namespace across kinds within a file (ADR 0008 §1).
    pub fn id(&self) -> Option<&str> {
        match self {
            SourceEntry::Local(l) => l.id.as_deref(),
            SourceEntry::Ssh(s) => Some(s.id.as_str()),
        }
    }
}

/// Local `sources[]` payload: zero-migration superset of the pre-ADR-0008
/// shape (`id` is the only addition, and it is optional).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct LocalSource {
    pub path: String,
    pub provider: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

/// SSH auth block (ADR 0008 §1): tagged by `mode`, camelCase `keyPath`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "mode", rename_all = "lowercase")]
pub enum SourceAuth {
    Agent,
    Key {
        #[serde(rename = "keyPath")]
        key_path: String,
    },
}

fn default_port() -> u16 {
    22
}

/// SSH `sources[]` payload (ADR 0008 §1). `id`/`host`/`root` are required
/// by the loader (warn + skip when missing); `user` and `auth` are required
/// by the shape (a missing field fails entry parse → warn + skip, same net
/// behavior). `extra` preserves unknown fields (e.g. a stray `provider`)
/// verbatim through saves — forward compatibility per ADR 0008 §2.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SshSource {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    pub user: String,
    pub root: String,
    pub auth: SourceAuth,
    #[serde(rename = "providerHint", default, skip_serializing_if = "Option::is_none")]
    pub provider_hint: Option<String>,
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
        // Kind-first discrimination (ADR 0008 §2); absent kind = "local".
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

/// Outcome of an ADR 0008 §1a auto-heal attempt (`heal_provider_hint`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderHintHeal {
    /// Hint persisted (atomic save done); caller emits `settings-changed`.
    Applied,
    /// File contained comments — write refused to protect them (§1a
    /// condition 1). The probe itself may still have run.
    SkippedComments,
    /// Entry already carries this exact hint — idempotent no-op.
    AlreadySet,
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
/// the CLI adapter (ADR 0005) can consume it too.
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
                        if SettingValue::SourceList(list.clone())
                            != default_of("sources")
                        {
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
            SettingValue::SourceList(list) => {
                list.into_iter().filter(|s| s.is_enabled()).collect()
            }
            _ => Vec::new(),
        }
    }

    /// ADR 0008 §1a auto-heal write path: persist a scan-detected provider
    /// onto the ssh source identified by `source_id` (the same id the Remote
    /// locator anchors to). This is the ONLY sanctioned way for scan-layer
    /// code to write a detected hint — it reuses the D7 comment guard, the
    /// atomic sparse save, and the full-list round-trip in one place, so no
    /// second comment-detection or settings-writing logic may grow beside it.
    /// CLI adapters MUST NOT call this (read-only semantics, §1a condition 2).
    /// The caller emits `settings-changed` when the outcome is `Applied`.
    pub fn heal_provider_hint(
        &self,
        source_id: &str,
        hint: &str,
    ) -> Result<ProviderHintHeal, String> {
        let mut store = self.store.lock().unwrap();
        let mut list: Vec<SourceEntry> = match store.overrides.get("sources") {
            Some(SettingValue::SourceList(list)) => list.clone(),
            _ => return Err(format!("Unknown source id: {source_id}")),
        };
        let entry = list
            .iter_mut()
            .find(|e| matches!(e, SourceEntry::Ssh(s) if s.id == source_id))
            .ok_or_else(|| format!("Unknown source id: {source_id}"))?;
        let SourceEntry::Ssh(ssh) = entry else {
            unreachable!("find matched an Ssh entry");
        };
        if ssh.provider_hint.as_deref() == Some(hint) {
            return Ok(ProviderHintHeal::AlreadySet);
        }
        if store.had_comments {
            // §1a condition 1: commented files are never healed — the probe
            // may run, the write does not (D7 strip-diff detection lives
            // here, nowhere else).
            log::warn!(
                "settings: auto-heal skipped for source `{source_id}` — file contains comments (ADR 0008 §1a)"
            );
            return Ok(ProviderHintHeal::SkippedComments);
        }
        ssh.provider_hint = Some(hint.to_string());
        store
            .overrides
            .insert("sources".to_string(), SettingValue::SourceList(list));
        self.save(&mut store)?;
        Ok(ProviderHintHeal::Applied)
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
                root.insert(section.to_string(), Value::Object(std::mem::take(&mut fields)));
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
                store.overrides.get(def.key).cloned().unwrap_or_else(|| def.default.clone()),
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

/// Kind-first lenient loader (ADR 0008 §2): each entry is parsed
/// independently and a bad entry is warned + skipped WITHOUT dropping the
/// rest of the list. Per-kind rules fall out of the typed payload parse:
/// - unknown `kind` → error → warn + skip that entry only;
/// - `local` missing `provider` (or wrong-typed path/enabled) → skip (D5);
/// - `ssh` missing `id`/`host`/`root` (or `user`/`auth`) → skip;
/// - ssh extra unknown fields (e.g. `provider`) → tolerated and preserved;
/// - duplicate `id` across the file (ssh AND local share the namespace)
///   → warn + skip the LATER entry.
fn parse_sources(value: &Value) -> Option<Vec<SourceEntry>> {
    let array = value.as_array()?;
    let mut list = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::new();
    for entry in array {
        match serde_json::from_value::<SourceEntry>(entry.clone()) {
            Ok(parsed) => {
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
        assert_eq!(snapshot.values["update.autoCheck"], SettingValue::Bool(true));
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
        assert!(saved.get("update").is_none(), "default-equal key must be omitted");
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

    #[test]
    fn enabled_sources_skips_disabled_and_requires_provider() {
        let dir = tempdir().expect("tempdir");
        let path = write_settings(
            dir.path(),
            "{\"sources\": [\
                {\"path\": \"D:/jsonl/dump\", \"provider\": \"codex\"},\
                {\"path\": \"D:/other\", \"provider\": \"claude\", \"enabled\": false},\
                {\"path\": \"D:/no-provider\"}\
            ]}",
        );
        let manager = SettingsManager::new(path);
        assert_eq!(manager.enabled_sources(), vec![local("D:/jsonl/dump", "codex", true)]);
        // The disabled entry is kept in the merged view...
        assert_eq!(
            manager.get_value("sources"),
            Some(SettingValue::SourceList(vec![
                local("D:/jsonl/dump", "codex", true),
                local("D:/other", "claude", false),
            ]))
        );
    }

    fn local(path: &str, provider: &str, enabled: bool) -> SourceEntry {
        SourceEntry::Local(LocalSource {
            path: path.to_string(),
            provider: provider.to_string(),
            enabled,
            id: None,
        })
    }

    fn ssh(id: &str, host: &str) -> SourceEntry {
        SourceEntry::Ssh(SshSource {
            id: id.to_string(),
            label: None,
            host: host.to_string(),
            port: 22,
            user: "u".to_string(),
            root: "~".to_string(),
            auth: SourceAuth::Agent,
            provider_hint: None,
            enabled: true,
            extra: BTreeMap::new(),
        })
    }

    #[test]
    fn zero_migration_old_file_parses_unchanged() {
        // A pre-ADR-0008 file (no kind field anywhere) parses as-is.
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
                local("D:/a", "claude", true),
                local("D:/b", "codex", true),
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
            Some(SettingValue::SourceList(vec![local("D:/a", "claude", true)]))
        );
    }

    #[test]
    fn loader_ssh_missing_required_fields_skipped() {
        let dir = tempdir().expect("tempdir");
        let path = write_settings(
            dir.path(),
            "{\"sources\": [\
                {\"kind\": \"ssh\", \"host\": \"h\", \"root\": \"~\", \"user\": \"u\", \"auth\": {\"mode\": \"agent\"}},\
                {\"kind\": \"ssh\", \"id\": \"i\", \"root\": \"~\", \"user\": \"u\", \"auth\": {\"mode\": \"agent\"}},\
                {\"kind\": \"ssh\", \"id\": \"i\", \"host\": \"h\", \"user\": \"u\", \"auth\": {\"mode\": \"agent\"}}\
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
                {\"kind\": \"ssh\", \"id\": \"dup\", \"host\": \"h1\", \"user\": \"u\", \"root\": \"~\", \"auth\": {\"mode\": \"agent\"}},\
                {\"kind\": \"ssh\", \"id\": \"dup\", \"host\": \"h2\", \"user\": \"u\", \"root\": \"~\", \"auth\": {\"mode\": \"agent\"}},\
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
                    provider: "codex".to_string(),
                    enabled: true,
                    id: Some("other".to_string()),
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
                \"host\": \"192.0.2.10\", \"user\": \"admin\", \"root\": \"~/.claude/projects\",\
                \"auth\": {\"mode\": \"key\", \"keyPath\": \"~/.ssh/id_ed25519\"},\
                \"providerHint\": \"claude\", \"provider\": \"stray-local-field\"\
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
            SourceAuth::Key { key_path: "~/.ssh/id_ed25519".to_string() }
        );
        assert_eq!(s.provider_hint.as_deref(), Some("claude"));
        // Forward compat: the stray local field is tolerated AND preserved.
        assert_eq!(
            s.extra.get("provider"),
            Some(&serde_json::json!("stray-local-field"))
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
        let SettingValue::SourceList(mut current) =
            manager.get_value("sources").expect("sources")
        else {
            panic!("expected source list")
        };
        let ssh_entry = current.remove(1);
        current[0] = local("D:/renamed", "codex", true);
        let next = vec![current[0].clone(), ssh_entry.clone()];
        manager
            .set_value("sources", SettingValue::SourceList(next))
            .expect("set");

        // Reload from disk: both entries round-trip faithfully.
        let reloaded = SettingsManager::new(path.clone());
        assert_eq!(
            reloaded.get_value("sources"),
            Some(SettingValue::SourceList(vec![
                local("D:/renamed", "codex", true),
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
        // settings core, filters to Local — ADR 0008 §3).
        let dir = tempdir().expect("tempdir");
        let path = write_settings(
            dir.path(),
            "{\"sources\": [\
                {\"kind\": \"ssh\", \"id\": \"a\", \"host\": \"h\", \"user\": \"u\", \"root\": \"~\", \"auth\": {\"mode\": \"agent\"}},\
                {\"kind\": \"ssh\", \"id\": \"b\", \"host\": \"h\", \"user\": \"u\", \"root\": \"~\", \"auth\": {\"mode\": \"agent\"}, \"enabled\": false}\
            ]}",
        );
        let manager = SettingsManager::new(path);
        assert_eq!(manager.enabled_sources(), vec![ssh("a", "h")]);
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
        assert!(!text.contains("sources"), "empty list is default-equal and must be omitted");
    }

    #[test]
    fn snapshot_carries_descriptors_and_merged_values() {
        let dir = tempdir().expect("tempdir");
        let path = write_settings(dir.path(), "{\"update\": {\"autoCheck\": false}}");
        let manager = SettingsManager::new(path);
        let snapshot = manager.get();
        assert_eq!(snapshot.version, 1);
        assert_eq!(snapshot.descriptors.len(), SETTINGS.len());
        assert_eq!(snapshot.values["update.autoCheck"], SettingValue::Bool(false));
        assert_eq!(snapshot.values["sources"], SettingValue::SourceList(Vec::new()));
    }

    #[test]
    fn heal_applies_persists_and_is_idempotent() {
        let dir = tempdir().expect("tempdir");
        let path = write_settings(
            dir.path(),
            "{\"sources\": [{\"kind\": \"ssh\", \"id\": \"srv\", \"host\": \"h\", \
              \"user\": \"u\", \"root\": \"~\", \"auth\": {\"mode\": \"agent\"}}]}",
        );
        let manager = SettingsManager::new(path.clone());
        assert_eq!(
            manager.heal_provider_hint("srv", "claude").expect("heal"),
            ProviderHintHeal::Applied
        );
        // Persisted: a fresh manager sees the hint (terminal state explicit).
        let reloaded = SettingsManager::new(path);
        match reloaded.enabled_sources()[0] {
            SourceEntry::Ssh(ref s) => assert_eq!(s.provider_hint.as_deref(), Some("claude")),
            ref other => panic!("expected ssh entry, got {other:?}"),
        }
        // Idempotent: same hint again is a no-op outcome.
        assert_eq!(
            reloaded.heal_provider_hint("srv", "claude").expect("heal"),
            ProviderHintHeal::AlreadySet
        );
    }

    #[test]
    fn heal_refuses_commented_files_without_writing() {
        let dir = tempdir().expect("tempdir");
        let original = "{\n  // my annotated server\n  \"sources\": [{\"kind\": \"ssh\", \
            \"id\": \"srv\", \"host\": \"h\", \"user\": \"u\", \"root\": \"~\", \
            \"auth\": {\"mode\": \"agent\"}}]\n}";
        let path = write_settings(dir.path(), original);
        let manager = SettingsManager::new(path.clone());
        assert_eq!(
            manager.heal_provider_hint("srv", "claude").expect("heal"),
            ProviderHintHeal::SkippedComments
        );
        // File untouched: comment survives, no hint written.
        let text = std::fs::read_to_string(&path).expect("read");
        assert_eq!(text, original);
    }

    #[test]
    fn heal_unknown_source_id_is_an_error() {
        let dir = tempdir().expect("tempdir");
        let path = write_settings(
            dir.path(),
            "{\"sources\": [{\"kind\": \"ssh\", \"id\": \"srv\", \"host\": \"h\", \
              \"user\": \"u\", \"root\": \"~\", \"auth\": {\"mode\": \"agent\"}}]}",
        );
        let manager = SettingsManager::new(path);
        assert!(manager.heal_provider_hint("nope", "claude").is_err());
    }
}
