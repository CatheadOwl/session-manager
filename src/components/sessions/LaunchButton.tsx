import { useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Play, X } from "lucide-react";

interface LaunchButtonProps {
  /** The rendered command line to execute (Rust `run_shell_command`). */
  command: string;
  /**
   * The session's project directory — becomes the spawned process's
   * working directory (Rust sets it natively on every platform; only
   * Windows default templates need no cd, macOS/Linux ones carry their
   * own).
   */
  workDir?: string | null;
  label?: string;
  className?: string;
}

/**
 * Execute a rendered launch-template command line in the user's terminal.
 * Detached fire-and-forget: success leaves no trace in the UI (a terminal
 * window appears); a local spawn failure (no shell, empty line) flips the
 * icon to ✗ for a beat with the error in the tooltip. The tooltip when idle
 * shows the exact line that will run — what you hover is what launches.
 */
export function LaunchButton({ command, workDir, label = "Launch resume command", className }: LaunchButtonProps) {
  const [failed, setFailed] = useState<string | null>(null);
  const resetTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const Icon = failed ? X : Play;
  const statusLabel = failed ? "Launch failed" : label;

  const launch = async () => {
    if (resetTimer.current) clearTimeout(resetTimer.current);
    setFailed(null);
    try {
      await invoke("run_shell_command", { command, workDir: workDir ?? undefined });
    } catch (err) {
      setFailed(err instanceof Error ? err.message : String(err));
      resetTimer.current = setTimeout(() => setFailed(null), 2500);
    }
  };

  return (
    <button
      type="button"
      className={`ghost-button copy-feedback-button${className ? ` ${className}` : ""}`}
      onClick={() => void launch()}
      aria-label={statusLabel}
      title={failed ?? `${label}: ${command}`}
    >
      <Icon aria-hidden="true" size={14} strokeWidth={2} />
    </button>
  );
}
