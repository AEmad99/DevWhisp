# DevWhisp Review — Inference Optimization & UI Audit

> **Date:** 2026-07-02  
> **Scope:** CPU/GPU inference pipeline, frontend UI/UX, architecture  
> **Files reviewed:** `src-tauri/src/stt/*.rs`, `src-tauri/src/audio/*.rs`, `src-tauri/src/ipc.rs`, `src-tauri/Cargo.toml`, `src/App.svelte`, `src/lib/Dashboard.svelte`, `src/lib/Settings.svelte`, `src/lib/History.svelte`, `src/lib/Pill.svelte`, `src/app.css`, `src/main.ts`, `src/lib/api.ts`

---

## 1. Inference Optimization (CPU / GPU)

### 1.1 What's Already Excellent

The STT backend is **surprisingly well-tuned** for a Phase-1 build. Several production-grade optimizations are already in place:

| Optimization | Location | Impact |
|---|---|---|
| **Adaptive `audio_ctx`** | `stt/whisper.rs:148-155` | ⭐ Biggest win. Scales encoder context to actual utterance length instead of fixed 1500-frame (30 s) window. Cuts encoder cost roughly proportional to clip length. |
| **Thread auto-scaling** | `stt/whisper.rs:135-139` | Uses `available_parallelism()` clamped 2–8. Whisper-tiny benefits from up to ~physical-core count. |
| **Lazy + reusable context** | `stt/whisper.rs:14-18`, `warm()` | Model loads once on first use, then reused for app lifetime. `warm()` preloads in background thread at startup. |
| **Race-free state init** | `stt/whisper.rs:97-121` | Single critical section for both CONTEXT + STATE initialization. Fixes a documented race with concurrent `reset()`. |
| **Streaming resampler** | `audio/resampler.rs` | Rubato-based, real-time 48 kHz → 16 kHz without blocking. |
| **Per-capture thread isolation** | `audio/mod.rs:87-145` | Fresh `Arc<AtomicBool>` per recording prevents stale-sample contamination (the "duplicate recording" bug). |
| **Throttled audio-level IPC** | `audio/capture.rs:281-320` | 60 fps ticker with delta/staleness gates. Never floods the event channel. |
| **Release profile** | `Cargo.toml:90-94` | `lto = true`, `codegen-units = 1`, `panic = "abort"`, `strip = true`. Proper shipping config. |

### 1.2 Issues & Recommendations

#### 🔴 High Priority

**1.2.1 `opt-level = "s"` trades speed for size — wrong default for STT**

```toml
# Cargo.toml:94
opt-level = "s"   # optimizes for binary size
```

For a real-time STT app, **inference latency matters more than binary size** (the model is already 75 MB). Whisper’s encoder/decoder loops are heavily auto-vectorized at `opt-level = 3`.

**Recommendation:**
```toml
[profile.release]
opt-level = 3        # or "z" if you truly need tiny binary, but "s" is the worst of both worlds
# Keep the rest
lto = true
codegen-units = 1
panic = "abort"
strip = true
```
> If binary size is a hard constraint, consider a **dual profile**: `release` (speed) and `release-small` (size) for CI builds.

---

**1.2.2 Missing CPU BLAS/SIMD backend for Whisper**

`whisper-rs` has feature flags for accelerated linear algebra that dramatically speed up the decoder matrices:

```toml
# Cargo.toml:47 — current
whisper-rs = { version = "0.16", default-features = false, features = ["log_backend", "raw-api"] }
```

**Recommendation:** Add at least one CPU BLAS path:
```toml
# macOS
[target.'cfg(target_os = "macos")'.dependencies]
whisper-rs = { features = ["accelerate"] }

# Windows / Linux (requires OpenBLAS dev libs at build time)
[target.'cfg(not(target_os = "macos"))'.dependencies]
whisper-rs = { features = ["openblas"] }
```
> Note: `accelerate` is zero-extra-deps on Apple Silicon and gives ~20-40% speedup on decoder-heavy workloads.

---

**1.2.3 `available_parallelism()` returns logical cores — can oversubscribe on hyper-threaded CPUs**

```rust
// whisper.rs:135-139
let n_threads = std::thread::available_parallelism()
    .map(|n| n.get())
    .unwrap_or(4)
    .clamp(2, 8) as i32;
```

On a 6-core/12-thread Intel CPU, this sets 8 threads. Whisper-tiny’s compute is ALU-bound; hyper-threads fight for the same execution units. **Physical cores are the right ceiling.**

**Recommendation:**
```rust
fn physical_core_count() -> usize {
    // Use num_cpus crate or a simple heuristic
    std::thread::available_parallelism()
        .map(|n| (n.get() / 2).max(1)) // rough physical-core estimate
        .unwrap_or(4)
}
```
> Even better: add the `num_cpus` crate (already pulled in transitively by Tokio) and use `num_cpus::get_physical()`.

