import type { ReactNode } from "react";

export interface SettingRowProps {
  /** Human label for the setting (TS-side presentation, never the raw key). */
  label: string;
  /** Optional one-line explanation under the label. */
  description?: string;
  /** The control (toggle, editor, …) rendered to the right of the label. */
  children: ReactNode;
  /** Associates the label with a focusable control (`htmlFor` semantics). */
  id?: string;
  /** Display-only inline error surfaced on this row (failed write, invalid value). */
  error?: string | null;
}

/**
 * UI primitive: the shared shell of every settings row — label, optional
 * description, control slot, and an inline error line. Self-sufficient
 * `.setting-*` classes live in styles/ui.css (ADR 0004 constraints:
 * typography baked in, tokens only). Renderers own the control, never the
 * row rhythm.
 */
export function SettingRow({ label, description, children, id, error }: SettingRowProps) {
  return (
    <div className="setting-row">
      <div className="setting-row-text">
        <label className="setting-label" htmlFor={id}>
          {label}
        </label>
        {description ? <div className="setting-description">{description}</div> : null}
        {error ? (
          <div className="setting-error" role="alert">
            {error}
          </div>
        ) : null}
      </div>
      <div className="setting-control">{children}</div>
    </div>
  );
}
