import { memo, useEffect, useState } from "react";
import { useClickOutside } from "@/hooks/useClickOutside";
import { getBaseName } from "@/utils/format";
import type { FolderGroup } from "@/lib/domain";
import type { UpdateStatus } from "@/hooks/useUpdater";
import { SegmentedControl } from "./SegmentedControl";
import { UpdateToast } from "./UpdateToast";

interface FolderFilterProps {
  folders: FolderGroup[];
  selectedFolder: string;
  onSelectFolder: (folder: string) => void;
  pinnedFolders: string[];
  onTogglePin: (folder: string) => void;
  isCollapsed: boolean;
  onToggleCollapse: () => void;
  scope: "active" | "archived";
  onScopeChange: (scope: "active" | "archived") => void;
  onArchiveFolder: (folder: string) => void;
  onRestoreFolder: (folder: string) => void;
  isFolderOperationPending: boolean;
  updateStatus: UpdateStatus;
  updateVersion?: string;
  onInstallUpdate: () => void;
}

type FolderSortKey = "recent" | "alpha";

const SORT_OPTIONS: { value: FolderSortKey; label: string }[] = [
  { value: "recent", label: "Recent" },
  { value: "alpha", label: "A-Z" },
];

// Exhaustive map: adding a FolderSortKey without a comparator is a compile error.
const SORT_COMPARATORS: Record<FolderSortKey, (a: FolderGroup, b: FolderGroup) => number> = {
  recent: (a, b) => b.lastActiveAt - a.lastActiveAt || a.name.localeCompare(b.name),
  alpha: (a, b) => a.name.localeCompare(b.name),
};

