import { useCallback, useState } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import { sessionsApi } from "@/lib/api/sessions";
import type { ExportOutcome } from "@/types";
import type { ResolvedRange } from "@/utils/time-range";

export interface QaExportStatus {
  state: "idle" | "exporting" | "done" | "error";
  message: string;
}

const DEFAULT_STATUS: QaExportStatus = { state: "idle", message: "" };

/**
 * Q&A export orchestration: native save dialog → backend export command →
 * status feedback. Contains no distill logic — the Rust core owns that.
 */
export function useQaExport(scope: "active" | "archived") {
  const [status, setStatus] = useState<QaExportStatus>(DEFAULT_STATUS);

  const exportRange = useCallback(
    async (range: ResolvedRange | null) => {
      if (!range) {
        setStatus({ state: "error", message: "Choose a time range first (not “All”)." });
        return;
      }

      const destPath = await save({
        title: "Export Q&A sessions",
        defaultPath: `qa-export-${new Date().toISOString().slice(0, 10)}.json`,
        filters: [
          { name: "JSON", extensions: ["json"] },
          { name: "Markdown", extensions: ["md"] },
        ],
      });
      // Cancelled dialog aborts the export silently.
      if (!destPath) return;

      const format = destPath.toLowerCase().endsWith(".md") ? "markdown" : "json";

      setStatus({ state: "exporting", message: "Exporting…" });
      try {
        const outcome: ExportOutcome = await sessionsApi.exportQaSessions({
          scope,
          from: range.from,
          to: range.to,
          destPath,
          format,
        });
        if (outcome.count === 0) {
          setStatus({
            state: "done",
            message: "No sessions in this time range.",
          });
        } else {
          const skippedNote =
            outcome.skipped.length > 0 ? ` (${outcome.skipped.length} skipped)` : "";
          setStatus({
            state: "done",
            message: `Exported ${outcome.count} session(s)${skippedNote} → ${outcome.destPath}`,
          });
        }
      } catch (error) {
        setStatus({
          state: "error",
          message: error instanceof Error ? error.message : String(error),
        });
      }
    },
    [scope],
  );

  const clearStatus = useCallback(() => setStatus(DEFAULT_STATUS), []);

  return { status, exportRange, clearStatus };
}
