import { invoke } from "@tauri-apps/api/core";

/**
 * Settings core IPC surface (ADR 0006). The Rust side is the SSOT for the
 * schema; these wrappers are typed mirrors of `session_manager/settings.rs`.
 * There is no other reach into the settings file from TS — hand-edits are
 * picked up through the `settings-changed` event / query invalidation.
 */

export interface SourceEntry {
  /** Directory containing session files. */
  path: string;
  /** Provider id owning the parser for this root (`claude`, `codex`, …). Required. */
  provider: string;
  /** Disabled entries are kept in the file but not scanned. */
  enabled: boolean;
}

/**
 * Externally-tagged setting value — matches the serde enum on the Rust side:
 * `{ bool: true }`, `{ string: "…" }`, `{ stringList: […] }`, `{ sourceList: […] }`.
 */
export type SettingValue =
  | { bool: boolean }
  | { string: string }
  | { stringList: string[] }
  | { sourceList: SourceEntry[] };

export type SettingType = "bool" | "string" | "stringList" | "sourceList";

export interface SettingDefinition {
  key: string;
  type: SettingType;
  default: SettingValue;
  group: string;
}

/** Merged view served by `get_settings`: descriptors + effective values. */
export interface SettingsSnapshot {
  version: number;
  descriptors: SettingDefinition[];
  values: Record<string, SettingValue>;
}

export async function fetchSettings(): Promise<SettingsSnapshot> {
  return await invoke("get_settings");
}

export async function setSettingValue(key: string, value: SettingValue): Promise<void> {
  return await invoke("set_setting_value", { key, value });
}

/**
 * Read-only provider id listing (same registry that backs the `agents` CLI
 * subcommand). Consumed by the settings UI's source editor provider picker.
 */
export async function fetchProviders(): Promise<string[]> {
  return await invoke("list_providers");
}
