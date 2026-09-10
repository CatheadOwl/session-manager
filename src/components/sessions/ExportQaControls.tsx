import { memo, useCallback, useState } from "react";
import { CalendarClock, Check, ChevronDown, Download } from "lucide-react";
import { DayPicker, type DateRange } from "react-day-picker";
import { Menu, MenuItem } from "@/components/ui/Menu";
import { Popover } from "@/components/ui/Popover";
import type { QaExportStatus } from "@/hooks/useQaExport";
import {
  dayStartToEpochMs,
  resolveExportRange,
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

  const choosePreset = useCallback(
    (preset: TimeRangePreset) => {
      if (preset === "custom") {
        setCalendarOpen(true);
        return;
      }
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
  // Export needs a concrete window; "All time" resolves to a full-history
  // window (allowed — the >50 confirm in useQaExport is the size guard),
  // and only an incomplete custom range resolves to null — disable instead
  // of failing after the click.
  const hasResolvedRange = resolveExportRange(timeRange) !== null;

  const presetLabel =
    timeRange.preset === "custom" && timeRange.customFrom !== undefined
      ? "Custom range"
      : (PRESETS.find((p) => p.value === timeRange.preset)?.label ?? "All time");

  return (
    <div className="export-row">
      <div className="export-row-main">
        <Menu
          label="Time range presets"
          className="export-range"
          renderTrigger={(triggerProps) => (
            <button
              type="button"
              className={`export-range-pill${hasResolvedRange ? " is-filtering" : ""}`}
              title={hasResolvedRange ? `List filtered by: ${presetLabel}` : undefined}
              {...triggerProps}
            >
              <CalendarClock size={13} className="export-range-icon" aria-hidden="true" />
              <span className="export-range-value">{presetLabel}</span>
              <ChevronDown size={13} className="export-range-chevron" aria-hidden="true" />
            </button>
          )}
        >
          {PRESETS.map((preset) => (
            <MenuItem
              key={preset.value}
              checked={timeRange.preset === preset.value}
              active={timeRange.preset === preset.value}
              onClick={() => choosePreset(preset.value)}
            >
              <span className="export-range-item-label">{preset.label}</span>
              {timeRange.preset === preset.value ? <Check size={13} aria-hidden="true" /> : null}
            </MenuItem>
          ))}
        </Menu>

        <Popover
          open={calendarOpen}
          onClose={() => setCalendarOpen(false)}
          label="Custom time range"
          className="export-calendar-popover"
        >
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
        </Popover>
      </div>

      <button
        type="button"
        className="secondary-button export-button"
        onClick={onExport}
        disabled={isExporting || !hasResolvedRange}
        title={
          hasResolvedRange
            ? `Export Q&A sessions (${presetLabel}) to a file`
            : "Pick a complete time range first (both dates)"
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
