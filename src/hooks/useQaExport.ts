import { useCallback, useState } from "react";
import { confirm, save } from "@tauri-apps/plugin-dialog";
import { sessionsApi } from "@/lib/api/sessions";
import type { ExportOutcome } from "@/types";
import type { SessionMeta } from "@/types";
import type { ResolvedRange } from "@/utils/time-range";

export interface QaExportStatus {
  state: "idle" | "exporting" | "done" | "error";
  message: string;
}

const DEFAULT_STATUS: QaExportStatus = { state: "idle", message: "" };

/**
 * Large-export threshold: exporting more than this many sessions distills a
 * lot of JSONL in one synchronous command (no progress/cancel, D4), so the
 * user gets a native confirm dialog before the save dialog opens. This is
 * the size guard that replaces the old "All time cannot be exported" rule —
 * it guards the actual cost driver (visible session count), not the time
 * window.
 */
const LARGE_EXPORT_THRESHOLD = 50;

/**
 * Remote-backed sessions (SSH sources) export like local ones since the
 * 20260911 bridge (workunit 20260911-1031-remote-qa-export): the backend
 * fetches their content into the transient cache (first fetch ~0.2s/file,
 * P3 bench; re-exports free) and keeps the remote locator in provenance.
 * A per-item fetch failure surfaces as that session being `skipped`, never
 * an aborted batch — so no pre-filter is needed here anymore.
 */
export const countRemoteSessions = (sessions: SessionMeta[]): number =>
  sessions.filter((session) => session.locator?.kind === "remote").length;

/**
 * Q&A export orchestration: native save dialog → backend export command →
 * status feedback. Contains no distill logic — the Rust core owns that.
 * The exported set is "what you see": the caller passes the resolved export
 * list — the visible session list (folder/search/star/time filters already
 * applied by the UI), narrowed to the checked sessions when selection mode
 * is on. Selection resolution is the caller's job; this hook exports
 * exactly what it receives.
 */
export function useQaExport(scope: "active" | "archived") {
  const [status, setStatus] = useState<QaExportStatus>(DEFAULT_STATUS);

  const exportRange = useCallback(
    async (range: ResolvedRange | null, sessions: SessionMeta[]) => {
      if (!range) {
        setStatus({ state: "error", message: "Choose a complete time range first." });
        return;
      }
      if (sessions.length === 0) {
        setStatus({ state: "error", message: "No visible sessions to export." });
        return;
      }

      // Large exports: native confirm BEFORE the save dialog (the user
      // should know the cost before picking a destination). Cancel aborts
      // silently, same as cancelling the save dialog. Remote sessions add a
      // first-fetch SSH transfer (~0.2s each, P3 bench) — called out in the
      // message so the count alone doesn't understate the cost.
      const remoteCount = countRemoteSessions(sessions);
      if (sessions.length > LARGE_EXPORT_THRESHOLD) {
        const remoteNote =
          remoteCount > 0
            ? ` It includes ${remoteCount} remote session(s) fetched over SSH on first export (cached afterwards).`
            : "";
        const ok = await confirm(
          `Export ${sessions.length} sessions? Large exports take a while and cannot be cancelled mid-run.${remoteNote}`,
          { title: "Large Q&A export", kind: "warning" },
        );
        if (!ok) return;
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
          sessions,
          destPath,
          format,
          // The native save dialog already asked "replace file?" — a returned
          // path with an existing file means the user confirmed replacement.
          overwrite: true,
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