---

**1.2.4 Moonshine ONNX sessions ignore the user's acceleration mode**

`ipc.rs:407-420` lets the user pick `auto` / `cpu` / `gpu`, and `probe_acceleration()` detects providers. But `stt/moonshine.rs:75-76` builds sessions with **default** provider selection:

```rust
let enc = Session::builder()?.commit_from_file(ep)?;
let dec = Session::builder()?.commit_from_file(dp)?;
```

This means even if the user picks **GPU**, the ONNX session may silently fall back to CPU, and the UI still reports "GPU in use."

**Recommendation:** Wire the selected provider into `moonshine.rs`:
```rust
use ort::ep::{CUDA, DirectML, CPUExecutionProvider};

let mut builder = Session::builder()?;
match selected_provider {
    "cuda" => builder = builder.with_execution_providers([CUDA::default().build()])?,
    "directml" => builder = builder.with_execution_providers([DirectML::default().build()])?,
    _ => builder = builder.with_execution_providers([CPUExecutionProvider::default().build()])?,
};
let enc = builder.commit_from_file(ep)?;
```
> Also: call `moonshine::reset()` when the user changes acceleration mode in Settings, so the next transcription picks up the new provider without app restart.

---

#### 🟡 Medium Priority

**1.2.5 `audio_ctx` floor of 256 may be too aggressive for very short clips**

```rust
let audio_ctx: i32 = if secs >= 24.0 { 0 } else {
    (((secs * 50.0) as i32) + 80).clamp(256, 1500)
};
```

For a 1-second clip, `audio_ctx = 130` gets clamped to **256**. That’s fine. But for a 3-second clip, `audio_ctx = 230` also gets clamped to 256. The encoder still processes 256 frames (~5.1 s equivalent). **The clamp floor could be lower (150) for sub-3-second clips** — whisper-tiny tolerates it with negligible WER loss.

**Recommendation:** Consider lowering the floor after A/B testing:
```rust
.clamp(150, 1500)
```

---

**1.2.6 No memory pre-allocation for captured audio buffer**

`CAPTURED` starts as an empty `Vec<f32>` and grows dynamically during recording. For a 30-second utterance at 16 kHz, this is ~480k samples × 4 bytes = ~1.9 MB. The reallocs are amortized but still unnecessary.

**Recommendation:** Pre-allocate a reasonable capacity in `audio::start()`:
```rust
// audio/mod.rs:121
CAPTURED.lock().clear();
CAPTURED.lock().reserve(30 * 16_000); // ~30s @ 16kHz
```

---

**1.2.7 `transcribe_buffer` copies the entire sample Vec across the IPC boundary**

```rust
// ipc.rs:216-218
pub async fn transcribe_buffer(samples: Vec<f32>, ...) -> Result<String, String> {
```

Tauri’s IPC serializer copies `Vec<f32>` into a JSON array. For 30s of audio (~480k f32s), that’s ~1.9 MB of data serialized to JSON numbers — extremely slow and memory-heavy.

**Recommendation:** Zero-copy via Tauri’s binary payload support (if available in your Tauri version), or at minimum **base64-encode** the f32 slice to cut serialization overhead. Even better: keep the buffer entirely in Rust land — `start_listening` / `stop_listening` already returns the buffer from Rust. The frontend should **not** be shuttling audio samples; it should tell Rust "transcribe what you just captured."

> This is actually already the case in the hotkey path (Rust captures → Rust transcribes → Rust injects). The `transcribe_buffer` command is only used by the Dashboard IPC smoke-test. Consider **deprecating or removing it** to eliminate this foot-gun.

---

### 1.3 GPU Acceleration Summary

| Path | Current State | Recommendation |
|---|---|---|
| **Whisper** | Compile-time only (`cuda` / `vulkan` features) | Document clearly that users need the special `tauri:build:vulkan` script. Add a runtime warning if `probe_acceleration` sees a GPU but the binary was built CPU-only. |
| **Moonshine** | Runtime provider selection exists but not wired to session builder | Wire `ExecutionProvider` into `moonshine.rs` session builder. Reset sessions on mode change. |
| **UI Feedback** | `AccelerationInfo` shows detected vs in-use | Good. Add a "Restart required" badge when Whisper GPU features are detected as unavailable at runtime but the user selects GPU mode. |

---

## 2. UI / UX Review

### 2.1 What's Already Solid

