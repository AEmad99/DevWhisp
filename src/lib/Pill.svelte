<script lang="ts">
  /**
   * DevWhisp Pill — floating, frameless, always-on-top status widget.
   *
   * Visualizes the current STT state (idle / listening / processing /
   * success / error) and the live audio level. Listens to `audio-level` and
   * `pill-state` events emitted from the Rust backend.
   *
   * A live sound-wave visualizer animates when listening; the pill collapses
   * to a compact "Ready" hint when idle. Drag the bar to move the window;
   * click to fire the push-to-talk hotkey.
   *
   * The pill also shows the brand mark (voice-wave arcs + source node) so it
   * reads as DevWhisp at any size, matching the family DNA.
   */
	  import { onMount, onDestroy } from 'svelte';
	  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
	  import { getCurrentWindow } from '@tauri-apps/api/window';
	  import { invoke } from '@tauri-apps/api/core';
	  import BrandMark from './BrandMark.svelte';
	  import { getHotkey } from './api';

  type PillStateValue = 'idle' | 'listening' | 'processing' | 'success' | 'error';

  interface PillStatePayload {
    state: PillStateValue;
    message?: string;
  }

  interface AudioLevelPayload {
    level: number;
  }

  interface PillSizePayload {
    width: number;
    height: number;
  }

  /** Samples along the listening sound-wave path. */
  const WAVE_SAMPLES = 28;
  const WAVE_WIDTH = 96;
  const WAVE_HEIGHT = 28;

  let pillState = $state<PillStateValue>('idle');
  let errorMessage = $state<string | null>(null);
  let audioLevel = $state(0);

  /** Target level from the backend; displayLevel eases toward it each frame. */
  let targetLevel = 0;
  let displayLevel = 0;
  let waveRaf: number | null = null;

  /** Window size (logical px) broadcast by the backend `pill-size` event. */
  let pillW = $state<number>(200);
  let pillH = $state<number>(48);

  /** Smoothed amplitudes that drive the live sound wave. */
  let waveSamples = $state<number[]>(Array(WAVE_SAMPLES).fill(0.08));

  /** Canonical display form of the active hotkey, shown briefly after success. */
  let currentHotkey = $state('Ctrl+Shift+Space');
  let showHotkeyHint = $state(false);
  let hotkeyHintTimer: number | null = null;

  let unlistenLevel: UnlistenFn | null = null;
  let unlistenState: UnlistenFn | null = null;
  let unlistenStyle: UnlistenFn | null = null;
  let unlistenSize: UnlistenFn | null = null;

  // ---- Pill style customization (persisted to localStorage) ----------
  const PILL_LS_KEY = 'devwhisp.pill.style';
  type PillStyle = { bgAlpha: number; noHalo: boolean; compact: boolean };
  let pillStyle = $state<PillStyle>({ bgAlpha: 0.35, noHalo: false, compact: false });

  function loadPillStyle(): PillStyle {
    try {
      const raw = window.localStorage.getItem(PILL_LS_KEY);
      if (!raw) return { bgAlpha: 0.35, noHalo: false, compact: false };
      const parsed = JSON.parse(raw);
      return {
        bgAlpha: clamp01(typeof parsed.bgAlpha === 'number' ? parsed.bgAlpha : 0.35, 0.08, 0.9),
        noHalo: parsed.noHalo === true,
        compact: parsed.compact === true,
      };
    } catch {
      return { bgAlpha: 0.35, noHalo: false, compact: false };
    }
  }

  function clamp01(v: number, min: number, max: number) {
    return Math.max(min, Math.min(max, v));
  }

  // Apply a style update broadcast from the Settings view. Settings lives in
  // a SEPARATE window, so this arrives via a Tauri event (global emit), not a
  // same-window DOM event.
  function applyStyle(detail: PillStyle | undefined) {
    if (detail && typeof detail.bgAlpha === 'number') {
      pillStyle = {
        bgAlpha: clamp01(detail.bgAlpha, 0.08, 0.9),
        noHalo: detail.noHalo === true,
        compact: detail.compact === true,
      };
    }
  }

  /** Suggested expanded dimensions per state — passed to the window. */
  const expanded = $derived(pillState === 'listening' || pillState === 'success');

  /** SVG paths for the oscillating sound wave (stroke + filled body). */
  const wavePaths = $derived.by(() => {
    const mid = WAVE_HEIGHT / 2;
    const step = WAVE_WIDTH / (WAVE_SAMPLES - 1);
    const tops: string[] = [];
    const bottoms: string[] = [];

    for (let i = 0; i < WAVE_SAMPLES; i++) {
      const x = i * step;
      const sample = waveSamples[i] ?? 0.08;
      const amp = 1.5 + sample * (mid - 1.5);
      const phase = (i / WAVE_SAMPLES) * Math.PI * 5.5;
      const yTop = mid - Math.sin(phase) * amp;
      const yBot = mid + Math.sin(phase) * amp * 0.72;
      tops.push(`${x.toFixed(1)},${yTop.toFixed(1)}`);
      bottoms.push(`${x.toFixed(1)},${yBot.toFixed(1)}`);
    }

    const topLine = `M ${tops.join(' L ')}`;
    const bottomLine = [...bottoms].reverse().join(' L ');
    return {
      stroke: topLine,
      fill: `${topLine} L ${bottomLine} Z`,
    };
  });

  /** Tauri window handle — works inside the pill webview specifically. */
  const pillWindow = getCurrentWindow();

  function setTargetLevel(level: number) {
    targetLevel = Math.min(1, Math.max(0, level));
  }

  /**
   * Drive the sound wave from the eased display level. Fast attack / slower
   * decay keeps speech punchy while avoiding jitter on quiet mics.
   */
  function tickWaveform(now: number) {
    const diff = targetLevel - displayLevel;
    const easing = diff > 0 ? 0.42 : 0.2;
    displayLevel += diff * easing;
    audioLevel = displayLevel;

    const t = now * 0.0045;
    const energy = Math.min(1, displayLevel * 1.08);
    const next: number[] = [];

    for (let i = 0; i < WAVE_SAMPLES; i++) {
      const centerBias = 1 - Math.abs(i / (WAVE_SAMPLES - 1) - 0.5) * 0.22;
      const ripple = (Math.sin(t * 2.2 + i * 0.62) + 1) / 2;
      const sparkle = (Math.sin(t * 3.6 + i * 1.15) + 1) / 2;
      const target = Math.min(
        1,
        energy * centerBias * (0.28 + ripple * 0.52 + sparkle * 0.2),
      );
      const floor = 0.06 + energy * 0.14;
      const prev = waveSamples[i] ?? floor;
      const sampleEase = target > prev ? 0.55 : 0.28;
      next.push(prev + (Math.max(floor, target) - prev) * sampleEase);
    }
    waveSamples = next;
    waveRaf = requestAnimationFrame(tickWaveform);
  }

  function startWaveLoop() {
    if (waveRaf !== null) return;
    waveRaf = requestAnimationFrame(tickWaveform);
  }

  function stopWaveLoop() {
    if (waveRaf !== null) {
      cancelAnimationFrame(waveRaf);
      waveRaf = null;
    }
    targetLevel = 0;
    displayLevel = 0;
    audioLevel = 0;
    waveSamples = Array(WAVE_SAMPLES).fill(0.08);
  }



  // Drag state
  let dragStartX = 0;
  let dragStartY = 0;
  let isDragging = $state(false);
  let didDrag = $state(false);
  let dragUnlisten: (() => void) | null = null;

  /**
   * Begin dragging the pill. We track pointer movement at the window
   * level (not the pill level) so the cursor can leave the pill bounds
   * without losing the drag.
   */
  async function startDrag(event: PointerEvent) {
    // Ignore drags that start on interactive controls (buttons, textarea) so quick-edit
    // feels solid — you can click/select text without moving the pill.
    const target = event.target as HTMLElement | null;
    if (target && (target.closest('button') || target.closest('textarea'))) {
      return;
    }
    isDragging = true;
    didDrag = false;
    dragStartX = event.screenX;
    dragStartY = event.screenY;

    try {
      await pillWindow.startDragging();
    } catch (err) {
      // Fallback: manual drag — track delta and persist via save_pill_position
      console.warn('startDragging failed, using manual fallback:', err);
      const startX = event.screenX;
      const startY = event.screenY;
      let lastX = 0;
      let lastY = 0;
      const onMove = (ev: PointerEvent) => {
        lastX = (ev.screenX - startX);
        lastY = (ev.screenY - startY);
        if (Math.abs(lastX) > 3 || Math.abs(lastY) > 3) {
          didDrag = true;
        }
      };
      const onUp = () => {
        window.removeEventListener('pointermove', onMove);
        window.removeEventListener('pointerup', onUp);
        if (didDrag) {
          void invoke('save_pill_position', { x: lastX, y: lastY }).catch(() => undefined);
        }
        isDragging = false;
      };
      window.addEventListener('pointermove', onMove);
      window.addEventListener('pointerup', onUp);
    }
  }

  // Clean up any global pointer listeners on unmount
  function cleanupDrag() {
    if (dragUnlisten) {
      dragUnlisten();
      dragUnlisten = null;
    }
  }

  /** Idle click → simulate the push-to-talk hotkey. */
  async function onClickIdle(event: MouseEvent) {
    // Don't trigger hotkey if we just finished a drag
    if (didDrag) {
      didDrag = false;
      return;
    }
    event.stopPropagation();
    try {
      await invoke('trigger_hotkey');
    } catch (e) {
      console.warn('trigger_hotkey failed', e);
    }
  }

  function close() {
    void invoke('hide_pill').catch(() => undefined);
  }

  onMount(async () => {
    pillStyle = loadPillStyle();

    // Respect the "Show pill on startup" preference (shared via localStorage,
    // same origin as the main window). If explicitly off, hide on launch.
    try {
      const raw = window.localStorage.getItem('devwhisp.settings.showPillOnStartup');
      if (raw !== null && JSON.parse(raw) === false) {
        void invoke('hide_pill').catch(() => {});
      }
    } catch {
      /* ignore */
    }

    try {
      currentHotkey = await getHotkey();
    } catch (e) {
      console.warn('getHotkey failed', e);
    }

    try {
      unlistenStyle = await listen<PillStyle>('pill-style', (event) => {
        applyStyle(event.payload);
      });
    } catch (e) {
      console.warn('pill-style listen failed', e);
    }
    try {
      unlistenLevel = await listen<AudioLevelPayload>('audio-level', (event) => {
        const lvl = Number(event.payload?.level ?? 0);
        if (Number.isFinite(lvl)) setTargetLevel(lvl);
      });
    } catch (e) {
      console.warn('audio-level listen failed', e);
    }
    try {
      unlistenState = await listen<PillStatePayload>('pill-state', (event) => {
        const next = event.payload?.state ?? 'idle';
        const prev = pillState;

        pillState = next;
        errorMessage = event.payload?.message ?? null;
        if (next !== 'error') {
          errorMessage = null;
        }
        if (next === 'listening') {
          startWaveLoop();
        } else {
          stopWaveLoop();
          if (next === 'idle' && prev === 'success') {
            // Fleeting hotkey reminder after a successful paste.
            showHotkeyHint = true;
            if (hotkeyHintTimer !== null) window.clearTimeout(hotkeyHintTimer);
            hotkeyHintTimer = window.setTimeout(() => {
              showHotkeyHint = false;
            }, 2000);
          }
        }
      });
    } catch (e) {
      console.warn('pill-state listen failed', e);
    }

    // (partial-transcript listener removed — no transcript text is rendered in the pill)

    // Listen for size changes broadcast from the backend when the user
    // adjusts the width/height sliders in Settings.
    try {
      unlistenSize = await listen<PillSizePayload>('pill-size', (event) => {
        const w = Number(event.payload?.width);
        const h = Number(event.payload?.height);
        if (Number.isFinite(w) && w > 0) pillW = w;
        if (Number.isFinite(h) && h > 0) pillH = h;
      });
    } catch (e) {
      console.warn('pill-size listen failed', e);
    }
  });

  onDestroy(() => {
    stopWaveLoop();
    if (unlistenLevel) unlistenLevel();
    if (unlistenState) unlistenState();
    if (unlistenStyle) unlistenStyle();
    if (unlistenSize) unlistenSize();
    if (hotkeyHintTimer !== null) window.clearTimeout(hotkeyHintTimer);
    cleanupDrag();
  });
