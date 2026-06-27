# DevWhisp — Embedded Voice-to-Text Pill Application

> A small, tray-resident, always-on-top "pill" desktop app that turns your voice into text and pastes it into whatever app has your cursor focused. BridgeVoice-style design, fully offline, runs on CPU.

**Status:** Draft v3 (brand kit + family design added)
**Date:** 2026-06-26
**Workspace:** `D:\projects\DevWhisp`

---

## 1. Project Definition

### 1.1 Goal

Build a cross-platform desktop **"pill"** application — small footprint, tray-resident, hotkey-driven, BridgeVoice-style — that:

1. Lives as a small **floating, always-on-top pill** on the user's desktop (visible-but-discreet)
2. Captures microphone audio on demand (push-to-talk or toggle hotkey)
3. Runs an **embedded speech-to-text model locally** (no cloud, no API key)
4. Runs **entirely on CPU** — works on machines with no dedicated GPU
5. Transcribes **fast** (sub-second first token on short utterances), **performant** (low idle RAM), and **accurate** (WER ≤ 5% on clean English)
6. **Pastes** the transcribed text into whatever app currently has the cursor focused
7. Keeps a **local transcription history** accessible from the system tray, with one-click copy
8. Looks like a polished product, not a tech demo

### 1.2 Scope

**In scope (v1):**
- Windows 10/11 first; macOS & Linux follow-up
- English (Tiny + Base models)
- Global hotkey capture, push-to-talk mode, toggle mode
- **Floating always-on-top pill** widget (3 states: idle / listening / processing)
- **System tray** icon + menu
- **Transcription history window** (browse, search, copy, delete)
- **Settings panel** (model picker, hotkey, language, theme, audio device)
- Local-first, offline by default
- Audio capture via OS APIs (WASAPI on Windows, CoreAudio on macOS, PulseAudio/PipeWire on Linux)
- **Cursor-paste text injection** (save focus → clipboard.set(text) → keystroke Ctrl+V / Cmd+V)

**Out of scope (v1):**
- Cloud fallback / online transcription
- Speaker diarization (multi-speaker separation)
- Custom fine-tuned models
- Mobile (iOS/Android)

### 1.3 Success Criteria

| Metric | Target |
|---|---|
| Installer size | ≤ 25 MB (Tauri build, model not bundled — downloaded on first run) |
| Idle RAM | ≤ 80 MB |
| Cold start to tray | ≤ 800 ms |
| First-token latency (5 s utterance, Moonshine Tiny) | ≤ 300 ms |
| Real-time factor (RTF) on CPU (Moonshine Tiny) | ≥ 5× |
| WER on LibriSpeech clean | ≤ 5% (Moonshine Tiny: 2.71%) |
| Languages | EN v1; ≥ 8 languages v2 |

---

## 2. User Experience & UI Design

### 2.1 Brand Kit — The App Family

DevWhisp is the third in a family of developer desktop apps alongside **DevTerm** and **DevSpace**. The visual identity follows a shared grammar so the three apps feel like siblings.

**Visual mockup:** [`mockup_app_family.png`](../mockup_app_family.png)

#### Design DNA (shared across all three apps)

| Property | Value | Rationale |
|---|---|---|
| Canvas | 1024 × 1024 viewBox | Standard app icon, retina-ready |
| Container | Rounded square, 196–208 px corner radius | Soft, modern, iOS/macOS-friendly |
| Padding | 64–96 px from edges | Generous breathing room |
| Accent strategy | White stroke + 1 solid fill element | Single bold glyph, no clutter |
| Stroke weight | 52–104 px | Bold, readable at 16 px favicon size |
| Naming | `Dev` + noun | Consistent with existing family |

#### Color palette per app

| App | Top gradient | Bottom gradient | Accent | Vibe |
|---|---|---|---|---|
| **DevTerm** | `#2a3350` | `#12131b` | `#9bbcff → #6a8dff` (cool blue) | Deep, terminal, command-line |
| **DevSpace** | `#7cc1ff` | `#4a90d9` | `#ffffff` (white) | Open, sky, AI workspace |
| **DevWhisp** | `#c4b5fd` | `#7c3aed` | `#ffffff` (white) | Soft, airy, violet — "whisper" |

The three apps form a cool-spectrum gradient: navy → sky blue → violet. Each is a different position on the cool-temperature axis (deep → mid → soft) and a different stop on the lightness scale.

#### DevWhisp icon — FINAL: "Voice Waves" ✅

The "whisper" essence had to be encoded without falling back on a generic microphone glyph (too on-the-nose), **and** it had to read as a sibling of DevTerm (`>_`) and DevSpace (`<✦>`) — crisp white geometric glyphs, not organic art.

