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
  /** Display-only inline error surfaced on the row (failed write, invalid value). */
  error?: string | null;
  /**
   * Layout variant: `default` keeps the two-column rhythm
   * (label/description left, compact control right — built for toggles);
   * `full` stacks the text on top and gives the control the full row
   * width below it — for large list editors (the sources list) that must
   * not be squeezed into a half-width column.
   */
  variant?: "default" | "full";
}

/**
 * UI primitive: the shared shell of every settings row — label, optional
 * description, control slot, and an inline error line. Self-sufficient
 * `.setting-*` classes live in styles/ui.css (design constraints:
 * typography baked in, tokens only). Renderers own the control, never the
 * row rhythm.
 */
export function SettingRow({ label, description, children, id, error, variant = "default" }: SettingRowProps) {
  const className = variant === "full" ? "setting-row setting-row--full" : "setting-row";
  return (
    <div className={className}>
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
