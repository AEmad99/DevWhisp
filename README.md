# DevWhisp

> Small, tray-resident, push-to-talk voice-to-text desktop app with embedded STT. Runs on CPU only.

A "pill" application — floating always-on-top, listens on a hotkey, transcribes locally with a bundled on-device model, and pastes the result wherever your cursor is focused. No cloud, no API keys, no internet required.

## Stack

- **Tauri 2** + **Rust** — desktop shell
- **Svelte 5** + **TypeScript** — UI
- **whisper-rs** (whisper-tiny.en) — on-device STT, ~77 MB, CPU-only, **bundled in the installer**. A [Moonshine](https://github.com/moonshine-ai/moonshine) ONNX path is feature-gated for a later release.
- **cpal** + **rubato** — audio capture + resampling to 16 kHz
- **enigo** + **arboard** — cursor-paste text injection
- **rusqlite** — local transcription history

## Status

**Working prototype (Phase 1 complete, Phase 2 core + Phase 3 pill largely done).**
End-to-end today: hold **Ctrl+Shift+Space** → speak → whisper-tiny transcribes on
CPU → text is pasted into the focused app, and saved to local history. The pill
widget, system tray (with recording-mode switch + History/Settings shortcuts),
custom dictionary, and formatter are all functional. STT runs via `whisper-rs`;
the Moonshine ONNX path is feature-gated for a later phase. See
[`plans/DevWhisp-Plan.md`](plans/DevWhisp-Plan.md) for the full phase status.

## Develop

Prereqs: Rust 1.80+, Node 22+, npm, Tauri CLI v2.

```bash
npm install
npm run tauri:dev      # launches the Tauri app
```

Build the installer:

```bash
npm run tauri:build    # → src-tauri/target/release/bundle/nsis/
```

The Windows NSIS installer is small and **model-free**. The speech model is
downloaded **inside the app** on first use (one-time, choose Tiny or larger).
WebView2 runtime is bundled for offline install. Fully offline after the
one-time model download. Installs per-user (no admin prompt).

## Features

- Push-to-talk **and** toggle recording modes (switch in Settings or the tray)
- Floating, draggable pill that snaps to screen edges, with live audio bars
- System tray: start/stop, mode switch, History/Settings shortcuts, Help
- Transcription history with search, time filters, stats, copy, and re-paste
- Custom dictionary (longest-match-first) + auto-capitalize / trailing-space
- Start-at-login, first-run onboarding, and a settings panel

## Project layout

```
.
├── src/                  # Svelte frontend
├── src-tauri/            # Rust backend
├── design/               # Canonical app icon SVGs
├── docs/mockups/         # Design mockups
├── plans/                # Project plan and task graph
└── public/               # Static assets
```

## License

TBD — see `plans/DevWhisp-Plan.md` for the open-questions section.