**Final glyph: Voice Waves** — a solid white **source dot** with **three concentric arcs** radiating right (the universal "sound/voice" mark), on the violet gradient tile. The arcs are true circular arcs (geometric, family-consistent with the sibling chevrons) and their opacity tapers (1.0 → 0.80 → 0.60) so the voice "whispers" outward — which also echoes the pill's three states (idle → listening → processing). This replaces the earlier organic three-wisp concept, which read as a feather/broom and broke the family's geometric language.

The glyph follows the family DNA exactly: **white strokes + one solid fill element** (the dot), round caps/joins, generous padding, legible down to 16 px.

**Canonical vector source:** [`../design/devwhisp-icon.svg`](../design/devwhisp-icon.svg).
**Raster pipeline (single source of truth):** [`../design/render_voicewaves_icon.py`](../design/render_voicewaves_icon.py) — renders every shipped size (16/20/24/32/40/48/64/128/256/1024 px), the multi-frame `.ico`, the `.icns`, and the Windows Square logos, all from one geometry definition (heavy supersampling + LANCZOS). This supersedes the two earlier, conflicting pipelines (`render_clean_icon.py` and `regen_icons.py`, which fought over a hand-rendered wisp vs. an AI raster). The in-app brand components (`src/lib/AppIcon.svelte`, `BrandMark.svelte`) carry the same geometry.

**Concept archive** (kept for reference, do not ship):
- [`../design/devwhisp-icon-A-wisp.svg`](../design/devwhisp-icon-A-wisp.svg) — earlier wisp concept (superseded)
- [`../design/devwhisp-icon-B-monogram.svg`](../design/devwhisp-icon-B-monogram.svg)
- [`../design/devwhisp-icon-C-breath.svg`](../design/devwhisp-icon-C-breath.svg)
- [`../design/devwhisp-icon-D-soundwisp.svg`](../design/devwhisp-icon-D-soundwisp.svg)

#### Pill UI accent (pill in-app, not the app icon)

The pill widget uses a slightly different palette than the app icon — it's softer and more contextual (sits on top of other apps all day, needs to be unobtrusive):

- **Idle state:** soft violet `#7C5CFF` at 30% alpha
- **Listening state:** red→pink gradient `#FF5C7C → #FF9C5C` (warm, attention-grabbing)
- **Processing state:** neutral gray `#8A8FA3` (calm, working)
- **Success state:** mint green `#5CFF9C` (positive, ephemeral)
- **Error state:** amber `#FFB45C` (warning, not aggressive)

The accent on the pill waveform bars and icon matches the state. The app-icon violet is the brand color; the pill-state colors are functional indicators.

#### Typography

- **UI font:** Inter (matches DevTerm/DevSpace system)
- **Transcript text (pill):** JetBrains Mono (developer-friendly, matches terminal aesthetic)
- **Transcript text (history):** Inter (more readable for long-form)

#### Filename + identifier convention

- App name: `DevWhisp`
- Binary name: `devwhisp` (lowercase, no spaces, matches `devterm` and `devspace`)
- Tray tooltip: `DevWhisp`
- User-agent (auto-update): `devwhisp/<version> (tauri)`
- npm scope: not applicable (Rust + Svelte, no npm scope)
- Cargo crate: `devwhisp`

### 2.2 The Pill Widget (always-on-top, floating)

The pill is the heart of the UX. It mirrors BridgeVoice's design pattern with three visual states.

**Visual mockups:** [`mockup_pill_idle.png`](mockup_pill_idle.png) · [`mockup_pill_listening.png`](mockup_pill_listening.png)

#### State 1 — Idle

Small, collapsed pill, ~120×40 px. Sits in a chosen screen corner or follows cursor. Glassmorphism, low opacity (~80%), app mark icon dimmed (not a microphone — uses the chosen DevWhisp icon concept, ~16px), subtle pulse every 4 s to signal "ready."

```
  ┌──────────────────────┐
  │  ◉  DevWhisp · idle  │   ← faint "ready" text on hover only
  └──────────────────────┘
```

ASCII detail:
```
╭─────────────────────────╮
│  🎙  ⌃  DevWhisp        │  ← 36px tall, 160px wide
│       (idle, 30% alpha) │
╰─────────────────────────╯
```

**Interaction:**
- Hover: opacity → 100%, label "Hold ⌃Space to talk" appears
- Double-click: toggle recording (if mode = Toggle)
- Drag: reposition anywhere on screen (snaps to screen edge on release)
- Right-click: context menu (Start/Stop, Settings, History, Quit)

#### State 2 — Listening