export const FolderFilter = memo(function FolderFilter({
  folders,
  selectedFolder,
  onSelectFolder,
  pinnedFolders,
  onTogglePin,
  isCollapsed,
  onToggleCollapse,
  scope,
  onScopeChange,
  onArchiveFolder,
  onRestoreFolder,
  isFolderOperationPending,
  updateStatus,
  updateVersion,
  onInstallUpdate,
}: FolderFilterProps) {
  const [isActionsOpen, setIsActionsOpen] = useState(false);
  const [isSortOpen, setIsSortOpen] = useState(false);
  const [sortKey, setSortKey] = useState<FolderSortKey>("recent");
  const actionsRef = useClickOutside<HTMLDivElement>({
    isOpen: isActionsOpen,
    onClose: () => setIsActionsOpen(false),
  });
  const sortRef = useClickOutside<HTMLDivElement>({
    isOpen: isSortOpen,
    onClose: () => setIsSortOpen(false),
  });
  const canMoveSelectedFolder = selectedFolder !== "all" && selectedFolder !== "Unknown";
  const folderActionLabel = scope === "active" ? "Archive folder" : "Restore folder";

  useEffect(() => {
    setIsActionsOpen(false);
    setIsSortOpen(false);
  }, [selectedFolder, scope, isCollapsed]);

  const handleSortSelect = (key: FolderSortKey) => {
    setSortKey(key);
    setIsSortOpen(false);
  };

  const handleFolderAction = () => {
    if (!canMoveSelectedFolder || isFolderOperationPending) return;
    setIsActionsOpen(false);
    if (scope === "active") {
      onArchiveFolder(selectedFolder);
    } else {
      onRestoreFolder(selectedFolder);
    }
  };

  if (isCollapsed) {
    return (
      <div className="folder-column collapsed">
        <button
          type="button"
          className="folder-expand-btn"
          onClick={onToggleCollapse}
          title={selectedFolder === "all" ? "Expand folder panel" : `Expand folder panel: ${selectedFolder}`}
          aria-label="Expand folder panel"
        >
          <PanelToggleIcon direction="expand" />
        </button>
        <UpdateToast
          status={updateStatus}
          version={updateVersion}
          onInstall={onInstallUpdate}
        />
      </div>
    );
  }

  const totalCount = folders.reduce((sum, f) => sum + f.count, 0);
  const pinned = sortFolders(
    pinnedFolders
      .map((name) => folders.find((f) => f.name === name))
      .filter((f): f is FolderGroup => f !== undefined),
    sortKey,
  );
  const unpinned = sortFolders(
    folders.filter((f) => !pinnedFolders.includes(f.name)),
    sortKey,
  );

  return (
    <div className="folder-column">
      <div className="folder-header">
        <span className="folder-header-title">Folders</span>
        <button
          type="button"
          className="folder-collapse-btn"
          onClick={onToggleCollapse}
          title="Collapse folder panel"
          aria-label="Collapse folder panel"
        >
          <PanelToggleIcon direction="collapse" />
          {updateStatus === "available" && (
            <span className="update-dot update-dot--header" />
          )}
        </button>
      </div>

      <div className="folder-scope-block">
        <div className="folder-scope-row">
          <div className="scope-toggle">
            <SegmentedControl
              options={[{ value: "active" as const, label: "Active" }, { value: "archived" as const, label: "Archived" }]}
              value={scope}
              onChange={(v) => onScopeChange(v)}
            />
          </div>
          <div className="folder-actions" ref={actionsRef}>
            <button
              type="button"
              className="folder-actions-trigger"
              onClick={() => setIsActionsOpen((open) => !open)}
              aria-label="Folder actions"
              aria-haspopup="menu"
              aria-expanded={isActionsOpen}
              aria-controls="folder-actions-menu"
              title="Folder actions"
            >
              <MoreHorizontalIcon />
            </button>
            {isActionsOpen ? (
              <div className="folder-actions-menu" id="folder-actions-menu" role="menu">
                <button
                  type="button"
                  className="folder-actions-item"
                  role="menuitem"
                  onClick={handleFolderAction}
                  disabled={!canMoveSelectedFolder || isFolderOperationPending}
                  title={canMoveSelectedFolder ? `${folderActionLabel}: ${selectedFolder}` : "Select a folder first"}
                >
                  {folderActionLabel}
                </button>
              </div>
            ) : null}
          </div>
        </div>
      </div>

      <div className="folder-sort" ref={sortRef}>
        <button
          type="button"
          className="folder-sort-trigger"
          onClick={() => setIsSortOpen((open) => !open)}
          aria-haspopup="menu"
          aria-expanded={isSortOpen}
          aria-controls="folder-sort-menu"
          aria-label={`Sort folders: ${SORT_OPTIONS.find((o) => o.value === sortKey)?.label ?? sortKey}`}
        >
          <span className="folder-sort-label">
            {SORT_OPTIONS.find((o) => o.value === sortKey)?.label ?? sortKey}
          </span>
          <ChevronDownIcon />
        </button>
        {isSortOpen ? (
          <div className="folder-sort-menu" id="folder-sort-menu" role="menu">
            {SORT_OPTIONS.map((option) => (
              <button
                key={option.value}
                type="button"
                className={`folder-sort-item${sortKey === option.value ? " active" : ""}`}
                role="menuitemradio"
                aria-checked={sortKey === option.value}
                onClick={() => handleSortSelect(option.value)}
              >
                {option.label}
              </button>
            ))}
          </div>
        ) : null}
      </div>

      <div className={`folder-item${selectedFolder === "all" ? " selected" : ""}`}>
        <span
          className="folder-item-label"
          role="button"
          tabIndex={0}
          onClick={() => onSelectFolder("all")}
          onKeyDown={(e) => { if (e.key === "Enter" || e.key === " ") onSelectFolder("all"); }}
        >
          <span className="folder-item-text">
            <span className="folder-item-name">All</span>
          </span>
          <span className="folder-count">{totalCount}</span>
        </span>
      </div>

      {pinned.length > 0 && (
        <div className="folder-section-label">Pinned</div>
      )}
      {pinned.map((f) => (
        <FolderItem
          key={f.name}
          folder={f}
          selected={selectedFolder === f.name}
          pinned
          onSelectFolder={onSelectFolder}
          onTogglePin={onTogglePin}
        />
      ))}

      {unpinned.length > 0 && (
        <div className="folder-section-label">Folders</div>
      )}
      {unpinned.map((f) => (
        <FolderItem
          key={f.name}
          folder={f}
          selected={selectedFolder === f.name}
          pinned={false}
          onSelectFolder={onSelectFolder}
          onTogglePin={onTogglePin}
        />
      ))}
    </div>
  );
});

