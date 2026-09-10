import { useCallback, useEffect, useState } from "react";
import {
  fetchSshAliases,
  fetchSshConfigPath,
  testSshSource,
  type SshAliasInfo,
  type SshSourceEntry,
  type SshTestResult,
} from "@/lib/api/settings";
import { CopyButton } from "@/components/sessions/CopyButton";
import { Menu, MenuItem } from "./Menu";

interface AddSshSourcePanelProps {
  /** Commits the fully-formed ssh entry (the caller appends it to the list). */
  onAdd: (entry: SshSourceEntry) => void;
  /** Closes the panel without adding. */
  onClose: () => void;
}

/** Manual ("advanced") form state — the no-ssh-config fallback path. */
interface ManualDraft {
  host: string;
  port: string;
  user: string;
  authMode: "agent" | "key";
  keyPath: string;
}

const EMPTY_MANUAL: ManualDraft = {
  host: "",
  port: "22",
  user: "",
  authMode: "agent",
  keyPath: "",
};

/**
 * UI primitive: the "Add SSH source" flow panel (ADR 0010 / ADR 0008 修订 1),
 * embedded inline under the sources list. Three states driven by
 * `list_ssh_aliases`:
 *
 * 1. **Aliases present** — picker rows (alias + `user@host` preview; ProxyJump
 *    rows greyed with "not supported yet") → editable source id →
 *    [Test connection] (`test_ssh_source`; ✓ "Connected — N sessions found"
 *    or the actionable error) → [Add] commits a
 *    `{ kind: "ssh", auth: { mode: "sshConfig", alias } }` entry. The
 *    preview is display-only: nothing but the alias is persisted (live
 *    reference, ADR 0010 Option A).
 * 2. **Empty (double exit)** — a) a guide card with the full `~/.ssh/config`
 *    path (CopyButton) + a three-line example block + [Refresh]; b) a
 *    collapsed "Advanced: manual configuration" form (host/port/user/auth
 *    agent|key, `~` supported in the key path) with the same test→add flow,
 *    committing a hand-filled entry.
 * 3. **Loading / error** — error carries a Retry.
 *
 * All controls come from the shared primitives (`Menu`, `CopyButton`,
 * `.secondary-button`/`.primary-button`, `.setting-source-input`); no
 * hardcoded colors (ADR 0004).
 */