Pill expands horizontally to ~280×56 px. Microphone icon turns solid accent color. Live audio visualization: 7 vertical frequency bars (16 Hz bands from a 1024-point FFT, ~30 fps). Background gradient pulses subtly (red→pink during recording, intensity tracks RMS).

```
╭───────────────────────────────────────╮
│  🎙 ▁▃▆█▅▃▁  │  "hello can you hear"  │  ← live partial transcript (truncated)
╰───────────────────────────────────────╯
        ↑                                ↑
   7 freq bars                  partial text appears as you speak
```

**Behavior:**
- Pops up in 200 ms ease-out spring animation from idle position
- Audio bars update at 30 fps from system audio capture
- Partial text (Moonshine streaming output) appears in the right side, max 1 line, fades old words out
- **No window border, no title bar** — pure frameless transparent WebView
- Stays above all apps (`alwaysOnTop = true`)

#### State 3 — Processing

Pill keeps listening-state size but the audio bars morph into a thin loading spinner (rotating ring) and color shifts to neutral gray. Shows "Transcribing…" for ~100–500 ms.

```
╭───────────────────────────────────────╮
│  ◌  Transcribing…  · 0.2s              │
╰───────────────────────────────────────╯
```

#### State 4 — Success (auto-dismiss, optional)

Pill briefly shows a green checkmark + "Pasted ✓" for 600 ms, then collapses back to idle.

```
╭───────────────────╮
│  ✓  Pasted        │  ← 600ms, then back to idle
╰───────────────────╯
```

#### State 5 — Error

Pill turns amber/red, shows error icon + short message for 3 s, then back to idle.

```
╭─────────────────────────────────────╮
│  ⚠  No microphone detected          │
╰─────────────────────────────────────╯
```

#### Pill Behaviors

- **Default position:** Bottom-center, 24 px from screen edge (configurable)
- **Snap zones:** When dragged within 80 px of a screen edge, snap and stick
- **Auto-hide:** Optional "hide after 5 s of idle" mode (off by default)
- **Always-on-top:** Yes, on top of fullscreen apps too (use `setAlwaysOnTop(true, "screen-saver")` on macOS, `HWND_TOPMOST` on Windows)
- **Multi-monitor:** Latches to primary monitor by default
- **Click-through:** When collapsed-idle, can be set to click-through (lets clicks pass through to apps behind)

### 2.3 The Tray Icon + Menu

The system tray is the always-present control point. Right-click for the menu, left-click (or click on the main item) to open the History window.

**Tray icon design:** A circular microphone badge, dark background, accent-color mic. Animates (pulses) while recording.

**Tray menu (right-click):**

```
╭───────────────────────────────────╮
│  ◉ Start Recording                │  ← or "Stop Recording" if active
│  ─────────────────────────────    │
│  ⌃ Space  ·  Push-to-Talk         │  ← current hotkey, dynamic
│  🎙 Base  ·  English              │  ← current model, dynamic
│  ─────────────────────────────    │
│  📜 Open History                  │
│  ⚙ Settings…                      │
│  ❓ Help                          │
│  ─────────────────────────────    │
│  ↗ Launch on startup       ☑       │
│  ⏻ Quit DevWhisp                  │
╰───────────────────────────────────╯
```

### 2.4 The History Window (transcription history browser)

**Visual mockup:** [`mockup_history.png`](mockup_history.png)

A full window, ~720×640, opened from tray. Shows past transcriptions with full metadata, search, copy, delete, and export.

**Layout:**

```
╭──────────────────────────────────────────────────────────────────────╮
│  ◉  DevWhisp                                          ⌄ Collapse  ✕  │
├──────────────────────────────────────────────────────────────────────┤
│  ⌕ Search transcriptions…                                  248 total  │
│  [ All ]  [ Today ]  [ This week ]  [ Local ]  [ Cloud ]             │
├──────────────────────────────────────────────────────────────────────┤
│  ┌──────────────────────────────────────────────────────────────┐    │
│  │ 14:32  ·  0:04  ·  8 words  ·  🖥 Local                       │ ⧉ │
│  │ "Can you review the latest PR before standup?"                │    │
│  └──────────────────────────────────────────────────────────────┘    │
│  ┌──────────────────────────────────────────────────────────────┐    │
│  │ 14:18  ·  0:12  ·  24 words  ·  🖥 Local                      │ ⧉ │
│  │ "Let's ship the new dashboard today — I'll handle the…"      │    │
│  └──────────────────────────────────────────────────────────────┘    │
│  ┌──────────────────────────────────────────────────────────────┐    │
│  │ 13:55  ·  0:03  ·  5 words  ·  🖥 Local                       │ ⧉ │
│  │ "Looks good to me"                                            │    │
│  └──────────────────────────────────────────────────────────────┘    │
│  …                                                                   │
├──────────────────────────────────────────────────────────────────────┤
│  Today: 47 transcriptions · 1,243 words · avg 26 wpm     ⚙ Settings  │
╰──────────────────────────────────────────────────────────────────────╯
```