interface FolderItemProps {
  folder: FolderGroup;
  selected: boolean;
  pinned: boolean;
  onSelectFolder: (folder: string) => void;
  onTogglePin: (folder: string) => void;
}

function FolderItem({
  folder,
  selected,
  pinned,
  onSelectFolder,
  onTogglePin,
}: FolderItemProps) {
  const display = getFolderDisplay(folder.name);

  return (
    <div className={`folder-item${selected ? " selected" : ""}${pinned ? " pinned" : ""}`}>
      <span
        className="folder-item-label"
        role="button"
        tabIndex={0}
        title={folder.name}
        onClick={() => onSelectFolder(folder.name)}
        onKeyDown={(e) => { if (e.key === "Enter" || e.key === " ") onSelectFolder(folder.name); }}
      >
        <span className="folder-item-text">
          <span className="folder-item-name">{display.primary}</span>
          {display.secondary ? <span className="folder-item-path">{display.secondary}</span> : null}
        </span>
        <span className="folder-action-slot">
          <span className="folder-count">{folder.count}</span>
          <button
            type="button"
            className="folder-pin-btn"
            onClick={(e) => {
              e.stopPropagation();
              onTogglePin(folder.name);
            }}
            title={pinned ? "Unpin folder" : "Pin folder"}
          >
            {getPinIcon(pinned)}
          </button>
        </span>
      </span>
    </div>
  );
}

function MoreHorizontalIcon() {
  return (
    <svg
      className="folder-actions-icon"
      viewBox="0 0 24 24"
      aria-hidden="true"
      focusable="false"
    >
      <circle cx="6.5" cy="12" r="1.35" />
      <circle cx="12" cy="12" r="1.35" />
      <circle cx="17.5" cy="12" r="1.35" />
    </svg>
  );
}

function ChevronDownIcon() {
  return (
    <svg
      className="folder-sort-icon"
      viewBox="0 0 24 24"
      aria-hidden="true"
      focusable="false"
    >
      <path d="M6 9l6 6 6-6" />
    </svg>
  );
}

function PanelToggleIcon({ direction }: { direction: "collapse" | "expand" }) {
  return (
    <svg
      className="folder-toggle-icon"
      viewBox="0 0 24 24"
      aria-hidden="true"
      focusable="false"
    >
      <rect className="folder-toggle-icon-border" x="3" y="4" width="18" height="16" rx="2" />
      <line className="folder-toggle-icon-divider" x1="9" y1="4" x2="9" y2="20" />
      {direction === "expand" && (
        <rect className="folder-toggle-icon-fill" x="3" y="4" width="6" height="16" rx="2" />
      )}
    </svg>
  );
}

function getFolderDisplay(folder: string) {
  if (folder === "Unknown") {
    return { primary: "Unknown", secondary: "" };
  }

  const primary = getBaseName(folder) || folder;
  return {
    primary,
    secondary: primary === folder ? "" : folder,
  };
}

function getPinIcon(pinned: boolean) {
  return pinned ? "\u{1F4CD}" : "\u{1F4CC}";
}

function sortFolders(list: FolderGroup[], sortKey: FolderSortKey): FolderGroup[] {
  return [...list].sort(SORT_COMPARATORS[sortKey]);
}
