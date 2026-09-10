import { invoke } from "@tauri-apps/api/core";

/**
 * Settings core IPC surface (ADR 0006). The Rust side is the SSOT for the
 * schema; these wrappers are typed mirrors of `session_manager/settings.rs`.
 * There is no other reach into the settings file from TS — hand-edits are
 * picked up through the `settings-changed` event / query invalidation.
 */

/**
 * One `sources[]` entry (ADR 0008): kind-discriminated union mirroring the
 * Rust `SourceEntry` enum. `kind` defaults to `"local"` on the Rust side, so
 * local entries may omit it (and the Rust writer omits it for them).
 */
export type SourceEntry = LocalSourceEntry | SshSourceEntry;

/** Local extra scan root (ADR 0006 overlay; kind defaults to "local"). */
export interface LocalSourceEntry {
  kind?: "local";
  /** Directory containing session files. */
  path: string;
  /** Provider id owning the parser for this root (`claude`, `codex`, …). Required. */
  provider: string;
  /** Disabled entries are kept in the file but not scanned. */
  enabled: boolean;
  /** Optional stable id (ADR 0008; ssh Remote locators need it, local may omit). */
  id?: string;
}

/** SSH auth block: tagged by `mode`, camelCase `keyPath`. */
export type SshSourceAuth = { mode: "agent" } | { mode: "key"; keyPath: string };

/**
 * SSH remote source (ADR 0008 / ADR 0007 remote v1). Not yet editable in the
 * UI — rendered read-only by SourcesEditor and passed through verbatim on
 * every commit. Unknown fields are preserved by the Rust loader
 * (forward compatibility), hence the index signature.
 */
export interface SshSourceEntry {
  kind: "ssh";
  /** Stable id — Remote locator `source_id` anchor. Required. */
  id: string;
  /** Optional display label. */
  label?: string;
  host: string;
  /** Defaults to 22 on the Rust side. */
  port?: number;
  user: string;
  auth: SshSourceAuth;
  enabled: boolean;
  /** Forward-compat: unknown fields round-trip through the Rust loader. */
  [extra: string]: unknown;
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
