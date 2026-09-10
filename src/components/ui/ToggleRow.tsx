import { useId } from "react";
import { SettingRow } from "./SettingRow";

export interface ToggleRowProps {
  label: string;
  description?: string;
  value: boolean;
  onChange: (next: boolean) => void;
  /** Display-only inline error surfaced on the row. */
  error?: string | null;
  disabled?: boolean;
}

/**
 * UI primitive: accessible boolean switch for settings rows. `role="switch"`
 * button with `aria-checked`; Space/Enter toggle through an explicit keydown
 * handler (preventDefault suppresses the browser's synthetic click so the
 * toggle fires exactly once); focus-visible ring from tokens
 * (`.setting-toggle` in ui.css).
 */
export function ToggleRow({ label, description, value, onChange, error, disabled }: ToggleRowProps) {
  const id = useId();
  return (
    <SettingRow label={label} description={description} id={id} error={error}>
      <button
        id={id}
        type="button"
        role="switch"
        aria-checked={value}
        aria-label={label}
        className="setting-toggle"
        disabled={disabled}
        onClick={() => onChange(!value)}
        onKeyDown={(event) => {
          if (event.key === "Enter" || event.key === " ") {
            event.preventDefault();
            onChange(!value);
          }
        }}
      >
        <span className="setting-toggle-knob" aria-hidden="true" />
      </button>
    </SettingRow>
  );
}
