import { useState, useMemo, useEffect, useCallback, useRef, memo } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import type { TreeNodeData } from "@/lib/api/sessions";
import type { SessionMeta } from "@/types";
import { SessionItem } from "./SessionItem";
import { getMetadataKey } from "@/lib/domain";

interface TreeViewProps {
  roots: TreeNodeData[];
  sessionMap: Map<string, SessionMeta>;
  selectedKey: string | null;
  search: string;
  starredMap: Map<string, boolean>;
  onSelect: (session: SessionMeta) => void;
  onForkJump?: (session: SessionMeta, forkAtUser: number) => void;
  starFilterActive?: boolean;
}

/** Flattened row produced by the pre-order walk */
interface FlatTreeItem {
  node: TreeNodeData;
  depth: number;
  /** Belongs to a root group that has children (gets group background) */
  inGroup: boolean;
  /** First visible row of its root group */
  groupFirst: boolean;
  /** Last visible row of its root group */
  groupLast: boolean;
}

/** Get the latest lastActiveAt within a tree node and all its descendants */
function getLatestTime(node: TreeNodeData): number {
  let latest = node.lastActiveAt ?? 0;
  for (const child of node.children) {
    const childTime = getLatestTime(child);
    if (childTime > latest) latest = childTime;
  }
  return latest;
}

/** Collect session keys of all nodes at depth <= maxDepth (for initial auto-expand) */
function collectKeysToDepth(roots: TreeNodeData[], maxDepth: number): Set<string> {
  const keys = new Set<string>();
  function walk(node: TreeNodeData, depth: number) {
    if (depth > maxDepth) return;
    keys.add(node.sessionKey);
    for (const child of node.children) walk(child, depth + 1);
  }
  for (const root of roots) walk(root, 0);
  return keys;
}

/**
 * Pre-order flatten: only descends into expanded nodes, skips invisible ones.
 * Single O(n) pass — the virtualizer then renders just the viewport slice.
 */
function flattenTree(
  roots: TreeNodeData[],
  expandedKeys: Set<string>,
  visibleKeys: Set<string>,
): FlatTreeItem[] {
  const items: FlatTreeItem[] = [];

  for (const root of roots) {
    if (!visibleKeys.has(root.sessionKey)) continue;
    const inGroup = root.children.length > 0;
    const groupStart = items.length;

    items.push({ node: root, depth: 0, inGroup, groupFirst: true, groupLast: false });

    if (inGroup && expandedKeys.has(root.sessionKey)) {
      walkChildren(root, 1);
    }

    if (inGroup) {
      items[items.length - 1].groupLast = true;
      // Single-row group: mark as both first & last for full border-radius
      if (items.length - 1 === groupStart) {
        items[groupStart].groupFirst = true;
      }
    }
  }

  function walkChildren(parent: TreeNodeData, depth: number) {
    for (const child of parent.children) {
      if (!visibleKeys.has(child.sessionKey)) continue;
      items.push({ node: child, depth, inGroup: true, groupFirst: false, groupLast: false });
      if (child.children.length > 0 && expandedKeys.has(child.sessionKey)) {
        walkChildren(child, depth + 1);
      }
    }
  }

  return items;
}

// ─── Visibility pre-computation ─────────────────────────────────────────────

/**
 * Single post-order traversal to determine which nodes are visible.
 * A node is visible if it passes both search and star filters,
 * considering descendant matches (ancestor of a match is also visible).
 */
function computeVisibleKeys(
  roots: TreeNodeData[],
  search: string,
  sessionMap: Map<string, SessionMeta>,
  starredMap: Map<string, boolean>,
  starFilterActive: boolean,
): Set<string> {
  const visible = new Set<string>();
  const needle = search.toLowerCase();

  function walk(node: TreeNodeData): { searchMatch: boolean; starMatch: boolean } {
    // Post-order: process children first
    let childSearchMatch = false;
    let childStarMatch = false;
    for (const child of node.children) {
      const result = walk(child);
      if (result.searchMatch) childSearchMatch = true;
      if (result.starMatch) childStarMatch = true;
    }

    const selfSearchMatch = !needle || node.title.toLowerCase().includes(needle);
    const session = sessionMap.get(node.sessionKey);
    const metaKey = session ? getMetadataKey(session) : null;
    const selfStarMatch = !starFilterActive || (metaKey ? starredMap.has(metaKey) : false);

    const searchOk = selfSearchMatch || childSearchMatch;
    const starOk = selfStarMatch || childStarMatch;

    if (searchOk && starOk) {
      visible.add(node.sessionKey);
    }

    return { searchMatch: searchOk, starMatch: starOk };
  }

  for (const root of roots) {
    walk(root);
  }
  return visible;
}

// ─── TreeView (virtualized) ─────────────────────────────────────────────────