**Each row:**
- Timestamp (HH:MM)
- Duration (M:SS)
- Word count
- Source badge (Local / Cloud in v2)
- Transcribed text (1–2 lines, ellipsized)
- Copy button (⧉) — copies to clipboard with success toast
- Hover: row background lightens, shows delete (🗑) and "View full" options

**Header actions:**
- Search bar (filter by text content)
- Filter chips (All, Today, This week, Local, Cloud)
- Total count

**Footer stats bar:**
- Today's stats: transcriptions, words, WPM
- Settings shortcut

**Click row:** expands to show full text + Edit / Re-inject buttons
**Right-click row:** Copy / Re-inject (paste again) / Delete / Add to dictionary

### 2.5 The Settings Panel

**Visual mockup:** [`mockup_settings.png`](mockup_settings.png)

A modal window, ~640×720. Sidebar nav on the left, content on the right.

```
╭─────────────────────────────────────────────────────────────╮
│  ⚙ DevWhisp Settings                                  ✕    │
├──────────────┬──────────────────────────────────────────────┤
│  ◉ General   │  ─── Recording ───                           │
│  🎙 Models   │  Hotkey        [ ⌃  Space         ]  ▾       │
│  🎨 Theme    │  Mode          ( ) Push-to-Talk               │
│  🔤 Language │                (●) Toggle                     │
│  🎤 Audio    │                ( ) Hands-free wake word  v2   │
│  ⌨ Shortcuts │  VAD silence    [▮▮▮▮▯▯▯▯▯]  500ms          │
│  📖 History  │                                               │
│  🔔 Notif.   │  ─── Model ───                               │
│  ⬆ Updates   │  (●) Moonshine Tiny   50 MB   ⬇ downloaded    │
│  ℹ About     │  ( ) Moonshine Base  100 MB                  │
│              │  ( ) Whisper small    466 MB                 │
│              │  ( ) Vosk small       50 MB / per-language   │
│              │                                               │
│              │  ─── Text Injection ───                      │
│              │  (●) Clipboard + paste (works everywhere)     │
│              │  ( ) Direct keystroke (faster, less compat.) │
│              │  [ ] Append space after paste                 │
│              │  [ ] Capitalize first letter                 │
│              │                                               │
│              │  ─── Custom Dictionary ───                   │
│              │  spoken phrase         →   replacement        │
│              │  "next js"                  Next.js            │
│              │  "typescript"               TypeScript         │
│              │  "tauri"                    Tauri             │
│              │  + add entry                                    │
└──────────────┴──────────────────────────────────────────────┘
```

**Sections:**
1. **General** — launch on startup, minimize to tray, language
2. **Models** — picker with size/accuracy/wer metadata, download manager
3. **Theme** — Dark / Light / System, accent color picker
4. **Language** — UI language + STT language
5. **Audio** — input device, noise suppression, gain
6. **Shortcuts** — all hotkeys (record, open history, open settings, etc.)
7. **History** — clear all, export, retention period
8. **Notifications** — toast on completion, sound, badge
9. **Updates** — auto-update channel (stable/beta)
10. **About** — version, links, check for updates

### 2.6 First-Run / Onboarding

Linear 4-step wizard on first launch:

1. **Welcome** — "Talk instead of type." Quick value prop + screenshot of pill
2. **Permissions** — Microphone permission request (OS-level), then accessibility (for paste injection)
3. **Pick a model** — Moonshine Tiny (recommended, 50 MB) or Base (100 MB, more accurate)
4. **Set your hotkey** — Press the keys you want, defaults to `Ctrl+Shift+Space`
5. **Try it** — Press hotkey, say "hello world", see it appear in the focused text field. **Test pass criterion.**

### 2.7 Text Injection Flow (the cursor-paste bit)

This is the core "paste where my cursor is" feature. Must work universally across editors, terminals, browsers, chat apps, etc.

**Algorithm:**

1. **Before recording starts:** Save the currently focused window's HWND/NSWindow handle + a snapshot of clipboard content (so we can restore it)
2. **On transcription complete:**
   - `clipboard.set(formatted_text)` via `arboard` / OS API
   - Wait 30 ms (let clipboard propagate)
   - Send synthetic `Ctrl+V` (Windows/Linux) or `Cmd+V` (macOS) via `enigo`
3. **Optional formatting (user-configurable):**
   - Auto-capitalize first letter
   - Append space (if user toggle)
   - Apply dictionary replacements (`"next js"` → `Next.js`)
   - Strip leading/trailing whitespace
