# ui/ — Reusable UI primitives (workunit 20260909-1258)

## Contract

New UI takes materials from here FIRST. A raw `<button>` + bespoke CSS start
needs justification in review.

- **Buttons**: use the base classes (`.secondary-button` / `.primary-button` /
  `.danger-button` / `.ghost-button` / `.link-button`, defined in
  `src/styles/ui.css`). They are self-sufficient — typography, height, radius
  are baked in; scope CSS should only tweak size/shape, never re-add basics.
- **Menus**: `Menu` + `MenuItem` — trigger render-prop, outside-click
  dismissal, menu a11y (role, aria-expanded, menuitemradio for choices).
- **Popovers**: `Popover` — anchored panel (`top: calc(100% + 0.4rem)`) with
  outside-click dismissal; wrap trigger + popover in one `position: relative`
  container. Panel skin is `.ui-panel`; do not hand-roll panel CSS.
- **Settings form primitives** (workunit 20260910-1131): `SettingRow`
  (label + description + control slot + inline error), `ToggleRow`
  (`role="switch"` bool renderer), `SourcesEditor` (sourceList CRUD; provider
  picker built on `Menu`, removal guarded by ConfirmDeleteDialog). Their
  `.setting-*` classes in `ui.css` are self-sufficient like the buttons.
- **Tokens only**: colors/sizes come from `variables.css` tokens. Hardcoded
  hex values in component CSS are a review-reject (the export.css fallback
  colors were exactly this failure mode).

## Consumers

- `sessions/ExportQaControls` (preset menu + calendar popover)
- `sessions/FolderFilter` (folder actions menu + sort menu)
- `settings/SettingsPage` (ToggleRow + SourcesEditor for its renderer
  registry; SourcesEditor also consumes `Menu`)

## Known follow-ups

- `sessions/QATocPopover` still hand-rolls its panel (different anchoring
  context inside the messages section) — migrate when its layout is touched.
- `SegmentedControl` / `CopyButton` / `StarButton` are generic atoms still
  living in `sessions/`; relocate here at next touch to avoid import churn
  in this change.