export const TreeView = memo(function TreeView({
  roots,
  sessionMap,
  selectedKey,
  search,
  starredMap,
  onSelect,
  onForkJump,
  starFilterActive = false,
}: TreeViewProps) {
  const scrollRef = useRef<HTMLDivElement>(null);

  // Expanded state lifted to top level so the flatten pass can respect it
  const [expandedKeys, setExpandedKeys] = useState<Set<string>>(
    () => collectKeysToDepth(roots, 1), // auto-expand depth < 2
  );

  // Sort roots by latest activity within each group (newest first)
  const sortedRoots = useMemo(
    () => [...roots].sort((a, b) => getLatestTime(b) - getLatestTime(a)),
    [roots],
  );

  // Auto-expand new shallow nodes that appear on data refresh (never removes user collapses)
  useEffect(() => {
    const shallowKeys = collectKeysToDepth(sortedRoots, 1);
    setExpandedKeys((prev) => {
      let changed = false;
      const next = new Set(prev);
      for (const k of shallowKeys) {
        if (!next.has(k)) {
          next.add(k);
          changed = true;
        }
      }
      return changed ? next : prev;
    });
  }, [sortedRoots]);

  // Pre-compute visible keys in a single O(n) traversal instead of O(n²) per-node recursion
  const visibleKeys = useMemo(
    () => computeVisibleKeys(sortedRoots, search, sessionMap, starredMap, starFilterActive),
    [sortedRoots, search, sessionMap, starredMap, starFilterActive],
  );

  // Flatten visible + expanded nodes into a renderable row list
  const flatItems = useMemo(
    () => flattenTree(sortedRoots, expandedKeys, visibleKeys),
    [sortedRoots, expandedKeys, visibleKeys],
  );

  const virtualizer = useVirtualizer({
    count: flatItems.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => 86,
    overscan: 8,
    getItemKey: (index) => flatItems[index].node.sessionKey,
  });

  const toggleExpand = useCallback((key: string) => {
    setExpandedKeys((prev) => {
      const next = new Set(prev);
      if (next.has(key)) {
        next.delete(key);
      } else {
        next.add(key);
      }
      return next;
    });
  }, []);

  if (sortedRoots.length === 0) {
    return (
      <div className="empty-state" style={{ margin: "1rem" }}>
        <strong>No fork relationships found</strong>
        <span>All sessions are independent (no shared user inputs).</span>
      </div>
    );
  }

  return (
    <div className="tree-view tree-view--virtual" ref={scrollRef}>
      <div style={{ height: `${virtualizer.getTotalSize()}px`, position: "relative" }}>
        {virtualizer.getVirtualItems().map((virtualRow) => {
          const item = flatItems[virtualRow.index];
          return (
            <div
              key={item.node.sessionKey}
              data-index={virtualRow.index}
              ref={virtualizer.measureElement}
              style={{
                position: "absolute",
                top: 0,
                left: 0,
                width: "100%",
                transform: `translateY(${virtualRow.start}px)`,
              }}
            >
              <TreeRow
                item={item}
                sessionMap={sessionMap}
                selectedKey={selectedKey}
                search={search}
                expandedKeys={expandedKeys}
                onSelect={onSelect}
                onForkJump={onForkJump}
                onToggleExpand={toggleExpand}
              />
            </div>
          );
        })}
      </div>
    </div>
  );
});

// ─── TreeRow (single virtualized row) ───────────────────────────────────────

interface TreeRowProps {
  item: FlatTreeItem;
  sessionMap: Map<string, SessionMeta>;
  selectedKey: string | null;
  search: string;
  expandedKeys: Set<string>;
  onSelect: (session: SessionMeta) => void;
  onForkJump?: (session: SessionMeta, forkAtUser: number) => void;
  onToggleExpand: (key: string) => void;
}

const TreeRow = memo(function TreeRow({
  item,
  sessionMap,
  selectedKey,
  search,
  expandedKeys,
  onSelect,
  onForkJump,
  onToggleExpand,
}: TreeRowProps) {
  const { node, depth, inGroup, groupFirst, groupLast } = item;
  const hasChildren = node.children.length > 0;
  const expanded = expandedKeys.has(node.sessionKey);
  const session = sessionMap.get(node.sessionKey);

  const groupClass = inGroup
    ? ` tree-group${groupFirst ? " tree-group-first" : ""}${groupLast ? " tree-group-last" : ""}`
    : "";

  return (
    <div
      className={`tree-node${hasChildren ? " has-children" : ""}${depth === 0 ? " tree-root" : ""}${groupClass}`}
      style={{ "--indent": `${depth * 24}px` } as React.CSSProperties}
    >
      <div className="tree-node-row">
        {/* Expand/collapse toggle */}
        {hasChildren ? (
          <button
            type="button"
            className="tree-toggle-btn"
            onClick={() => onToggleExpand(node.sessionKey)}
            aria-label={expanded ? "Collapse" : "Expand"}
            aria-expanded={expanded}
          >
            <svg className="tree-toggle-icon" viewBox="0 0 16 16" aria-hidden="true">
              <path d="M6 4.5 9.5 8 6 11.5" />
            </svg>
          </button>
        ) : (
          <span className="tree-toggle-btn tree-toggle-spacer" />
        )}

        {/* Session item */}
        {session ? (
          <div className="tree-session-item-wrapper">
            <SessionItem
              session={session}
              selected={selectedKey === node.sessionKey}
              search={search}
              onSelect={() => onSelect(session)}
            />
            {/* Fork context: show the first diverging user input, click to jump */}
            {depth > 0 && node.forkUserText && (
              <div
                className="tree-fork-text"
                title={`Click to jump to user #${node.forkedAtUser}: ${node.forkUserText}`}
                onClick={() => {
                  if (session && onForkJump) {
                    onForkJump(session, node.forkedAtUser);
                  }
                }}
                role="button"
                tabIndex={0}
                onKeyDown={(e) => {
                  if ((e.key === "Enter" || e.key === " ") && session && onForkJump) {
                    e.preventDefault();
                    onForkJump(session, node.forkedAtUser);
                  }
                }}
              >
                {node.forkUserText}
              </div>
            )}
          </div>
        ) : (
          // Session not in current scope — render minimal info from tree data
          <div className="session-item tree-session-item-fallback">
            <div className="session-item-header">
              <span className="session-title">{node.title}</span>
            </div>
            <div className="session-meta-row">
              <span className="session-time">Not in current scope</span>
            </div>
          </div>
        )}
      </div>
    </div>
  );
});