4. **Edge cases:**
   - **No focused text field** → fall back to: show notification "Nothing to paste into — copied to clipboard" + keep text on clipboard
   - **Terminal with bracketed paste mode** → wrap text in `\x1b[200~...\x1b[201~`
   - **Browser address bar / URL field** → still works, but skip auto-capitalize
   - **Rich text editor** (Notion, Google Docs) → clipboard paste preserves plain text fallback
5. **Clipboard restoration:** After 5 s, restore original clipboard content (so the user's previous copy isn't lost)

**Why this matters:** The user explicitly asked for "paste the whatever I say into the place I have my cursor selecting." This is the exact behavior of BridgeVoice and Wispr Flow. It's table stakes.

---

## 3. Architecture

### 3.1 Recommended: Tauri 2 + Rust + Moonshine (via ONNX Runtime)

```
┌─────────────────────────────────────────────────────────┐
│  Pill Widget (WebView, transparent, always-on-top)      │
│  • Svelte 5 UI · live partial transcript · 3 states     │
│  • 7-band audio visualizer (requestAnimationFrame)      │
└─────────────────────────────────────────────────────────┘
                ▲ Tauri IPC │ (commands / events)
┌─────────────────────────────────────────────────────────┐
│  Rust Core (Tauri host)                                 │
│  • cpal audio capture (WASAPI/CoreAudio/PulseAudio)     │
│  • silero-vad (VAD, 30ms frames)                        │
│  • ring buffer → 16kHz mono PCM                        │
│  • Moonshine runner (ort crate / ONNX Runtime, INT8)   │
│  • streaming partial + final transcript                 │
│  • enigo + arboard (cursor-paste injection)             │
│  • tauri-plugin-global-shortcut                         │
│  • tauri-plugin-system-tray                             │
│  • rusqlite (transcription history DB)                  │
│  • tauri-plugin-store (settings)                        │
│  • single-instance + autostart                          │
└─────────────────────────────────────────────────────────┘
                ▲ tray menu click │ opens History window
┌─────────────────────────────────────────────────────────┐
│  History Window (WebView, normal window)                │
│  • search + filter chips + list of past transcriptions  │
│  • one-click copy, re-inject, delete, add to dictionary │
│  • stats footer (words, WPM, sessions)                  │
└─────────────────────────────────────────────────────────┘
```

### 3.2 Why this stack

**Tauri 2** for the shell:
- Bundle: ~5–15 MB shell + 50 MB model (downloaded on first run, not bundled)
- Idle RAM: 30–60 MB
- Cold start: 200–400 ms
- Native Rust → direct FFI to ONNX Runtime, no Node.js boundary
- Frameless transparent window = perfect for the floating pill
- Cross-platform (Win/macOS/Linux) from one codebase

**Moonshine** for STT:
- 5× faster than Whisper on short audio
- 50 MB INT8 Tiny, WER 2.71% on LibriSpeech clean (beats Whisper tiny.en at 5%)
- Sub-200ms first-token latency
- Apache-2.0, ONNX exports
- v2 multilingual via Moonshine Medium (245 M, 8 languages)

**Svelte 5** for the UI:
- Smaller bundle than React (matters for a "pill" app)
- Built-in transitions for the state animations
- Excellent DX for tiny components

### 3.3 Tech stack

#### Languages & frameworks
- **Rust 1.80+** — backend, audio, STT
- **TypeScript 5.x strict** — frontend
- **Svelte 5** (preferred) or React 18
- **Tauri 2.x** — desktop shell

#### Core STT & audio
- **`ort` crate** v2.x — ONNX Runtime bindings for Rust, CPU provider
- **Moonshine ONNX exports** — tiny / base / medium variants from `moonshine-ai/moonshine`
- **`whisper-rs` crate** — optional fallback (binds whisper.cpp)
- **`vosk-rs` crate** — low-end fallback for v2
- **`cpal` crate** — cross-platform audio capture
- **`rubato` crate** — resample 48 kHz device → 16 kHz model input
- **`hound` crate** — WAV I/O (debug)
- **`silero-vad` crate** or small Rust port — VAD

#### Desktop integration
- **`tauri-plugin-global-shortcut`** — push-to-talk hotkey
- **`tauri-plugin-system-tray`** — tray icon + menu
- **`tauri-plugin-autostart`** — launch on login
- **`tauri-plugin-single-instance`** — prevent duplicates
- **`tauri-plugin-updater`** — auto-update
- **`tauri-plugin-clipboard-manager`** — clipboard
- **`tauri-plugin-store`** — settings KV
- **`enigo` crate** — cross-platform keystroke sim
- **`arboard` crate** — clipboard r/w
- **`rusqlite` crate** — transcription history DB