export function AddSshSourcePanel({ onAdd, onClose }: AddSshSourcePanelProps) {
  // null aliases = still loading (distinct from [] = empty config).
  const [aliases, setAliases] = useState<SshAliasInfo[] | null>(null);
  const [configPath, setConfigPath] = useState("");
  const [loadError, setLoadError] = useState<string | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [idDraft, setIdDraft] = useState("");
  const [manualOpen, setManualOpen] = useState(false);
  const [manual, setManual] = useState<ManualDraft>(EMPTY_MANUAL);
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<SshTestResult | null>(null);

  const load = useCallback(async () => {
    setLoadError(null);
    setAliases(null);
    setTestResult(null);
    try {
      const [list, path] = await Promise.all([fetchSshAliases(), fetchSshConfigPath()]);
      setAliases(list);
      setConfigPath(path);
    } catch (err) {
      setLoadError(String(err));
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const selectedAlias = aliases?.find((a) => a.alias === selected) ?? null;

  const pickAlias = (alias: string) => {
    setSelected(alias);
    // The id defaults to the alias but stays editable.
    setIdDraft(alias);
    setTestResult(null);
  };

  /** The entry this panel would commit right now (null = incomplete form). */
  const draftEntry = (): SshSourceEntry | null => {
    if (manualOpen) {
      const host = manual.host.trim();
      const user = manual.user.trim();
      if (!host || !user) return null;
      const parsedPort = Number.parseInt(manual.port, 10);
      const port = Number.isFinite(parsedPort) && parsedPort > 0 ? parsedPort : 22;
      return {
        kind: "ssh",
        id: idDraft.trim() || host,
        host,
        port,
        user,
        auth:
          manual.authMode === "key"
            ? { mode: "key", keyPath: manual.keyPath.trim() || "~/.ssh/id_ed25519" }
            : { mode: "agent" },
        enabled: true,
      };
    }
    if (!selectedAlias || !selectedAlias.supported) return null;
    return {
      kind: "ssh",
      id: idDraft.trim() || selectedAlias.alias,
      // sshConfig placeholders — the connect layer overrides them from
      // the resolved Host block (ADR 0010 Decision 1).
      host: "",
      port: 22,
      user: "",
      auth: { mode: "sshConfig", alias: selectedAlias.alias },
      enabled: true,
    };
  };

  const runTest = async () => {
    const entry = draftEntry();
    if (!entry || testing) return;
    setTesting(true);
    setTestResult(null);
    try {
      setTestResult(await testSshSource(entry));
    } catch (err) {
      // Command-level failure (transport dropped, etc.) — same slot.
      setTestResult({ ok: false, error: String(err) });
    } finally {
      setTesting(false);
    }
  };

  const add = () => {
    const entry = draftEntry();
    if (entry) onAdd(entry);
  };

  const draft = draftEntry();

  return (
    <div className="setting-ssh-add-panel" role="group" aria-label="Add SSH source">
      <div className="setting-ssh-add-head">
        <span className="setting-ssh-add-title">Add SSH source</span>
        <button type="button" className="ghost-button setting-ssh-add-close" aria-label="Close" onClick={onClose}>
          ×
        </button>
      </div>

      {aliases === null && !loadError ? (
        <div className="setting-ssh-add-note">Reading ssh config…</div>
      ) : null}

      {loadError ? (
        <div className="setting-ssh-add-error-section">
          <div className="setting-error" role="alert">
            {loadError}
          </div>
          <button type="button" className="secondary-button" onClick={() => void load()}>
            Retry
          </button>
        </div>
      ) : null}

      {aliases && aliases.length > 0 ? (
        <div className="setting-ssh-add-body">
          <ul className="setting-ssh-alias-list">
            {aliases.map((alias) => (
              <li key={alias.alias}>
                <button
                  type="button"
                  className="setting-ssh-alias"
                  aria-pressed={selected === alias.alias}
                  disabled={!alias.supported}
                  onClick={() => pickAlias(alias.alias)}
                >
                  <span className="setting-ssh-alias-name">{alias.alias}</span>
                  <span className="setting-ssh-alias-preview">
                    {alias.user ? `${alias.user}@` : ""}
                    {alias.host}
                  </span>
                  {alias.supported ? null : (
                    <span className="setting-ssh-alias-note">ProxyJump — not supported yet</span>
                  )}
                </button>
              </li>
            ))}
          </ul>
          <AliasTestAdd
            idDraft={selectedAlias ? idDraft : ""}
            idPlaceholder={selectedAlias?.alias ?? "source id"}
            idLocked={!selectedAlias}
            testing={testing}
            testResult={testResult}
            canTest={Boolean(selectedAlias?.supported)}
            canAdd={Boolean(draft)}
            onIdChange={(v) => {
              setIdDraft(v);
            }}
            onTest={() => void runTest()}
            onAdd={add}
          />
        </div>
      ) : null}

      {aliases && aliases.length === 0 ? (
        <div className="setting-ssh-add-body">
          <div className="setting-ssh-guide">
            <p className="setting-ssh-guide-text">
              No ssh config found. Create it, add a Host block, then refresh:
            </p>
            <div className="setting-ssh-guide-path">
              <code>{configPath || "~/.ssh/config"}</code>
              {configPath ? <CopyButton text={configPath} label="Copy config path" /> : null}
            </div>
            <pre className="setting-ssh-guide-sample">{"Host ali\n  HostName 192.0.2.10\n  User admin"}</pre>
            <button type="button" className="secondary-button" onClick={() => void load()}>
              Refresh
            </button>
          </div>
          <button
            type="button"
            className="link-button setting-ssh-manual-toggle"
            aria-expanded={manualOpen}
            onClick={() => {
              setManualOpen((open) => !open);
              setTestResult(null);
            }}
          >
            Advanced: manual configuration
          </button>
          {manualOpen ? (
            <div className="setting-ssh-manual">
              <div className="setting-ssh-manual-grid">
                <input
                  type="text"
                  className="setting-source-input"
                  placeholder="host (192.0.2.10)"
                  aria-label="SSH host"
                  value={manual.host}
                  onChange={(e) => setManual((m) => ({ ...m, host: e.target.value }))}
                />
                <input
                  type="text"
                  className="setting-source-input"
                  placeholder="22"
                  aria-label="SSH port"
                  value={manual.port}
                  onChange={(e) => setManual((m) => ({ ...m, port: e.target.value }))}
                />
                <input
                  type="text"
                  className="setting-source-input"
                  placeholder="user"
                  aria-label="SSH user"
                  value={manual.user}
                  onChange={(e) => setManual((m) => ({ ...m, user: e.target.value }))}
                />
                <input
                  type="text"
                  className="setting-source-input"
                  placeholder="source id"
                  aria-label="SSH source id"
                  value={idDraft}
                  onChange={(e) => setIdDraft(e.target.value)}
                />
              </div>
              <div className="setting-ssh-manual-auth">
                <Menu
                  label={`SSH auth mode: ${manual.authMode}`}
                  renderTrigger={(triggerProps) => (
                    <button type="button" className="secondary-button setting-provider-trigger" {...triggerProps}>
                      <span className="setting-provider-name">{manual.authMode === "key" ? "Key file" : "ssh-agent"}</span>
                      <span className="setting-provider-caret" aria-hidden="true">▾</span>
                    </button>
                  )}
                >
                  <MenuItem
                    checked={manual.authMode === "agent"}
                    active={manual.authMode === "agent"}
                    onClick={() => setManual((m) => ({ ...m, authMode: "agent" }))}
                  >
                    ssh-agent
                  </MenuItem>
                  <MenuItem
                    checked={manual.authMode === "key"}
                    active={manual.authMode === "key"}
                    onClick={() => setManual((m) => ({ ...m, authMode: "key" }))}
                  >
                    Key file
                  </MenuItem>
                </Menu>
                {manual.authMode === "key" ? (
                  <input
                    type="text"
                    className="setting-source-input"
                    placeholder="key path (~/.ssh/id_ed25519)"
                    aria-label="SSH key path"
                    value={manual.keyPath}
                    onChange={(e) => setManual((m) => ({ ...m, keyPath: e.target.value }))}
                  />
                ) : null}
              </div>
              <AliasTestAdd
                idDraft=""
                idPlaceholder="host"
                idLocked
                testing={testing}
                testResult={testResult}
                canTest={Boolean(draft)}
                canAdd={Boolean(draft)}
                onIdChange={() => {}}
                onTest={() => void runTest()}
                onAdd={add}
              />
            </div>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

/** Shared tail of both add paths: test button + outcome + add button. */
function AliasTestAdd({
  idDraft,
  idPlaceholder,
  idLocked,
  testing,
  testResult,
  canTest,
  canAdd,
  onIdChange,
  onTest,
  onAdd,
}: {
  idDraft: string;
  idPlaceholder: string;
  idLocked: boolean;
  testing: boolean;
  testResult: SshTestResult | null;
  canTest: boolean;
  canAdd: boolean;
  onIdChange: (value: string) => void;
  onTest: () => void;
  onAdd: () => void;
}) {
  return (
    <div className="setting-ssh-test-add">
      {idLocked ? null : (
        <input
          type="text"
          className="setting-source-input setting-ssh-id-input"
          placeholder={idPlaceholder}
          aria-label="SSH source id"
          value={idDraft}
          onChange={(e) => onIdChange(e.target.value)}
        />
      )}
      <button type="button" className="secondary-button" disabled={!canTest || testing} onClick={onTest}>
        {testing ? "Testing…" : "Test connection"}
      </button>
      {testResult ? (
        testResult.ok ? (
          <span className="setting-ssh-test-ok" role="status">
            ✓ Connected — {testResult.sessionCount ?? 0} session{testResult.sessionCount === 1 ? "" : "s"} found
          </span>
        ) : (
          <span className="setting-error setting-ssh-test-error" role="alert">
            {testResult.error ?? "Test failed"}
          </span>
        )
      ) : null}
      <button type="button" className="primary-button setting-ssh-add-confirm" disabled={!canAdd} onClick={onAdd}>
        Add
      </button>
    </div>
  );
}
