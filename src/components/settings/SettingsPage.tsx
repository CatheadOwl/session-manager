import { useCallback, useEffect, useMemo, useState, type FC } from "react";
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

/**
 * Settings overlay page (workunit 20260910-1131): a pure renderer over the
 * settings core (ADR 0006). Descriptors/groups arrive via `get_settings`;
 * labels/order live here in TS. Every control change writes through
 * `setSettingValue` per key (auto-apply, no Save/Cancel) with an optimistic
 * React Query cache update; the app-level `settings-changed` listener handles
 * refetch invalidation.
 */

/** Category order + labels (TS-side presentation over descriptor `group`s). */
const CATEGORY_ORDER: { group: string; label: string }[] = [
  { group: "update", label: "General" },
  { group: "sources", label: "Sources" },
];

/** Per-setting presentation (user-side copy, never raw keys). */
const SETTING_PRESENTATION: Record<string, { label: string; description?: string }> = {
  "update.autoCheck": {
    label: "Check for updates automatically",
    description: "When off, the startup update check is skipped. Manual retry from the update toast still works.",
  },
  sources: {
    label: "Extra session sources",
    description: "Additional home-mirrored roots scanned on top of this machine's home.",
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
 * v1 renderers cover the shipped setting types (`bool`, `sourceList`).
 * `string`/`stringList` are deferred until a setting uses them — an unknown
 * or missing type renders the visible fallback row below, never a crash.
 */
const RENDERERS: Partial<Record<SettingType, SettingRenderer>> = {
  bool: BoolRenderer,
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