#### Build / release
- **`cargo-bundle` + Tauri bundler** — MSI/NSIS/DMG/DEB/AppImage
- **GitHub Actions + `tauri-action`** — CI/CD matrix

### 3.4 Project Layout

```
devwhisp/
├── src-tauri/                       # Rust backend
│   ├── src/
│   │   ├── main.rs                  # Entry point
│   │   ├── audio/
│   │   │   ├── mod.rs
│   │   │   ├── capture.rs           # cpal-based mic capture
│   │   │   ├── resampler.rs         # 48k → 16k via rubato
│   │   │   ├── vad.rs               # silero VAD
│   │   │   └── visualizer.rs        # FFT for pill bars
│   │   ├── stt/
│   │   │   ├── mod.rs
│   │   │   ├── moonshine.rs         # Moonshine runner (ONNX)
│   │   │   ├── whisper_fallback.rs  # whisper-rs runner
│   │   │   └── model_manager.rs     # download / cache / swap
│   │   ├── inject/
│   │   │   ├── mod.rs
│   │   │   ├── clipboard.rs         # save/restore + set
│   │   │   ├── keystroke.rs         # enigo paste
│   │   │   └── formatter.rs         # capitalize, dict replace
│   │   ├── hotkey.rs                # global hotkey handler
│   │   ├── tray.rs                  # system tray + menu
│   │   ├── history.rs               # rusqlite transcription DB
│   │   ├── config.rs                # settings
│   │   ├── window/
│   │   │   ├── pill.rs              # frameless always-on-top
│   │   │   ├── history_window.rs    # main history browser
│   │   │   └── settings_window.rs   # settings modal
│   │   ├── onboarding.rs            # first-run wizard
│   │   └── ipc.rs                   # Tauri commands
│   ├── Cargo.toml
│   └── tauri.conf.json
├── src/                             # Frontend (Svelte)
│   ├── main.ts
│   ├── App.svelte
│   ├── lib/
│   │   ├── stores/
│   │   │   ├── recording.ts         # state machine
│   │   │   ├── history.ts
│   │   │   └── settings.ts
│   │   ├── components/
│   │   │   ├── Pill.svelte          # floating widget (3 states)
│   │   │   ├── AudioVisualizer.svelte  # 7-band bars
│   │   │   ├── HistoryList.svelte
│   │   │   ├── HistoryRow.svelte
│   │   │   ├── SettingsPanel.svelte
│   │   │   ├── ModelPicker.svelte
│   │   │   ├── HotkeyCapture.svelte
│   │   │   └── OnboardingWizard.svelte
│   │   └── ipc.ts                   # Tauri command bindings
│   ├── styles/
│   │   ├── tokens.css               # design tokens
│   │   └── global.css
│   └── app.html
├── public/
│   └── icons/
├── package.json
├── vite.config.ts
├── svelte.config.js
├── tsconfig.json
└── README.md
```

### 3.5 Data flow (push-to-talk)

```
[User presses Ctrl+Shift+Space]
        ↓
[global-shortcut → hotkey.rs → emit "recording-start" to pill window]
        ↓
[Pill animates: idle → listening state]
        ↓
[cpal opens stream at device rate (typically 48 kHz)]
        ↓
[Audio thread → ring buffer (lock-free) → resampler → 16 kHz mono PCM]
        ↓
[Two consumers in parallel:]
  ├→ [VAD frames → if speech: mark; if silence > 500ms: emit "utterance-end"]
  └→ [Visualizer FFT (30 fps) → emit "audio-level" for pill bars]
        ↓
[VAD says: utterance ended → Moonshine final decode]
        ↓
[Partial transcripts emitted via "partial-transcript" event during decode]
        ↓
[On final decode: format text → save to history DB → emit "final-transcript"]
        ↓
[Pill animates: processing → success → idle]
        ↓
[inject.rs: clipboard.set(formatted) → enigo simulate Ctrl+V]
        ↓
[Cursor-paste complete; original clipboard restored after 5s]
```

### 3.6 Performance Budget

| Operation | Budget |
|---|---|
| Cold start → tray | 400 ms |
| Hotkey press → pill listening | 30 ms |
| Audio buffer latency (capture → STT) | 80 ms |
| VAD frame inference (30 ms) | 5 ms |
| Moonshine Tiny encoder (5 s) | 80 ms |
| Moonshine Tiny first token | 30 ms |
| End-to-end (5 s utterance → paste) | ~250 ms |
| Text injection (clipboard + paste) | 40 ms |
| Idle RAM | 60 MB |
| Peak RAM during inference | 350 MB |
| CPU idle | < 0.5% |
| CPU inference (1 core of 4) | 60% |

