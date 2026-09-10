import { useEffect, useState } from "react";
import type { SourceEntry } from "@/lib/api/settings";
import { ConfirmDeleteDialog, type ConfirmActionTarget } from "@/components/sessions/ConfirmDeleteDialog";
import { Menu, MenuItem } from "./Menu";
import { SettingRow } from "./SettingRow";

export interface SourcesEditorProps {
  label: string;
  description?: string;
  value: SourceEntry[];
  /** Provider ids for the picker (`list_providers` / `agents` SSOT). */
  providers: string[];
  /** Commits the WHOLE sources value (settings core writes per key; this key's value is the full list). */
  onChange: (next: SourceEntry[]) => void;
  /** Display-only inline error surfaced on the row (e.g. failed write). */
  error?: string | null;
}

interface DraftRow {
  entry: SourceEntry;
  /** In-progress path text; committed to `entry` (and upward) on blur/Enter. */
  pathDraft: string;
}

const toDraft = (value: SourceEntry[]): DraftRow[] =>
  value.map((entry) => ({ entry, pathDraft: entry.path }));

/**
 * UI primitive: `sourceList` renderer (ADR 0006 sources overlay). One row per
 * entry — path text input, provider picker (shared `Menu` primitive), enabled
 * toggle, remove (danger, guarded by ConfirmDeleteDialog). "Add source"
 * appends a draft row with the first provider preselected. Structural changes
 * (provider/enabled/add/remove) commit immediately via `onChange`; path edits
 * commit on blur/Enter so typing does not spam the IPC write path. Per-row
 * validation (empty path, duplicate paths) is display-only — the Rust core
 * re-validates on save.
 */
export function SourcesEditor({ label, description, value, providers, onChange, error }: SourcesEditorProps) {
  const [rows, setRows] = useState<DraftRow[]>(() => toDraft(value));
  const [pendingRemove, setPendingRemove] = useState<number | null>(null);
  const [openProvider, setOpenProvider] = useState<number | null>(null);

  // Resync when a new value arrives from outside (query refetch after
  // settings-changed). Local drafts are intentionally discarded.
  useEffect(() => {
    setRows(toDraft(value));
  }, [value]);

  const commit = (next: DraftRow[]) => {
    setRows(next);
    onChange(next.map((row) => row.entry));
  };

  const commitPath = (index: number) => {
    const trimmed = rows[index].pathDraft.trim();
    if (trimmed === rows[index].entry.path) return;
    commit(
      rows.map((row, i) =>
        i === index ? { entry: { ...row.entry, path: trimmed }, pathDraft: trimmed } : row,
      ),
    );
  };

  const updateEntry = (index: number, patch: Partial<SourceEntry>) => {
    commit(rows.map((row, i) => (i === index ? { ...row, entry: { ...row.entry, ...patch } } : row)));
  };

  const addSource = () => {
    if (providers.length === 0) return;
    commit([...rows, { entry: { path: "", provider: providers[0], enabled: true }, pathDraft: "" }]);
  };

  const removeRow = (index: number) => {
    setPendingRemove(null);
    commit(rows.filter((_, i) => i !== index));
  };

  const removeTarget: ConfirmActionTarget | null =
    pendingRemove === null
      ? null
      : { kind: "remove-source", path: rows[pendingRemove]?.entry.path ?? "" };

  return (
    <SettingRow label={label} description={description} error={error}>
      <div className="setting-sources">
        {rows.length === 0 ? (
          <div className="setting-sources-empty">No extra sources — built-in provider folders are always scanned.</div>
        ) : null}
        {rows.map((row, index) => {
          const validation = validateRow(row, rows);
          return (
            <div className="setting-source-row" key={index}>
              <input
                type="text"
                className="setting-source-input"
                value={row.pathDraft}
                placeholder="D:\\jsonl\\dump"
                aria-label={`Source ${index + 1} path`}
                aria-invalid={validation ? true : undefined}
                onChange={(e) =>
                  setRows((prev) => prev.map((r, i) => (i === index ? { ...r, pathDraft: e.target.value } : r)))
                }
                onBlur={() => commitPath(index)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    e.preventDefault();
                    commitPath(index);
                  }
                }}
              />
              <Menu
                label={`Source ${index + 1} provider: ${row.entry.provider}`}
                open={openProvider === index}
                onOpenChange={(open) => setOpenProvider(open ? index : null)}
                renderTrigger={(triggerProps) => (
                  <button type="button" className="secondary-button setting-provider-trigger" {...triggerProps}>
                    <span className="setting-provider-name">{row.entry.provider}</span>
                    <span className="setting-provider-caret" aria-hidden="true">▾</span>
                  </button>
                )}
              >
                {providers.map((id) => (
                  <MenuItem
                    key={id}
                    checked={row.entry.provider === id}
                    active={row.entry.provider === id}
                    onClick={() => {
                      updateEntry(index, { provider: id });
                      setOpenProvider(null);
                    }}
                  >
                    {id}
                  </MenuItem>
                ))}
              </Menu>
              <button
                type="button"
                role="switch"
                aria-checked={row.entry.enabled}
                aria-label={`Source ${index + 1} enabled`}
                title={row.entry.enabled ? "Disable source" : "Enable source"}
                className="setting-toggle setting-toggle--compact"
                onClick={() => updateEntry(index, { enabled: !row.entry.enabled })}
                onKeyDown={(event) => {
                  if (event.key === "Enter" || event.key === " ") {
                    event.preventDefault();
                    updateEntry(index, { enabled: !row.entry.enabled });
                  }
                }}
              >
                <span className="setting-toggle-knob" aria-hidden="true" />
              </button>
              <button
                type="button"
                className="danger-button setting-source-remove"
                aria-label={`Remove source ${index + 1}`}
                title="Remove source"
                onClick={() => setPendingRemove(index)}
              >
                Remove
              </button>
              {validation ? (
                <div className="setting-error setting-source-error" role="alert">
                  {validation}
                </div>
              ) : null}
            </div>
          );
        })}
        <button type="button" className="secondary-button setting-source-add" onClick={addSource} disabled={providers.length === 0}>
          Add source…
        </button>
        {removeTarget ? (
          <ConfirmDeleteDialog
            target={removeTarget}
            isWorking={false}
            onConfirm={() => pendingRemove !== null && removeRow(pendingRemove)}
            onCancel={() => setPendingRemove(null)}
          />
        ) : null}
      </div>
    </SettingRow>
  );
}

/** Display-only per-row validation: empty path, duplicate paths. */
function validateRow(row: DraftRow, all: DraftRow[]): string | null {
  const path = row.pathDraft.trim();
  if (path === "") return "Path is required";
  if (all.filter((r) => r.pathDraft.trim() === path).length > 1) return "Duplicate path";
  return null;
}
