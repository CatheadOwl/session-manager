---
description: Dev-surface updater stub — local endpoint fixture driving check / available / up-to-date UI states without a real release
no-toc: true
---

# updater-stub — dev-surface update fixture

Local updater endpoint for **development only**. It lets `tauri dev` exercise
the check → available → up-to-date UI states against a fixture, with no real
release and no cloud change. The release surface is untouched: `tauri build`
reads `tauri.conf.json` only and keeps the production endpoint (and release
builds hard-fail on http endpoints, so the overlay can never leak into a
release).

## Pieces

| File | Role |
|------|------|
| `latest.json` | The fake `latest.json` the updater `check()` fetches. `version: "99.0.0"` (above any real version) triggers `available`; edit it to `"0.0.1"` (below the current app version) to trigger the up-to-date path. |
| `artifact.bin` | Placeholder payload for the `platforms.windows-x86_64.url` entry. An install attempt downloads it and then FAILS signature validation against the release pubkey baked into `tauri.conf.json` — that failure is the designed install-error test state; nothing is ever executed. |
| `../tauri.dev.conf.json` | Dev-only config overlay: re-points `plugins.updater.endpoints` at this stub and sets `dangerousInsecureTransportProtocol` (http is only dev-tolerated otherwise, with a startup warning). |

This directory sits inside the tauri dev watch scope: **editing a fixture
rebuilds + relaunches the app (~10s), whose mount auto-check then reflects
the new version by itself** — the flip workflow is edit-and-wait, no manual
clicking. Vite additionally serves the fixtures via the `updater-stub`
middleware in `vite.config.ts` (configureServer never runs under
`vite build`).

## Usage

```bash
npm run tauri:dev      # tauri dev --config src-tauri/tauri.dev.conf.json
```

With `99.0.0` served the Settings › General row reports
`Update to v99.0.0 available` and the toolbar toast lights up (collapse the
folder rail to see it); with `0.0.1` it reports `You're up to date.`
Plain `npm run tauri dev` keeps the production endpoint — the stub only
exists behind the overlay.

## Why a config overlay and not the old conf-mutation script

An earlier fixture server (`publish/scripts/serve-update-fixture.mjs`)
temporarily rewrote the shared `tauri.conf.json` and restored it on exit —
a crash left `localhost` in the config that release builds share (and release
builds hard-fail on http endpoints). The overlay is merge-only and dev-bound
by construction: nothing release-side ever reads `tauri.dev.conf.json`.