</script>

<div
  class="pill"
  class:expanded
  class:listening={pillState === 'listening'}
  class:processing={pillState === 'processing'}
  class:success={pillState === 'success'}
  class:error={pillState === 'error'}
  class:no-halo={pillStyle.noHalo}
  class:compact={pillStyle.compact}
  class:dragging={isDragging}
  data-state={pillState}
  style:--pill-bg-alpha={pillStyle.bgAlpha.toFixed(2)}
  style:--pill-w={pillW + 'px'}
  style:--pill-h={pillH + 'px'}
  onpointerdown={startDrag}
  role="status"
  aria-live="polite"
>
	  <!-- Brand mark — voice-wave arcs from a source node. Matches the
	       canonical app icon motif exactly so the pill feels like part of the family. -->
	  <span class="brand-mark" aria-hidden="true">
	    <BrandMark size={20} gradient={false} />
	  </span>

  {#if pillState === 'listening'}
    <div class="icon">
      <span class="dot live" aria-hidden="true"></span>
    </div>
    <div class="waveform" aria-hidden="true">
      <svg
        viewBox="0 0 {WAVE_WIDTH} {WAVE_HEIGHT}"
        preserveAspectRatio="none"
        role="presentation"
      >
        <defs>
          <linearGradient id="pill-wave-fill" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stop-color="rgba(255, 255, 255, 0.55)" />
            <stop offset="100%" stop-color="rgba(255, 255, 255, 0.08)" />
          </linearGradient>
        </defs>
        <path class="wave-fill" d={wavePaths.fill} />
        <path class="wave-line" d={wavePaths.stroke} />
        <path class="wave-echo" d={wavePaths.stroke} />
      </svg>
    </div>
    <button class="close" onclick={(e) => { e.stopPropagation(); close(); }} title="Hide pill" aria-label="Hide pill">×</button>
  {:else if pillState === 'processing'}
    <div class="icon">
      <span class="spinner" aria-hidden="true"></span>
    </div>
    <div class="label">Processing…</div>
    <button class="close" onclick={(e) => { e.stopPropagation(); close(); }} title="Hide pill" aria-label="Hide pill">×</button>
  {:else if pillState === 'success'}
    <div class="icon">
      <span class="check" aria-hidden="true">✓</span>
    </div>
    <div class="label">Pasted</div>
    <button class="close" onclick={(e) => { e.stopPropagation(); close(); }} title="Hide pill" aria-label="Hide pill">×</button>
  {:else if pillState === 'error'}
    <div class="icon">
      <span class="dot err" aria-hidden="true"></span>
    </div>
    <div class="label err" title={errorMessage ?? undefined}>
      {errorMessage ?? 'Something went wrong'}
    </div>
    <button class="close" onclick={(e) => { e.stopPropagation(); close(); }} title="Hide pill" aria-label="Hide pill">×</button>
  {:else}
    <!-- idle -->
    <button class="idle-button" onclick={onClickIdle} aria-label="Start push-to-talk">
      <div class="icon">
        <span class="dot idle" aria-hidden="true"></span>
      </div>
      <div class="label">Ready</div>
      {#if showHotkeyHint}
        <span class="hotkey-hint">{currentHotkey}</span>
      {/if}
    </button>
  {/if}
</div>

<style>
  :global(html),
  :global(body) {
    margin: 0;
    padding: 0;
    background: transparent;
    overflow: hidden;
    height: 100%;
    color-scheme: dark;
  }

  :global(#pill) {
    height: 100vh;
    width: 100vw;
    display: flex;
    align-items: center;
    justify-content: center;
    background: transparent;
  }

  .pill {
    /* Background alpha is set per-instance from JS via --pill-bg-alpha.
       Default 0.35 reads cleanly on top of any app background. */
    --pill-bg-alpha: 0.35;
    /* User-driven window size from Settings sliders; reflows the content
       so it always fits whatever window dimensions the backend set. */
    --pill-w: 200px;
    --pill-h: 48px;
    --bg: rgba(20, 16, 30, var(--pill-bg-alpha));
    --border: rgba(255, 255, 255, 0.18);
    --fg: rgba(243, 240, 251, 0.95);
    --muted: rgba(243, 240, 251, 0.6);
    --accent: #c4b5fd;
    --accent-deep: #7c3aed;
    --danger: #ff5c7c;
    --ok: #5cff9c;

    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 4px 10px;
    width: var(--pill-w);
    height: var(--pill-h);
    box-sizing: border-box;
    border-radius: 999px;
    background: var(--bg);
    border: 1px solid var(--border);
    backdrop-filter: blur(20px) saturate(160%);
    -webkit-backdrop-filter: blur(20px) saturate(160%);
    color: var(--fg);
    font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Inter, system-ui, sans-serif;
    font-size: 12px;
    user-select: none;
    cursor: grab;
    overflow: hidden;
    box-shadow:
      0 2px 8px rgba(0, 0, 0, 0.18),
      0 0 0 1px rgba(124, 58, 237, 0.06);
    transition:
      background 220ms ease,
      box-shadow 220ms ease;
  }

  .pill.dragging {
    cursor: grabbing;
  }

  /* Drop the halo entirely when the user wants a flat pill. */
  .pill.no-halo {
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.2);
  }

  /* Expanded state (listening / success) gets a touch more height for the
     waveform + textarea. Width is whatever the user picked in Settings. */
  .pill.expanded {
    padding: 6px 12px;
    cursor: grab;
  }

  .pill.listening {
    --bg: rgba(24, 16, 38, calc(var(--pill-bg-alpha) * 1.75));
    --border: rgba(255, 255, 255, 0.22);
    box-shadow:
      0 0 0 1px rgba(196, 181, 253, 0.18),
      0 4px 18px rgba(124, 58, 237, 0.35);
  }

  .pill.processing {
    --bg: rgba(124, 58, 237, calc(var(--pill-bg-alpha) * 1.2));
    --border: rgba(196, 181, 253, 0.4);
  }

  .pill.success {
    --bg: rgba(92, 255, 156, calc(var(--pill-bg-alpha) * 1.2));
    --border: rgba(92, 255, 156, 0.55);
  }

  .pill.error {
    --bg: rgba(255, 92, 124, calc(var(--pill-bg-alpha) * 1.2));
    --border: rgba(255, 92, 124, 0.65);
  }

  .brand-mark {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    width: 20px;
    height: 20px;
  }
  .brand-mark :global(svg) { display: block; }

  .icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 18px;
    height: 18px;
    flex-shrink: 0;
  }

  .dot {
    display: inline-block;
    width: 10px;
    height: 10px;
    border-radius: 999px;
    background: var(--accent);
    box-shadow: 0 0 8px rgba(196, 181, 253, 0.55);
  }
  .dot.idle {
    background: var(--muted);
    box-shadow: none;
  }
  .dot.live {
    background: var(--danger);
    animation: pulse 1.2s ease-in-out infinite;
  }
  .dot.err {
    background: var(--danger);
  }

  @keyframes pulse {
    0%,
    100% {
      opacity: 1;
      transform: scale(1);
    }
    50% {
      opacity: 0.55;
      transform: scale(0.8);
    }
  }

  .spinner {
    width: 12px;
    height: 12px;
    border-radius: 999px;
    border: 2px solid rgba(196, 181, 253, 0.25);
    border-top-color: var(--accent);
    animation: spin 700ms linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .check {
    color: var(--ok);
    font-weight: 700;
    font-size: 14px;
    line-height: 1;
  }

  .label {
    font-weight: 500;
    letter-spacing: 0.01em;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 200px;
  }
  .label.err {
    color: var(--danger);
    max-width: 180px;
  }

  .waveform {
    flex: 1 1 0;
    min-width: 48px;
    max-width: 120px;
    height: 28px;
    padding: 2px 6px;
    border-radius: 8px;
    background: rgba(0, 0, 0, 0.42);
    border: 1px solid rgba(255, 255, 255, 0.16);
    box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.08);
  }

  .waveform svg {
    display: block;
    width: 100%;
    height: 100%;
    overflow: visible;
  }

  .wave-fill {
    fill: url(#pill-wave-fill);
  }

  .wave-line {
    fill: none;
    stroke: #ffffff;
    stroke-width: 2.5;
    stroke-linecap: round;
    stroke-linejoin: round;
    filter: drop-shadow(0 0 6px rgba(255, 255, 255, 0.9));
  }

  .wave-echo {
    fill: none;
    stroke: rgba(255, 255, 255, 0.35);
    stroke-width: 5;
    stroke-linecap: round;
    stroke-linejoin: round;
    opacity: 0.7;
  }

  .pill.listening .waveform {
    background: rgba(0, 0, 0, 0.55);
    border-color: rgba(255, 255, 255, 0.24);
    box-shadow:
      inset 0 1px 0 rgba(255, 255, 255, 0.1),
      0 0 12px rgba(255, 255, 255, 0.08);
  }

  .close {
    appearance: none;
    background: transparent;
    color: var(--muted);
    border: none;
    font-size: 18px;
    line-height: 1;
    width: 20px;
    height: 20px;
    border-radius: 999px;
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    transition: color 120ms ease, background 120ms ease;
  }
  .close:hover {
    color: var(--fg);
    background: rgba(255, 255, 255, 0.08);
  }

  .idle-button {
    appearance: none;
    background: transparent;
    border: none;
    color: inherit;
    font: inherit;
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 0;
    cursor: pointer;
    flex: 1;
    justify-content: center;
  }
  .idle-button:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
    border-radius: 999px;
  }
  .hotkey-hint {
    position: absolute;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    font-size: 10px;
    font-weight: 600;
    color: var(--muted);
    background: rgba(20, 16, 30, 0.85);
    border: 1px solid var(--border);
    border-radius: 999px;
    padding: 2px 8px;
    opacity: 0;
    pointer-events: none;
    animation: hint-fade 2s ease-out forwards;
    white-space: nowrap;
  }
  @keyframes hint-fade {
    0% { opacity: 0; transform: translate(-50%, -40%); }
    12% { opacity: 1; transform: translate(-50%, -50%); }
    70% { opacity: 1; transform: translate(-50%, -50%); }
    100% { opacity: 0; transform: translate(-50%, -60%); }
  }

  /* --- Refinements ------------------------------------------------------- */
  .pill {
    position: relative;
    animation: pill-in 280ms cubic-bezier(0.22, 1, 0.36, 1);
  }
  /* Subtle top glass highlight for depth (sits below content). */
  .pill::before {
    content: '';
    position: absolute;
    inset: 0;
    border-radius: inherit;
    background: linear-gradient(180deg, rgba(255, 255, 255, 0.07), transparent 38%);
    pointer-events: none;
  }
  @keyframes pill-in {
    from { opacity: 0; transform: translateY(6px) scale(0.96); }
    to { opacity: 1; transform: none; }
  }
  /* Idle "ready" breathing on the brand mark. */
  .pill[data-state='idle'] .brand-mark {
    animation: breathe 4s ease-in-out infinite;
  }
  @keyframes breathe {
    0%, 100% { transform: scale(1); opacity: 0.9; filter: drop-shadow(0 0 0 transparent); }
    50% { transform: scale(1.07); opacity: 1; filter: drop-shadow(0 0 6px rgba(196, 181, 253, 0.6)); }
  }
  /* Listening glow pulse layered over the entrance. */
  .pill.listening {
    animation: pill-in 280ms cubic-bezier(0.22, 1, 0.36, 1), listen-glow 2.1s ease-in-out infinite;
  }
  @keyframes listen-glow {
    0%, 100% { box-shadow: 0 0 0 1px rgba(196, 181, 253, 0.16), 0 4px 16px rgba(124, 58, 237, 0.3); }
    50% { box-shadow: 0 0 0 1px rgba(255, 255, 255, 0.22), 0 6px 22px rgba(124, 58, 237, 0.48); }
  }
  /* Success check pop. */
  .check { display: inline-block; animation: pop 300ms cubic-bezier(0.34, 1.56, 0.64, 1); }
  @keyframes pop {
    from { transform: scale(0.2); opacity: 0; }
    to { transform: scale(1); opacity: 1; }
  }

</style>
