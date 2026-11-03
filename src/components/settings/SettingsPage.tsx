import { useCallback, useEffect, useId, useMemo, useRef, useState, type FC } from "react";
import { useQueryClient } from "@tanstack/react-query";
import {
  setSettingValue,
  type SettingDefinition,
  type SettingType,
  type SettingValue,
  type SettingsSnapshot,
  type SourceEntry,
} from "@/lib/api/settings";
import { queryKeys } from "@/lib/query/keys";
import { useSettingsQuery } from "@/lib/query/queries";
import { SettingRow } from "@/components/ui/SettingRow";
import { SourcesEditor } from "@/components/ui/SourcesEditor";
import { ToggleRow } from "@/components/ui/ToggleRow";
import {
  DEFAULT_RESUME_TEMPLATE,
  RESUME_TEMPLATE_EXAMPLE,
  renderResumeTemplate,
} from "@/lib/resumeTemplate";

/**
 * Settings overlay page: a pure renderer over the
 * settings core. Descriptors/groups arrive via `get_settings`;
 * labels/order live here in TS. Every control change writes through
 * `setSettingValue` per key (auto-apply, no Save/Cancel) with an optimistic
 * React Query cache update; the app-level `settings-changed` listener handles
 * refetch invalidation.
 */

/** Category order + labels (TS-side presentation over descriptor `group`s). */
const CATEGORY_ORDER: { group: string; label: string }[] = [
  { group: "update", label: "General" },
  { group: "sources", label: "Sources" },
  { group: "sessions", label: "Sessions" },
];

/** Insertable placeholder a string setting may declare (chips under the input). */
interface SettingVariable {
  /** Bare name; the chip inserts `{name}` at the textarea caret. */
  name: string;
  /** One-line meaning shown as the chip's tooltip. */
  hint: string;
  /** Concrete rendered example for the tooltip. */
  example: string;
}

/** Per-setting presentation (user-side copy, never raw keys). */
const SETTING_PRESENTATION: Record<
  string,
  {
    label: string;
    description?: string;
    variables?: SettingVariable[];
    /** Renders the live result under the editor (draft value in, preview out). */
    preview?: (value: string) => string;
  }
> = {
  "update.autoCheck": {
    label: "Check for updates automatically",
    description: "When off, the startup update check is skipped. Manual retry from the update toast still works.",
  },
  sources: {
    label: "Extra session sources",
    description: "Additional home-mirrored roots scanned on top of this machine's home.",
  },
  "sessions.resumeTemplateEnabled": {
    label: "Resume copy template",
    description: "When on, the Resume row shows and copies the rendered template below.",
  },
  "sessions.resumeTemplate": {
    label: "Template",
    variables: [
      {
        name: "ResumeCommand",
        hint: "The agent's resume command",
        example: RESUME_TEMPLATE_EXAMPLE.resumeCommand,
      },
      {
        name: "WorkDir",
        hint: "The session's project directory",
        example: RESUME_TEMPLATE_EXAMPLE.workDir,
      },
      {
        name: "SessionId",
        hint: "The session id",
        example: RESUME_TEMPLATE_EXAMPLE.sessionId,
      },
    ],
    preview: (value) => renderResumeTemplate(value, RESUME_TEMPLATE_EXAMPLE),
  },
};

interface SettingRendererProps {
  def: SettingDefinition;
  value: SettingValue | undefined;
  error: string | null;
  onChange: (next: SettingValue) => void;
}

type SettingRenderer = FC<SettingRendererProps>;

const BoolRenderer: SettingRenderer = ({ def, value, error, onChange }) => {
  const bool = value !== undefined && "bool" in value ? value.bool : "bool" in def.default ? def.default.bool : false;
  return (
    <ToggleRow
      label={SETTING_PRESENTATION[def.key]?.label ?? def.key}
      description={SETTING_PRESENTATION[def.key]?.description}
      value={bool}
      error={error}
      onChange={(next) => onChange({ bool: next })}
    />
  );
};

const SourceListRenderer: SettingRenderer = ({ def, value, error, onChange }) => {
  const entries: SourceEntry[] =
    value !== undefined && "sourceList" in value
      ? value.sourceList
      : "sourceList" in def.default
        ? def.default.sourceList
        : [];
  return (
    <SourcesEditor
      label={SETTING_PRESENTATION[def.key]?.label ?? def.key}
      description={SETTING_PRESENTATION[def.key]?.description}
      value={entries}
      error={error}
      onChange={(next) => onChange({ sourceList: next })}
    />
  );
};