- **Design system:** Cohesive dark theme with aurora backgrounds, consistent radius scale (`--r-sm` through `--r-xl`), elevation shadows, and accent-color theming.
- **Accessibility:** `aria-label`, `aria-pressed`, `aria-expanded`, `role`, focus-visible outlines, and keyboard shortcuts (Ctrl+D/H/,).
- **Error handling:** Typed `IpcError` discriminated union with user-friendly messages. No raw `throw` strings in production UI.
- **Pill widget:** Glassmorphism, backdrop blur, live waveform, drag-to-move, snap-to-corner presets, size sliders — this is **polished enough to ship**.
- **Settings architecture:** Clear split between `localStorage` (cosmetic) and IPC (backend-consumed). No phantom save buttons.

### 2.2 Issues & Recommendations

#### 🔴 High Priority

**2.2.1 Settings.svelte is ~1500 lines — unmaintainable**

A single component handling models, audio devices, hotkeys, dictionary, pill sizing, font scaling, accent colors, retention, autostart, and acceleration will become a bug farm.

**Recommendation:** Split into sub-components:
```
src/lib/settings/
  SettingsShell.svelte      # nav + scroll container
  GeneralSection.svelte
  RecordingSection.svelte
  PerformanceSection.svelte
  ModelsSection.svelte
  AppearanceSection.svelte
  TextSection.svelte
  AboutSection.svelte
```
Each section receives props + callbacks. The state can stay in a `settings.svelte.ts` runes-based store if cross-section sharing is needed.

---

**2.2.2 No responsive layout for small viewports**

```css
/* App.svelte:224 */
grid-template-columns: 200px 1fr;
```

The sidebar dock is **fixed at 200px**. On a 1280×720 window (common on laptops), the content area is only ~1080px wide. On smaller sizes or split-screen, the dock will crowd the content.

**Recommendation:** Add a collapse breakpoint:
```css
@media (max-width: 900px) {
  .shell { grid-template-columns: 64px 1fr; }
  .dock-name, .dock-item-label { display: none; }
  .dock { min-width: 64px; align-items: center; }
}
@media (max-width: 600px) {
  .shell { grid-template-columns: 1fr; grid-template-rows: auto 1fr; }
  .dock { flex-direction: row; height: auto; position: static; }
}
```

---

**2.2.3 Dashboard "Model" card says "CPU" hardcoded**

```svelte
<!-- Dashboard.svelte:198 -->
<div class="mini-sub">{modelStatus.fileSizeMb} MB · downloaded · CPU</div>
```

If the user builds with `--features cuda` or switches to Moonshine GPU, this still says **CPU**. It undermines the acceleration work.

**Recommendation:** Read `accelerationInfo.inUse` and display accordingly:
```svelte
<div class="mini-sub">{modelStatus.fileSizeMb} MB · downloaded · {accelInfo?.inUse ?? 'CPU'}</div>
```

---

#### 🟡 Medium Priority

**2.2.4 Pill waveform is synthetic, not actual frequency data**

The `tickWaveform` function in `Pill.svelte:141-166` uses sinusoidal math:
```javascript
const ripple = (Math.sin(t * 2.2 + i * 0.62) + 1) / 2;
```

It looks gorgeous but is **not connected to the actual audio spectrum**. Users with musical ears notice the disconnect. The `audio-level` event only carries a single RMS scalar.

**Recommendation:** Enhance the Rust `level_ticker_loop` to compute a **3-bin spectral summary** (low/mid/high) using a lightweight FFT or filter bank, and emit:
```rust
struct AudioSpectrumPayload { low: f32, mid: f32, high: f32, overall: f32 }
```
Then animate the 3 waveform bands independently. This makes the pill feel **alive** and connected to the user’s actual voice timbre.

---

**2.2.5 History list lacks replay / audio preview**

Users often want to verify what they actually said, especially when the transcription looks wrong. The history DB stores text + duration but no audio.

**Recommendation:** (Creative, Phase 2) Save the captured audio buffer as a temporary WAV alongside the history row (with a size cap, e.g., keep last 50 recordings). Add a small ▶️ play button in the history entry to replay the original audio. This is invaluable for debugging STT quality and user training.

---

**2.2.6 Global toast only surfaces errors**

```svelte
<!-- App.svelte:45-53 -->
let toast = $state<{ id: number; message: string } | null>(null);
```

Success states ("Pasted", "Model downloaded", "Dictionary entry added") are invisible. Users lack positive reinforcement.

**Recommendation:** Add a `type: 'error' | 'success' | 'info'` field to the toast:
```svelte
{#if toast}
  <div class="toast" class:success={toast.type === 'success'} ...>
```
With a green left border for success, keeping the red for errors.

---

**2.2.7 No "What's my hotkey?" reminder in the pill idle state**

When the pill is idle, it shows a dot + "Ready". New users forget the binding instantly.

**Recommendation:** Add a **fleeting hotkey hint** that appears for ~2s when the pill transitions to idle after a successful paste, then fades:
```svelte
{#if pillState === 'idle' && showHotkeyHint}
  <span class="hotkey-hint">{currentHotkey}</span>
{/if}
```