---

## 4. Task Decomposition (5 phases, 11 weeks)

### Phase 1 — Feasibility spike (1 week)

| ID | Task | Effort |
|---|---|---|
| T1.1 | Stand up Tauri 2 + Svelte scaffold | 0.5 d |
| T1.2 | Add `ort` crate, verify Moonshine Tiny ONNX loads on Windows CPU | 1 d |
| T1.3 | Minimal audio capture (cpal) + resampler | 1 d |
| T1.4 | End-to-end spike: capture 5s WAV → Moonshine → print text | 1 d |
| T1.5 | **First cursor-paste test** — speak, see text appear in focused app | 0.5 d |
| T1.6 | Measure cold start, latency, RTF, RAM | 0.5 d |

**Exit criterion:** Speak into mic, text appears in any focused app within 1 second. BridgeVoice parity proof.

### Phase 2 — Core engine (3 weeks)

| ID | Task | Effort |
|---|---|---|
| T2.1 | Ring-buffer + lock-free audio pipeline | 2 d |
| T2.2 | silero-vad integration | 2 d |
| T2.3 | Streaming Moonshine runner with partial outputs | 4 d |
| T2.4 | Utterance segmentation (silence detection + final decode) | 2 d |
| T2.5 | Global hotkey wiring (Ctrl+Shift+Space) | 1 d |
| T2.6 | **Cursor-paste text injection** (clipboard save/set/paste + restore) | 2 d |
| T2.7 | **Text formatter** (capitalize, dict replace, append space) | 1 d |
| T2.8 | **Custom dictionary** (CRUD for replacements) | 1 d |
| T2.9 | System tray icon + menu (Start, Pause, History, Settings, Quit) | 2 d |
| T2.10 | History DB schema + insert/query (rusqlite) | 1 d |

**Exit criterion:** Hotkey → speak → text appears in any focused app on 3 Windows machines. Transcriptions saved to history. Dictionary works.

### Phase 3 — UI/UX (2 weeks)

| ID | Task | Effort |
|---|---|---|
| T3.1 | **Pill widget** — frameless, transparent, always-on-top WebView | 3 d |
| T3.2 | **Pill state machine** (idle / listening / processing / success / error) | 2 d |
| T3.3 | **7-band audio visualizer** (FFT → SVG bars) | 2 d |
| T3.4 | **Live partial transcript** in pill (streaming text, fade) | 1 d |
| T3.5 | **Pill drag + snap-to-edge** behavior | 1 d |
| T3.6 | **History window** — list, search, filter chips, copy/re-inject/delete | 3 d |
| T3.7 | **Settings panel** — full sidebar nav + all sections (10 sections) | 3 d |
| T3.8 | **Onboarding wizard** (4 steps, first-run only) | 2 d |
| T3.9 | **Stats footer** in history (WPM, total words, session count) | 0.5 d |

**Exit criterion:** A user with no docs can install → complete onboarding → record first transcription → find it in history → copy it. < 2 min from launch to first paste.

### Phase 4 — Hardening & release (2 weeks)

| ID | Task | Effort |
|---|---|---|
| T4.1 | Benchmark suite (RTF, latency, RAM across 5 machines) | 2 d |
| T4.2 | Tauri bundler config (MSI/NSIS for Windows, signed) | 2 d |
| T4.3 | Auto-update (signed manifests via tauri-plugin-updater) | 2 d |
| T4.4 | Code signing setup (Azure Trusted Signing) | 1 d |
| T4.5 | CI/CD (GitHub Actions: typecheck + lint + cargo test + smoke) | 1 d |
| T4.6 | Single-instance enforcement | 0.5 d |
| T4.7 | Autostart-on-login | 0.5 d |
| T4.8 | Crash reporting (sentry-rs or in-house) | 1 d |
| T4.9 | Theme support (Dark / Light / System) | 1 d |
| T4.10 | Accessibility (screen reader, keyboard nav, high contrast) | 1 d |
| T4.11 | First-run UX polish (permissions, model download wizard) | 2 d |

**Exit criterion:** Signed installer downloads, installs, runs, auto-updates cleanly. CI is green. v0.1.0 release.

### Phase 5 — Multilingual & polish (3 weeks)

| ID | Task | Effort |
|---|---|---|
| T5.1 | Moonshine Medium (multilingual) integration | 2 d |
| T5.2 | Language detection (auto, with manual override) | 2 d |
| T5.3 | Post-processing: punctuation restoration (small BERT or rules) | 3 d |
| T5.4 | Filler-word removal (um, uh, like) | 1 d |
| T5.5 | macOS port (CoreAudio, accessibility perms, code signing) | 3 d |
| T5.6 | Linux port (PulseAudio/PipeWire, AppImage) | 2 d |
| T5.7 | Whisper.cpp fallback integration (user opt-in) | 2 d |
| T5.8 | Optional: Vosk for very-low-end machines | 1 d |
| T5.9 | **Export history** to JSON / CSV / Markdown | 1 d |
| T5.10 | **Wake word detection** (v2, "Hey DevWhisp") | 3 d |

