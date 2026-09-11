import { useEffect, useState } from "react";
import type { LocalSourceEntry, SourceEntry, SshSourceEntry } from "@/lib/api/settings";
import { ConfirmDeleteDialog, type ConfirmActionTarget } from "@/components/sessions/ConfirmDeleteDialog";
import { AddSshSourcePanel } from "./AddSshSourcePanel";
import { SettingRow } from "./SettingRow";

export interface SourcesEditorProps {
  label: string;
  description?: string;
  value: SourceEntry[];
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
 * UI primitive: `sourceList` renderer (the settings sources overlay).
 * Rendered through SettingRow's FULL variant
 * — description on top, the list spanning the content width (list editors
 * do not fit the two-column toggle rhythm). One row per entry:
 *
 * - LOCAL (home-mirror model): path text input + enabled toggle +
 *   remove (danger, guarded by ConfirmDeleteDialog). There is NO provider
 *   picker — the path names an ALTERNATE HOME whose layout mirrors the
 *   real home, and every provider's sessions are auto-discovered under
 *   it. "Add source…" appends an empty draft row.
 * - SSH: the SAME structural controls as local rows — enabled
 *   toggle and guarded remove (unified expression; the read-only part is
 *   only the connection identity, which comes from the Add flow) — and
 *   MUST be included verbatim in every `onChange` commit: a local edit
 *   must never drop them from the file. The commit shape is therefore
 *   the FULL list. Adding NEW ssh entries goes through the "Add SSH
 *   source…" flow (`AddSshSourcePanel`: ssh-config alias picker or
 *   manual form, with a test connection step).
 *
 * Structural changes (enabled/add/remove) commit immediately via
 * `onChange`; path edits commit on blur/Enter so typing does not spam the
 * IPC write path. Per-row validation (empty path, duplicate paths) is
 * display-only — the Rust core re-validates on save.
 */
export function SourcesEditor({ label, description, value, onChange, error }: SourcesEditorProps) {
  const [rows, setRows] = useState<DraftRow[]>(() => toDraft(value));
  const [pendingRemove, setPendingRemove] = useState<number | null>(null);
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

  // SSH rows share the local rows' structural controls: same enabled
  // toggle and remove semantics, expressed on the summary identity
  // instead of a path input.
  const toggleSshEnabled = (index: number) => {
    commit(
      rows.map((row, i) =>
        i === index && row.entry.kind === "ssh"
          ? { ...row, entry: { ...row.entry, enabled: !row.entry.enabled } }
          : row,
      ),
    );
  };

  // A new local row is just `{ path, enabled }` — no provider.
  const addSource = () => {
    commit([...rows, { entry: { path: "", enabled: true }, pathDraft: "" }]);
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
    if (!entry) return null;
    if (entry.kind === "ssh") {
      return {
        kind: "remove-ssh-source",
        id: entry.id,
        alias: entry.auth.mode === "sshConfig" ? entry.auth.alias : null,
      };
    }
    return { kind: "remove-source", path: entry.path };
  })();

  return (
    <SettingRow label={label} description={description} error={error} variant="full">
      <div className="setting-sources">
        {rows.length === 0 ? (
          <div className="setting-sources-empty">No extra sources — this machine's home is always scanned.</div>
        ) : null}
        {rows.map((row, index) => {
          // SSH row: kind badge + ONE-LINE identity summary (label (id) ·
          // user@host:port, or · ssh config alias) + the SAME structural
          // controls as local rows (enabled toggle, guarded remove) — the
          // same visual rhythm as a local row: one line of identity +
          // toggle + Remove. The identity fields themselves stay read-only —
              // connection config comes from the Add flow or hand editing.
          if (row.entry.kind === "ssh") {
            const ssh = row.entry;
              // sshConfig entries keep placeholder host/user fields: the
              // identity line shows the LIVE alias reference
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
                  <span className="setting-source-ssh-sep" aria-hidden="true">·</span>
                  <span className="setting-source-ssh-host">
                    {viaAlias !== null
                      ? `ssh config alias: ${viaAlias}`
                      : `${ssh.user ? `${ssh.user}@` : ""}${ssh.host}${
                          ssh.port && ssh.port !== 22 ? `:${ssh.port}` : ""
                        }`}
                  </span>
                </span>
                <button
                  type="button"
                  role="switch"
                  aria-checked={ssh.enabled}
                  aria-label={`SSH source ${ssh.id} enabled`}
                  title={ssh.enabled ? "Disable SSH source" : "Enable SSH source"}
                  className="setting-toggle setting-toggle--compact"
                  onClick={() => toggleSshEnabled(index)}
                  onKeyDown={(event) => {
                    if (event.key === "Enter" || event.key === " ") {
                      event.preventDefault();
                      toggleSshEnabled(index);
                    }
                  }}
                >
                  <span className="setting-toggle-knob" aria-hidden="true" />
                </button>
                <button
                  type="button"
                  className="danger-button setting-source-remove"
                  aria-label={`Remove SSH source ${ssh.id}`}
                  title="Remove SSH source"
                  onClick={() => setPendingRemove(index)}
                >
                  Remove
                </button>
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
                placeholder="D:\backups\home-copy"
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
        <button type="button" className="secondary-button setting-source-add" onClick={addSource}>
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