/**
 * Multi-line text renderer for `string` settings — what you type is what is
 * saved: every change applies immediately (local file write, no commit
 * gesture); Enter is a plain newline, Escape just bubbles up and closes the
 * page. Settings that declare `variables` get a chip row that inserts
 * `{name}` at the textarea caret, and a `preview` renders the live result
 * under the editor so the effect is visible without leaving Settings.
 */
const StringRenderer: SettingRenderer = ({ def, value, error, onChange }) => {
  const id = useId();
  const str =
    value !== undefined && "string" in value ? value.string : "string" in def.default ? def.default.string : "";
  // Mirror of the served value; equals it except mid-write (apply-on-change
  // reconciles on the next settings-changed echo).
  const [draft, setDraft] = useState(str);
  useEffect(() => {
    setDraft(str);
  }, [str]);
  const set = (next: string) => {
    setDraft(next);
    onChange({ string: next });
  };
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);
  const insertVariable = (name: string) => {
    const token = `{${name}}`;
    const el = textareaRef.current;
    if (!el) {
      set(draft + token);
      return;
    }
    const start = el.selectionStart ?? draft.length;
    const end = el.selectionEnd ?? start;
    set(draft.slice(0, start) + token + draft.slice(end));
    // Caret lands AFTER the token once the re-render paints it.
    requestAnimationFrame(() => {
      el.focus();
      const caret = start + token.length;
      el.setSelectionRange(caret, caret);
    });
  };
  const presentation = SETTING_PRESENTATION[def.key];
  return (
    <SettingRow
      label={presentation?.label ?? def.key}
      description={presentation?.description}
      id={id}
      error={error}
      variant="full"
    >
      <textarea
        ref={textareaRef}
        id={id}
        className="setting-string-input"
        rows={3}
        value={draft}
        spellCheck={false}
        aria-invalid={error ? true : undefined}
        onChange={(event) => set(event.target.value)}
      />
      {presentation?.variables ? (
        <div className="setting-variable-chips">
          {presentation.variables.map((variable) => (
            <button
              key={variable.name}
              type="button"
              className="setting-variable-chip"
              aria-label={`Insert {${variable.name}}`}
              title={`${variable.hint} — e.g. ${variable.example}`}
              onClick={() => insertVariable(variable.name)}
            >
              {`{${variable.name}}`}
            </button>
          ))}
        </div>
      ) : null}
      {presentation?.preview ? (
        <div className="setting-value-preview" title="Preview with example values">
          {presentation.preview(draft)}
        </div>
      ) : null}
    </SettingRow>
  );
};

/**
 * v1 renderers cover the shipped setting types (`bool`, `string`,
 * `sourceList`). `stringList` is deferred until a setting uses it — an
 * unknown or missing type renders the visible fallback row below, never a
 * crash.
 */
const RENDERERS: Partial<Record<SettingType, SettingRenderer>> = {
  bool: BoolRenderer,
  string: StringRenderer,
  sourceList: SourceListRenderer,
};

