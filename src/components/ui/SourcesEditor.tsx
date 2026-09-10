import { useEffect, useState } from "react";
import type { LocalSourceEntry, SourceEntry, SshSourceEntry } from "@/lib/api/settings";
import { ConfirmDeleteDialog, type ConfirmActionTarget } from "@/components/sessions/ConfirmDeleteDialog";
import { AddSshSourcePanel } from "./AddSshSourcePanel";
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
  value.map((entry) => ({
    entry,
    // SSH rows are read-only; pathDraft only applies to local rows.
    pathDraft: entry.kind === "ssh" ? "" : entry.path,
  }));

/**
 * UI primitive: `sourceList` renderer (ADR 0006 sources overlay). One row per
 * entry — path text input, provider picker (shared `Menu` primitive), enabled
 * toggle, remove (danger, guarded by ConfirmDeleteDialog). "Add source"
 * appends a draft row with the first provider preselected. Structural changes
 * (provider/enabled/add/remove) commit immediately via `onChange`; path edits
 * commit on blur/Enter so typing does not spam the IPC write path. Per-row
 * validation (empty path, duplicate paths) is display-only — the Rust core
 * re-validates on save.
 *
 * ADR 0008: ssh entries render as a READ-ONLY summary row (per-field remote
 * editing is future work) and MUST be included verbatim in every `onChange`
 * commit — a local edit must never drop them from the file. The commit shape
 * is therefore the FULL list: edited local entries + ssh entries untouched.
 * Adding NEW ssh entries goes through the "Add SSH source…" flow
 * (`AddSshSourcePanel`: ssh-config alias picker or manual form, with a test
 * connection step — ADR 0010).
 */
export function SourcesEditor({ label, description, value, providers, onChange, error }: SourcesEditorProps) {
  const [rows, setRows] = useState<DraftRow[]>(() => toDraft(value));
  const [pendingRemove, setPendingRemove] = useState<number | null>(null);
  const [openProvider, setOpenProvider] = useState<number | null>(null);
  const [sshAddOpen, setSshAddOpen] = useState(false);

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
    const current = rows[index];
    if (current.entry.kind === "ssh") return; // read-only row, no path edits
    const trimmed = current.pathDraft.trim();
    if (trimmed === current.entry.path) return;
    commit(
      rows.map((row, i) =>
        i === index && row.entry.kind !== "ssh"
          ? { entry: { ...row.entry, path: trimmed }, pathDraft: trimmed }
          : row,
      ),
    );
  };

  const updateEntry = (index: number, patch: Partial<LocalSourceEntry>) => {
    commit(
      rows.map((row, i) =>
        i === index && row.entry.kind !== "ssh"
          ? { ...row, entry: { ...row.entry, ...patch } }
          : row,
      ),
    );
  };

  const addSource = () => {
    if (providers.length === 0) return;
    commit([...rows, { entry: { path: "", provider: providers[0], enabled: true }, pathDraft: "" }]);
  };

  // The add-SSH flow commits a fully-formed entry from AddSshSourcePanel
  // (sshConfig alias reference or hand-filled fields) — same full-list
  // commit shape as every other structural change.
  const addSshSource = (entry: SshSourceEntry) => {
    setSshAddOpen(false);
    commit([...rows, { entry, pathDraft: "" }]);
  };

  const removeRow = (index: number) => {
    setPendingRemove(null);
    commit(rows.filter((_, i) => i !== index));
  };

  const removeTarget: ConfirmActionTarget | null = (() => {
    if (pendingRemove === null) return null;
    const entry = rows[pendingRemove]?.entry;
    return entry && entry.kind !== "ssh"
      ? { kind: "remove-source", path: entry.path }
      : null;
  })();

  return (
    <SettingRow label={label} description={description} error={error}>
      <div className="setting-sources">
        {rows.length === 0 ? (
          <div className="setting-sources-empty">No extra sources — built-in provider folders are always scanned.</div>
        ) : null}
        {rows.map((row, index) => {
          // SSH summary row (read-only): kind badge + identity + pointer to
          // hand editing. The entry object itself rides along in every
          // commit untouched (see the component doc comment).
          if (row.entry.kind === "ssh") {
            const ssh = row.entry;
            // sshConfig entries keep placeholder host/user fields (ADR
            // 0010): the identity line shows the LIVE alias reference
            // instead — no resolution request here, by design (a config
            // edit must not need a settings round-trip to render).
            const viaAlias =
              ssh.auth.mode === "sshConfig" ? ssh.auth.alias : null;
            return (
              <div className="setting-source-row setting-source-row--ssh" key={index}>
                <span className="setting-source-kind-badge">SSH</span>
                <span className="setting-source-ssh-summary">
                  <span className="setting-source-ssh-title">
                    {ssh.label ? `${ssh.label} (${ssh.id})` : ssh.id}
                  </span>
                  <span className="setting-source-ssh-host">
                    {viaAlias !== null
                      ? `ssh config alias: ${viaAlias}`
                      : `${ssh.user ? `${ssh.user}@` : ""}${ssh.host}${
                          ssh.port && ssh.port !== 22 ? `:${ssh.port}` : ""
                        }`}
                  </span>
                  <span className="setting-source-ssh-note">
                    Edit in settings.json — remote editor coming
                  </span>
                </span>
              </div>
            );
          }
          const validation = validateRow(row, rows);
          // Capture the narrowed local entry: const keeps the narrowing
          // inside the nested render closures below.
          const entry = row.entry;
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
                label={`Source ${index + 1} provider: ${entry.provider}`}
                open={openProvider === index}
                onOpenChange={(open) => setOpenProvider(open ? index : null)}
                renderTrigger={(triggerProps) => (
                  <button type="button" className="secondary-button setting-provider-trigger" {...triggerProps}>
                    <span className="setting-provider-name">{entry.provider}</span>
                    <span className="setting-provider-caret" aria-hidden="true">▾</span>
                  </button>
                )}
              >
                {providers.map((id) => (
                  <MenuItem
                    key={id}
                    checked={entry.provider === id}
                    active={entry.provider === id}
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
        {sshAddOpen ? (
          <AddSshSourcePanel onAdd={addSshSource} onClose={() => setSshAddOpen(false)} />
        ) : (
          <button
            type="button"
            className="secondary-button setting-source-add"
            onClick={() => setSshAddOpen(true)}
          >
            Add SSH source…
          </button>
        )}
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

/** Display-only per-row validation (local rows): empty path, duplicate paths. */
function validateRow(row: DraftRow, all: DraftRow[]): string | null {
  const path = row.pathDraft.trim();
  if (path === "") return "Path is required";
  const locals = all.filter((r) => r.entry.kind !== "ssh");
  if (locals.filter((r) => r.pathDraft.trim() === path).length > 1) return "Duplicate path";
  return null;
}
