import { memo, useCallback, useState } from "react";
import { CalendarClock, ChevronDown, Download } from "lucide-react";
import { DayPicker, type DateRange } from "react-day-picker";
import { useClickOutside } from "@/hooks/useClickOutside";
import type { QaExportStatus } from "@/hooks/useQaExport"; // type-only: busy state for the button
import {
  dayStartToEpochMs,
  resolveTimeRange,
  type TimeRange,
  type TimeRangePreset,
} from "@/utils/time-range";
import "react-day-picker/dist/style.css";

const PRESETS: Array<{ value: TimeRangePreset; label: string }> = [
  { value: "all", label: "All time" },
  { value: "today", label: "Today" },
  { value: "7d", label: "Last 7 days" },
  { value: "30d", label: "Last 30 days" },
  { value: "custom", label: "Custom range…" },
];

interface ExportQaControlsProps {
  timeRange: TimeRange;
  onPresetChange: (preset: TimeRangePreset) => void;
  onCustomRange: (from: number, to: number) => void;
  onExport: () => void;
  exportStatus: QaExportStatus;
}

/**
 * Time-range selector + export button for the SessionList toolbar.
 * Pure props-driven UI: filtering lives in useSessionQueries, export
 * orchestration in useQaExport, distill logic in Rust.
 */
export const ExportQaControls = memo(function ExportQaControls({
  timeRange,
  onPresetChange,
  onCustomRange,
  onExport,
  exportStatus,
}: ExportQaControlsProps) {
  const [calendarOpen, setCalendarOpen] = useState(false);
  const [draftRange, setDraftRange] = useState<DateRange | undefined>(undefined);
  const calendarRef = useClickOutside<HTMLDivElement>({
    isOpen: calendarOpen,
    onClose: () => setCalendarOpen(false),
  });

  const handlePresetChange = useCallback(
    (preset: TimeRangePreset) => {
      if (preset === "custom") {
        setCalendarOpen(true);
        return;
      }
      setCalendarOpen(false);
      onPresetChange(preset);
    },
    [onPresetChange],
  );

  const applyDraftRange = useCallback(() => {
    if (!draftRange?.from || !draftRange.to) return; // require both ends before applying
    onCustomRange(dayStartToEpochMs(draftRange.from), dayStartToEpochMs(draftRange.to) + 86_399_999);
    setCalendarOpen(false);
  }, [draftRange, onCustomRange]);

  const isExporting = exportStatus.state === "exporting";
  // Export needs a concrete window; "All time" (or an incomplete custom
  // range) resolves to null — disable instead of failing after the click.
  const hasResolvedRange = resolveTimeRange(timeRange) !== null;

  const presetLabel =
    timeRange.preset === "custom" && timeRange.customFrom !== undefined
      ? "Custom range"
      : (PRESETS.find((p) => p.value === timeRange.preset)?.label ?? "All time");

  return (
    <div className="export-row">
      <div className="export-row-main" ref={calendarRef}>
        <div
          className={`export-range-pill${hasResolvedRange ? " is-filtering" : ""}`}
          title={hasResolvedRange ? `List filtered by: ${presetLabel}` : undefined}
        >
          <CalendarClock size={13} className="export-range-icon" aria-hidden="true" />
          <select
            id="export-time-range"
            className="export-range-select"
            value={timeRange.preset}
            onChange={(event) => handlePresetChange(event.target.value as TimeRangePreset)}
            aria-label="Filter sessions by time"
          >
            {PRESETS.map((preset) => (
              <option key={preset.value} value={preset.value}>
                {preset.label}
              </option>
            ))}
          </select>
          <ChevronDown size={13} className="export-range-chevron" aria-hidden="true" />
        </div>

        {calendarOpen ? (
          <div className="export-calendar-popover" role="dialog" aria-label="Custom time range">
            <DayPicker
              mode="range"
              selected={draftRange}
              onSelect={(range) => setDraftRange(range)}
              numberOfMonths={2}
            />
            <div className="export-calendar-actions">
              <button
                type="button"
                className="secondary-button"
                onClick={() => setCalendarOpen(false)}
              >
                Cancel
              </button>
              <button
                type="button"
                className="primary-button"
                onClick={applyDraftRange}
                disabled={!draftRange?.from || !draftRange.to}
              >
                Apply
              </button>
            </div>
          </div>
        ) : null}
      </div>

      <button
        type="button"
        className="secondary-button export-button"
        onClick={onExport}
        disabled={isExporting || !hasResolvedRange}
        title={
          hasResolvedRange
            ? `Export Q&A sessions (${presetLabel}) to a file`
            : "Pick a time range first — “All time” cannot be exported"
        }
      >
        <Download size={14} aria-hidden="true" />
        {isExporting ? "Exporting…" : "Export Q&A"}
      </button>
      {/* Result feedback lives in ExportToast (bottom-right); this toolbar
          only conveys busy state via the button itself. */}
    </div>
  );
});