**Exit criterion:** v1.0.0 with multilingual, cross-platform, polished UX.

---

## 5. Dependencies & Critical Path

```
T1.1 → T1.2 ─┐
     → T1.3 ─┴─→ T1.4 → T1.5 (FIRST PATCH!) → T1.6
                          ↓
              T2.1 → T2.2 → T2.4 → T2.6 (cursor paste) → T2.7 → T2.9
                    → T2.3 ──┘                        → T2.8 → T2.10 (history)
              T1.1 → T2.5 (hotkey)
                          ↓
              T3.1 → T3.2 → T3.3 → T3.4 → T3.5   ← PILL
              T2.10 → T3.6 → T3.9                  ← HISTORY
              T2.8 → T3.7                           ← SETTINGS
              T1.1 → T3.8                           ← ONBOARDING
                          ↓
              T4.1 → T4.2 → T4.3 → release
```

**Critical path:** T1.1 → T1.3 → T1.4 → T1.5 → T2.1 → T2.6 → T3.1 → T3.2 → T3.6 → T4.1 → T4.2 → T4.3.

---

## 6. Risks & Mitigation

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Moonshine ONNX has bugs on certain CPU features | Med | Med | Keep whisper.cpp as model-swap fallback; test on 5+ machines |
| `ort` crate breaking changes | Med | Low | Pin known-good version |
| Tauri tray plugin quirks on Linux | High | Low | Linux deferred to v2; rely on system-tray-spec |
| Rust ramp-up time | Med | Med | **M1 spike catches this early** — pivot to Electron if blocks ship |
| Cursor-paste blocked by some apps (games, fullscreen UWP) | High | Low | Fall back to clipboard-only + notification; document limitations |
| macOS accessibility permission UX | High | Low | First-run wizard with clear rationale; Apple notarization |
| Model download UX on slow connections | Med | Med | Stream with resume, SHA-256 verify, progress UI, offline install option |
| Code-signing cost/complexity | Med | High | Azure Trusted Signing ($10/mo); budget $300/yr for certs |
| Custom dictionary edge cases (regex, ordering) | Low | Low | Apply longest-match-first, case-insensitive matching |

---

## 7. Architecture alternatives (considered)

| Option | Bundle | RAM | Cold start | Verdict |
|---|---|---|---|---|
| **A. Tauri + Moonshine** ⭐ | 5–15 MB | 30–60 MB | 200–400 ms | Best for "pill" aesthetic; recommended |
| B. Electron + whisper.cpp | 80–150 MB | 120–200 MB | 800–1200 ms | Fastest dev velocity; you have Electron experience |
| C. Native Rust DIY | 3–8 MB | 25–50 MB | 150–300 ms | Premature optimization; too much DIY |

If Rust ramp-up is a concern, swap to **B (Electron + whisper-rs)** in Phase 1 — the UI designs and the task plan transfer 1:1. The frontend (Svelte) and the integration logic (cursor-paste, history, dictionary) are stack-agnostic.

---

## 8. Open questions

1. **License:** MIT? Apache 2.0? Commercial?
2. **Distribution:** GitHub only, or Microsoft Store / Winget / Homebrew?
3. **Telemetry:** Any usage analytics, or strict local-only?
4. **Monetization:** Free + donations, freemium, paid?
5. **Branding:** Real name + icon before Phase 4 (DevWhisp is a working title)
6. **macOS support priority:** ship with Windows v1, or block on macOS parity?
7. **Target hardware floor:** How old a CPU? Pentium? Core 2 Duo? Modern only?
8. **Wake word:** v2 feature or v1? ("Hey DevWhisp" hands-free)
9. **Cloud fallback:** Local-only, or optional Groq/OpenAI cloud (BridgeVoice Pro style)?

---

## 9. Deliverables

1. **`DevWhisp-Plan.md`** — this human-readable plan
2. **`DevWhisp-Tasks.json`** — machine-readable task graph (45 tasks, 5 phases)
3. **Visual mockups** — pill widget (idle + listening), history window, settings panel
4. (Phase 2) Working prototype: voice → cursor-paste on Windows
5. (Phase 4) Signed v0.1.0 installer
6. (Phase 5) Cross-platform v1.0.0

---

*Last updated: 2026-06-26. Owner: Ahmed. Status: ready for review + UI design sign-off.*