---

**2.2.8 Missing loading skeletons**

Settings sections flash "Loading…" text while awaiting IPC. The `History` view has a plain text spinner. This feels unpolished compared to the rest of the app.

**Recommendation:** Add a simple pulse-skeleton component:
```svelte
<!-- SkeletonRow.svelte -->
<div class="skeleton" style="width: {width}%; height: {height}px;"></div>
<style>
  .skeleton {
    background: linear-gradient(90deg, var(--card) 25%, var(--card-2) 50%, var(--card) 75%);
    background-size: 200% 100%;
    animation: shimmer 1.2s infinite;
    border-radius: var(--r-sm);
  }
</style>
```

---

### 2.3 Creative / "Wow Factor" Suggestions

These are not bugs — they're features that would elevate DevWhisp from "solid utility" to "delightful product."

#### 🎨 Visual Polish

1. **Aurora accent glow on the active nav item** — instead of the static purple left-border, animate a subtle radial glow that follows the accent color.

2. **Live transcription preview in the pill** — while holding the hotkey, stream partial Whisper tokens into a small scrolling marquee inside the pill (requires `whisper-rs` streaming API or chunked processing). This gives users **immediate feedback** that speech is being recognized.

3. **Tray icon audio level** — animate the tray icon itself (e.g., fill height) based on `last_audio_level`, so users know the mic is live even if the pill is hidden.

4. **Custom CSS injection** — power users love theming. A "Custom CSS" textarea in Settings → Appearance that injects into a `<style id="user-custom">` block.

#### 🧠 Smart Behaviors

5. **Auto-model switching by utterance length** — if both Whisper-tiny and a larger model are downloaded, route short commands (< 5s) through tiny for speed and longer dictation through the big model for accuracy.

6. **Voice Command Mode** — a special prefix (e.g., saying "Command: delete last") triggers app actions instead of typing. Starts with simple ones: "delete that", "undo", "new line", "capitalize that".

7. **Focus-aware injection** — detect if the focused app is a code editor vs. a chat app, and switch formatting presets automatically (no trailing space in VS Code, trailing space in Slack).

8. **Daily / Weekly stats dashboard** — a small chart (using canvas or SVG) showing words transcribed per day, average WPM, most-used time of day.

#### 🔧 Developer / Power-User

9. **Log viewer panel** — a "Debug" section in Settings that tails the Tauri log file in real time. Invaluable for support requests.

10. **Export formats** — History → Export as Markdown, JSON, or plain text with timestamps. Useful for meeting notes / journaling workflows.

---

## 3. Architecture Nits

| Issue | Location | Fix |
|---|---|---|
| `set_acceleration_mode` only saves a string; doesn't reset active Moonshine sessions | `ipc.rs:318-325` | Call `moonshine::reset()` when the mode changes so the next inference rebuilds sessions with the new provider. |
| `get_model_status` hardcodes expected sizes | `ipc.rs:452-456` | Move into `ModelVariant::expected_size_mb()` to keep source of truth in one place. |
| `download_file` re-creates `reqwest::Client` per call | `model_manager.rs:230` | Use a shared `lazy_static` or `once_cell` client for connection pooling. |
| `Pill.svelte` uses `requestAnimationFrame` for waveform but also reactive `$derived` SVG paths | `Pill.svelte:105-128` | The `$derived` recalculates on every `waveSamples` change (60×/s). This is fine for 28 samples, but for larger arrays, move path generation into the `tickWaveform` loop and mutate DOM directly. |
| `Dashboard.svelte` applies `audioLevel` decay via `setInterval` | `Dashboard.svelte:142-144` | Use `requestAnimationFrame` instead of 90ms interval for smoother meter animation. |
| `App.svelte` shortcut handler catches `Ctrl+D` | `App.svelte:137-141` | This shadows the browser bookmark shortcut. Add a `preventDefault()` but also consider making it `Alt+D` or user-configurable. |

---

## 4. Quick-Win Priority List

If you only have time for a few changes, do these in order:

1. **Change `opt-level = "s"` → `3`** in `Cargo.toml` (1-line, ~10-30% inference speedup).
2. **Add `accelerate` / `openblas` feature to `whisper-rs`** (1-line on macOS, significant decoder speedup).
3. **Wire Moonshine `ExecutionProvider` to user-selected accel mode** (`moonshine.rs`, medium effort, makes GPU setting actually work).
4. **Split `Settings.svelte` into section components** (refactor, pays off immediately for maintainability).
5. **Add responsive breakpoint for sidebar** (`App.svelte`, ~20 lines, makes app usable on small screens).
6. **Show actual acceleration in Dashboard model card** (`Dashboard.svelte`, 1 line, fixes misleading UI).

---

*End of review.*