export function SettingsPage({ onClose }: { onClose: () => void }) {
  const queryClient = useQueryClient();
  const settingsQuery = useSettingsQuery();
  const [activeGroup, setActiveGroup] = useState<string | null>(null);
  const [errors, setErrors] = useState<Record<string, string | null>>({});

  // Page-level Esc. Inner surfaces handle their own Esc first: menus call
  // preventDefault (checked here), and an open confirm dialog owns the key.
  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || event.defaultPrevented) return;
      if (document.querySelector(".confirm-dialog")) return;
      onClose();
    };
    document.addEventListener("keydown", handler);
    return () => document.removeEventListener("keydown", handler);
  }, [onClose]);

  const snapshot: SettingsSnapshot | undefined = settingsQuery.data;

  const categories = useMemo(() => {
    if (!snapshot) return [];
    const groups: string[] = [];
    for (const def of snapshot.descriptors) {
      if (!groups.includes(def.group)) groups.push(def.group);
    }
    const known = CATEGORY_ORDER.filter((c) => groups.includes(c.group));
    const extras = groups
      .filter((g) => !CATEGORY_ORDER.some((c) => c.group === g))
      .sort((a, b) => a.localeCompare(b))
      .map((g) => ({ group: g, label: g }));
    return [...known, ...extras];
  }, [snapshot]);

  const active = activeGroup && categories.some((c) => c.group === activeGroup) ? activeGroup : categories[0]?.group;

  const handleChange = useCallback(
    async (key: string, value: SettingValue) => {
      setErrors((prev) => ({ ...prev, [key]: null }));
      // Optimistic per-key cache update; the settings-changed listener's
      // invalidation still triggers the authoritative refetch.
      const prevSnapshot = queryClient.getQueryData<SettingsSnapshot>(queryKeys.settings());
      if (prevSnapshot) {
        queryClient.setQueryData(queryKeys.settings(), {
          ...prevSnapshot,
          values: { ...prevSnapshot.values, [key]: value },
        });
      }
      try {
        // Toggle-on seeding: enabling the resume template with an EMPTY
        // template fills the recommended default once (persisted like any
        // edit, mirrored into the cache so the editor shows it immediately)
        // — the feature works out of the box; custom templates are never
        // overwritten.
        if (key === "sessions.resumeTemplateEnabled" && "bool" in value && value.bool) {
          const template = prevSnapshot?.values["sessions.resumeTemplate"];
          const empty = !template || !("string" in template) || !template.string.trim();
          if (empty) {
            await setSettingValue("sessions.resumeTemplate", { string: DEFAULT_RESUME_TEMPLATE });
            queryClient.setQueryData<SettingsSnapshot>(queryKeys.settings(), (current: SettingsSnapshot | undefined) =>
              current
                ? {
                    ...current,
                    values: {
                      ...current.values,
                      "sessions.resumeTemplate": { string: DEFAULT_RESUME_TEMPLATE },
                    },
                  }
                : current,
            );
          }
        }
        await setSettingValue(key, value);
      } catch (err) {
        setErrors((prev) => ({ ...prev, [key]: err instanceof Error ? err.message : String(err) }));
        if (prevSnapshot) {
          queryClient.setQueryData(queryKeys.settings(), prevSnapshot);
        }
      }
    },
    [queryClient],
  );

  const descriptors = snapshot?.descriptors.filter((def) => def.group === active) ?? [];

  return (
    <div
      className="settings-overlay"
      role="presentation"
      onClick={onClose}
    >
      <section
        className="settings-page"
        role="dialog"
        aria-modal="true"
        aria-label="Settings"
        onClick={(event) => event.stopPropagation()}
      >
        <header className="settings-header">
          <button type="button" className="ghost-button settings-back" onClick={onClose} aria-label="Close settings">
            ← Back
          </button>
          <h1 className="settings-title">{categories.find((c) => c.group === active)?.label ?? "Settings"}</h1>
        </header>
        <div className="settings-body">
          <nav className="settings-sidebar" aria-label="Settings categories">
            {categories.map((category) => (
              <button
                key={category.group}
                type="button"
                className={`settings-nav-button${category.group === active ? " active" : ""}`}
                aria-current={category.group === active ? "true" : undefined}
                onClick={() => setActiveGroup(category.group)}
              >
                {category.label}
              </button>
            ))}
          </nav>
          <div className="settings-content">
            {settingsQuery.isLoading ? (
              <div className="settings-state">Loading settings…</div>
            ) : settingsQuery.isError ? (
              <div className="settings-state settings-state--error" role="alert">
                Failed to load settings: {settingsQuery.error instanceof Error ? settingsQuery.error.message : "unknown error"}
              </div>
            ) : snapshot === undefined ? null : (
              descriptors.map((def) => {
                const Renderer = RENDERERS[def.type];
                if (!Renderer) {
                  return (
                    <SettingRow
                      key={def.key}
                      label={SETTING_PRESENTATION[def.key]?.label ?? def.key}
                      description={def.key}
                    >
                      <div className="setting-unsupported">Unsupported setting type: {def.type}</div>
                    </SettingRow>
                  );
                }
                return (
                  <Renderer
                    key={def.key}
                    def={def}
                    value={snapshot.values[def.key]}
                    error={errors[def.key] ?? null}
                    onChange={(next) => void handleChange(def.key, next)}
                  />
                );
              })
            )}
          </div>
        </div>
      </section>
    </div>
  );
}
