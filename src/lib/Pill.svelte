<script lang="ts">
  /**
   * DevWhisp Pill — floating, frameless, always-on-top status widget.
   *
   * Compact glass capsule. States: idle / listening / paused / processing / success / error.
   * Live EQ bars while listening. Drag to move; click idle to trigger PTT.
   *
   * The capsule is inset inside the transparent window so shadows and rounded
   * edges are never clipped by the OS window bounds.
   */
  import { onMount, onDestroy } from 'svelte';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { invoke } from '@tauri-apps/api/core';
  import BrandMark from './BrandMark.svelte';
  import AppIcon from './AppIcon.svelte';
  import { downloadStore } from './downloadStore';
  import { getHotkey } from './api';

  type PillStateValue = 'idle' | 'listening' | 'paused' | 'processing' | 'success' | 'error';

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

  const BAR_COUNT = 9;

  let pillState = $state<PillStateValue>('idle');
  let errorMessage = $state<string | null>(null);
  let audioLevel = $state(0);

  let targetLevel = 0;
  let displayLevel = 0;
  let waveRaf: number | null = null;

  // Match OS window defaults (pill_window.rs) — shell insets the capsule.
  let pillW = $state<number>(196);
  let pillH = $state<number>(48);

  let bars = $state<number[]>(Array(BAR_COUNT).fill(0.12));

  let currentHotkey = $state('Ctrl+Shift+Space');
  let showHotkeyHint = $state(false);
  let hotkeyHintTimer: number | null = null;

  let unlistenLevel: UnlistenFn | null = null;
  let unlistenState: UnlistenFn | null = null;
  let unlistenStyle: UnlistenFn | null = null;
  let unlistenSize: UnlistenFn | null = null;

  const PILL_LS_KEY = 'devwhisp.pill.style';
  type PillStyle = { bgAlpha: number; noHalo: boolean; compact: boolean };
  let pillStyle = $state<PillStyle>({ bgAlpha: 0.55, noHalo: false, compact: false });

  function loadPillStyle(): PillStyle {
    try {
      const raw = window.localStorage.getItem(PILL_LS_KEY);
      if (!raw) return { bgAlpha: 0.55, noHalo: false, compact: false };
      const parsed = JSON.parse(raw);
      return {
        bgAlpha: clamp01(typeof parsed.bgAlpha === 'number' ? parsed.bgAlpha : 0.55, 0.08, 0.9),
        noHalo: parsed.noHalo === true,
        compact: parsed.compact === true,
      };
    } catch {
      return { bgAlpha: 0.55, noHalo: false, compact: false };
    }
  }

  function clamp01(v: number, min: number, max: number) {
    return Math.max(min, Math.min(max, v));
  }

  function applyStyle(detail: PillStyle | undefined) {
    if (detail && typeof detail.bgAlpha === 'number') {
      pillStyle = {
        bgAlpha: clamp01(detail.bgAlpha, 0.08, 0.9),
        noHalo: detail.noHalo === true,
        compact: detail.compact === true,
      };
    }
  }

  const expanded = $derived(pillState === 'listening' || pillState === 'paused' || pillState === 'success');

  const pillWindow = getCurrentWindow();

  function setTargetLevel(level: number) {
    targetLevel = Math.min(1, Math.max(0, level));
  }

  function tickWaveform(now: number) {
    const diff = targetLevel - displayLevel;
    const easing = diff > 0 ? 0.45 : 0.18;
    displayLevel += diff * easing;
    audioLevel = displayLevel;

    const t = now * 0.005;
    const energy = Math.min(1, displayLevel * 1.1);
    const next: number[] = [];

    for (let i = 0; i < BAR_COUNT; i++) {
      const center = 1 - Math.abs(i / (BAR_COUNT - 1) - 0.5) * 0.55;
      const ripple = (Math.sin(t * 2.4 + i * 0.85) + 1) / 2;
      const spark = (Math.sin(t * 3.8 + i * 1.4) + 1) / 2;
      const target = Math.min(1, energy * center * (0.22 + ripple * 0.55 + spark * 0.23));
      const floor = 0.1 + energy * 0.08;
      const prev = bars[i] ?? floor;
      const ease = target > prev ? 0.58 : 0.26;
      next.push(prev + (Math.max(floor, target) - prev) * ease);
    }
    bars = next;
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
    bars = Array(BAR_COUNT).fill(0.12);
  }

  let isDragging = $state(false);
  let didDrag = $state(false);
  let dragUnlisten: (() => void) | null = null;

  async function startDrag(event: PointerEvent) {
    const target = event.target as HTMLElement | null;
    if (target && (target.closest('button') || target.closest('textarea'))) {
      return;
    }
    isDragging = true;
    didDrag = false;

    try {
      await pillWindow.startDragging();
    } catch (err) {
      console.warn('startDragging failed, using manual fallback:', err);
      const startX = event.screenX;
      const startY = event.screenY;
      let lastX = 0;
      let lastY = 0;
      const onMove = (ev: PointerEvent) => {
        lastX = ev.screenX - startX;
        lastY = ev.screenY - startY;
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

  function cleanupDrag() {
    if (dragUnlisten) {
      dragUnlisten();
      dragUnlisten = null;
    }
  }

  async function onClickIdle(event: MouseEvent) {
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
    downloadStore.init();
    pillStyle = loadPillStyle();

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
        const nextMsg = event.payload?.message ?? null;
        const prev = pillState;

        if (next === prev && (next !== 'error' || nextMsg === errorMessage)) {
          return;
        }

        pillState = next;
        errorMessage = next !== 'error' ? null : nextMsg;
        if (next === 'listening' || next === 'paused') {
          startWaveLoop();
        } else {
          stopWaveLoop();
          if (next === 'idle' && prev === 'success') {
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

<!--
  Outer shell fills the transparent OS window.
  Inner .pill is inset so rounded corners + glow never get hard-clipped.
-->
<div
  class="pill-shell"
  style:--pill-w={pillW + 'px'}
  style:--pill-h={pillH + 'px'}
>
  <div
    class="pill"
    class:expanded
    class:listening={pillState === 'listening'}
    class:processing={pillState === 'processing'}
    class:success={pillState === 'success'}
    class:error={pillState === 'error'}
    class:paused={pillState === 'paused'}
    class:no-halo={pillStyle.noHalo}
    class:compact={pillStyle.compact}
    class:dragging={isDragging}
    data-state={pillState}
    style:--pill-bg-alpha={pillStyle.bgAlpha.toFixed(2)}
    onpointerdown={startDrag}
    role="status"
    aria-live="polite"
  >
    {#if pillState === 'listening'}
      <span class="live-ring" aria-hidden="true">
        <span class="live-core"></span>
      </span>
      <div class="eq" aria-hidden="true">
        {#each bars as mag, i (i)}
          <span class="eq-bar" style:--m={mag.toFixed(3)}></span>
        {/each}
      </div>
      <span class="label listening-label">Listening</span>
      <button class="close" onclick={(e) => { e.stopPropagation(); close(); }} title="Hide pill" aria-label="Hide pill">
        <svg viewBox="0 0 12 12" width="10" height="10" aria-hidden="true">
          <path d="M2 2l8 8M10 2L2 10" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" />
        </svg>
      </button>
    {:else if pillState === 'paused'}
      <span class="pause-ring" aria-hidden="true">
        <span class="pause-core"></span>
      </span>
      <div class="eq" aria-hidden="true">
        {#each bars as mag, i (i)}
          <span class="eq-bar" style:--m={mag.toFixed(3)}></span>
        {/each}
      </div>
      <span class="label paused-label">Paused</span>
      <button class="close" onclick={(e) => { e.stopPropagation(); close(); }} title="Hide pill" aria-label="Hide pill">
        <svg viewBox="0 0 12 12" width="10" height="10" aria-hidden="true">
          <path d="M2 2l8 8M10 2L2 10" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" />
        </svg>
      </button>
    {:else if pillState === 'processing'}
      <span class="spinner" aria-hidden="true"></span>
      <div class="label">Transcribing</div>
      <button class="close" onclick={(e) => { e.stopPropagation(); close(); }} title="Hide pill" aria-label="Hide pill">
        <svg viewBox="0 0 12 12" width="10" height="10" aria-hidden="true">
          <path d="M2 2l8 8M10 2L2 10" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" />
        </svg>
      </button>
    {:else if pillState === 'success'}
      <span class="check" aria-hidden="true">
        <svg viewBox="0 0 16 16" width="12" height="12">
          <path d="M3 8.5l3.2 3.2L13 4.5" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" />
        </svg>
      </span>
      <div class="label">Pasted</div>
      <button class="close" onclick={(e) => { e.stopPropagation(); close(); }} title="Hide pill" aria-label="Hide pill">
        <svg viewBox="0 0 12 12" width="10" height="10" aria-hidden="true">
          <path d="M2 2l8 8M10 2L2 10" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" />
        </svg>
      </button>
    {:else if pillState === 'error'}
      <span class="err-mark" aria-hidden="true">!</span>
      <div class="label err" title={errorMessage ?? undefined}>
        {errorMessage ?? 'Error'}
      </div>
      <button class="close" onclick={(e) => { e.stopPropagation(); close(); }} title="Hide pill" aria-label="Hide pill">
        <svg viewBox="0 0 12 12" width="10" height="10" aria-hidden="true">
          <path d="M2 2l8 8M10 2L2 10" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" />
        </svg>
      </button>
    {:else if downloadStore.isDownloading}
      <span class="spinner" aria-hidden="true"></span>
      <div class="label" title="Downloading model: {downloadStore.pct.toFixed(1)}%">
        Downloading {downloadStore.pct.toFixed(0)}%
      </div>
      <button class="close" onclick={(e) => { e.stopPropagation(); close(); }} title="Hide pill" aria-label="Hide pill">
        <svg viewBox="0 0 12 12" width="10" height="10" aria-hidden="true">
          <path d="M2 2l8 8M10 2L2 10" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" />
        </svg>
      </button>
    {:else}
      <button class="idle-button" onclick={onClickIdle} aria-label="Start push-to-talk">
        <span class="brand-mark" aria-hidden="true">
          <AppIcon size={16} />
        </span>
        <span class="label">Ready</span>
        {#if showHotkeyHint}
          <span class="hotkey-hint">{currentHotkey}</span>
        {/if}
      </button>
    {/if}
  </div>
</div>

<style>
  :global(html),
  :global(body) {
    margin: 0;
    padding: 0;
    background: transparent;
    /* Clip only the OS window chrome — shell keeps the capsule inset. */
    overflow: hidden;
    height: 100%;
    width: 100%;
    color-scheme: dark;
  }

  :global(#pill) {
    height: 100%;
    width: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    overflow: hidden;
    box-sizing: border-box;
  }

  /*
   * Fills the OS window. Generous safe inset is critical on Windows WebView2:
   * transparent frameless windows hard-clip anti-aliased edges at x=0.
   * Left padding is slightly larger so the brand mark never looks sliced.
   */
  .pill-shell {
    --pill-w: 196px;
    --pill-h: 48px;
    --safe-x: 10px;
    --safe-y: 8px;
    width: 100%;
    height: 100%;
    box-sizing: border-box;
    padding: var(--safe-y) var(--safe-x);
    display: flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    overflow: hidden;
  }

  .pill {
    --pill-bg-alpha: 0.55;
    --bg: rgba(16, 12, 28, var(--pill-bg-alpha));
    --border: rgba(255, 255, 255, 0.16);
    --fg: rgba(250, 248, 255, 0.96);
    --muted: rgba(250, 248, 255, 0.58);
    --accent: #c4b5fd;
    --accent-deep: #7c3aed;
    --danger: #ff6b8a;
    --ok: #4ade80;

    position: relative;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    /* Inset body — never flush against the OS window edge */
    width: 100%;
    height: 100%;
    min-width: 0;
    box-sizing: border-box;
    /* Extra left padding: brand mark + AA on left edge of capsule */
    padding: 0 14px 0 16px;
    border-radius: 999px;
    background: var(--bg);
    border: 1px solid var(--border);
    backdrop-filter: blur(22px) saturate(180%);
    -webkit-backdrop-filter: blur(22px) saturate(180%);
    color: var(--fg);
    font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Inter, system-ui, sans-serif;
    font-size: 11px;
    font-weight: 500;
    letter-spacing: 0.01em;
    user-select: none;
    cursor: grab;
    /* Visible so soft shadow can sit inside the shell safe inset */
    overflow: visible;
    box-shadow:
      0 4px 16px rgba(0, 0, 0, 0.45),
      0 0 0 1px rgba(255, 255, 255, 0.06);
    transition:
      background 200ms ease,
      box-shadow 200ms ease,
      border-color 200ms ease;
    animation: pill-in 240ms cubic-bezier(0.22, 1, 0.36, 1);
  }

  .pill.dragging {
    cursor: grabbing;
  }

  .pill.no-halo {
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.3);
  }

  .pill.compact {
    gap: 5px;
    padding: 0 12px 0 14px;
    font-size: 10px;
  }

  .pill.listening {
    --bg: rgba(20, 16, 32, calc(var(--pill-bg-alpha) * 1.2));
    --border: rgba(167, 139, 250, 0.4);
    box-shadow: 0 4px 20px rgba(0, 0, 0, 0.55);
  }

  .pill.paused {
    --bg: rgba(36, 26, 12, calc(var(--pill-bg-alpha) * 1.2));
    --border: rgba(251, 191, 36, 0.4);
    box-shadow: 0 4px 20px rgba(0, 0, 0, 0.55);
  }

  .pill.processing {
    --bg: rgba(30, 24, 48, calc(var(--pill-bg-alpha) * 1.2));
    --border: rgba(167, 139, 250, 0.4);
  }

  .pill.success {
    --bg: rgba(16, 44, 30, calc(var(--pill-bg-alpha) * 1.15));
    --border: rgba(52, 211, 153, 0.4);
  }

  .pill.error {
    --bg: rgba(50, 18, 26, calc(var(--pill-bg-alpha) * 1.15));
    --border: rgba(248, 113, 113, 0.45);
  }

  .brand-mark {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    width: 16px;
    height: 16px;
    opacity: 0.95;
    /* Keep mark fully inside capsule */
    overflow: visible;
  }
  .brand-mark :global(svg) {
    display: block;
    overflow: visible;
    max-width: 100%;
    max-height: 100%;
  }

  .pill[data-state='idle'] .brand-mark {
    animation: breathe 3.6s ease-in-out infinite;
  }

  .live-ring {
    position: relative;
    width: 12px;
    height: 12px;
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }
  .live-core {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--danger);
    animation: pulse 1.1s ease-in-out infinite;
  }
  .live-ring::after {
    content: '';
    position: absolute;
    inset: -2px;
    border-radius: 50%;
    border: 1.5px solid rgba(248, 113, 113, 0.45);
    animation: ring 1.4s ease-out infinite;
  }

  .pause-ring {
    position: relative;
    width: 12px;
    height: 12px;
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }
  .pause-core {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: #fbbf24;
    animation: pulse 1.6s ease-in-out infinite;
  }
  .pause-ring::after {
    content: '';
    position: absolute;
    inset: -2px;
    border-radius: 50%;
    border: 1.5px solid rgba(251, 191, 36, 0.35);
    animation: ring 1.8s ease-out infinite;
  }

  .eq {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 2px;
    flex: 1 1 0;
    min-width: 36px;
    max-width: 64px;
    height: 14px;
    padding: 0 4px;
    border-radius: 5px;
    background: rgba(0, 0, 0, 0.35);
    border: 1px solid rgba(255, 255, 255, 0.08);
  }
  .eq-bar {
    width: 2.5px;
    height: 100%;
    border-radius: 2px;
    background: var(--accent);
    transform-origin: center;
    transform: scaleY(calc(0.14 + var(--m, 0.12) * 0.86));
    opacity: 0.95;
  }

  .spinner {
    width: 11px;
    height: 11px;
    border-radius: 50%;
    border: 1.8px solid rgba(var(--accent-rgb), 0.25);
    border-top-color: var(--accent);
    flex-shrink: 0;
    animation: spin 650ms linear infinite;
  }

  .check {
    display: inline-flex;
    color: var(--ok);
    flex-shrink: 0;
    animation: pop 280ms cubic-bezier(0.34, 1.56, 0.64, 1);
  }

  .err-mark {
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: rgba(255, 107, 138, 0.2);
    border: 1px solid rgba(255, 107, 138, 0.55);
    color: var(--danger);
    font-size: 10px;
    font-weight: 700;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    line-height: 1;
  }

  .label {
    font-weight: 550;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 96px;
    flex-shrink: 1;
    min-width: 0;
  }
  .label.err {
    color: #ffb0c0;
    max-width: 100px;
  }
  .listening-label {
    color: rgba(255, 255, 255, 0.9);
    font-size: 10.5px;
    max-width: 54px;
  }
  .paused-label {
    color: rgba(251, 231, 160, 0.95);
    font-size: 10.5px;
    max-width: 54px;
  }

  .close {
    appearance: none;
    background: transparent;
    color: var(--muted);
    border: none;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    flex-shrink: 0;
    margin-left: auto;
    transition:
      color 120ms ease,
      background 120ms ease;
  }
  .close:hover {
    color: var(--fg);
    background: rgba(255, 255, 255, 0.1);
  }

  .idle-button {
    appearance: none;
    background: transparent;
    border: none;
    color: inherit;
    font: inherit;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 0;
    margin: 0;
    cursor: pointer;
    flex: 1;
    min-width: 0;
    height: 100%;
    /* Prevent focus ring from being clipped on the left */
    box-sizing: border-box;
  }
  .idle-button:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
    border-radius: 999px;
  }

  .hotkey-hint {
    position: absolute;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    font-size: 9px;
    font-weight: 600;
    color: var(--muted);
    background: rgba(12, 8, 22, 0.92);
    border: 1px solid var(--border);
    border-radius: 999px;
    padding: 2px 7px;
    opacity: 0;
    pointer-events: none;
    animation: hint-fade 2s ease-out forwards;
    white-space: nowrap;
    z-index: 2;
  }

  @keyframes pill-in {
    from {
      opacity: 0;
      transform: scale(0.94);
    }
    to {
      opacity: 1;
      transform: none;
    }
  }
  @keyframes breathe {
    0%,
    100% {
      transform: scale(1);
      opacity: 0.9;
      filter: drop-shadow(0 0 0 transparent);
    }
    50% {
      /* Keep scale modest so the mark stays inside the capsule */
      transform: scale(1.04);
      opacity: 1;
      filter: drop-shadow(0 0 3px rgba(var(--accent-rgb), 0.45));
    }
  }
  @keyframes pulse {
    0%,
    100% {
      opacity: 1;
      transform: scale(1);
    }
    50% {
      opacity: 0.55;
      transform: scale(0.82);
    }
  }
  @keyframes ring {
    0% {
      transform: scale(0.85);
      opacity: 0.7;
    }
    100% {
      transform: scale(1.55);
      opacity: 0;
    }
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  @keyframes pop {
    from {
      transform: scale(0.2);
      opacity: 0;
    }
    to {
      transform: scale(1);
      opacity: 1;
    }
  }
  @keyframes listen-glow {
    0%,
    100% {
      box-shadow:
        0 1px 0 rgba(255, 255, 255, 0.08) inset,
        0 0 0 1px rgba(var(--accent-rgb), 0.1),
        0 3px 12px rgba(var(--accent-rgb), 0.3);
    }
    50% {
      box-shadow:
        0 1px 0 rgba(255, 255, 255, 0.1) inset,
        0 0 0 1px rgba(255, 255, 255, 0.16),
        0 5px 16px rgba(var(--accent-rgb), 0.48);
    }
  }
  @keyframes pause-glow {
    0%,
    100% {
      box-shadow:
        0 1px 0 rgba(255, 255, 255, 0.08) inset,
        0 0 0 1px rgba(251, 191, 36, 0.1),
        0 3px 12px rgba(251, 191, 36, 0.22);
    }
    50% {
      box-shadow:
        0 1px 0 rgba(255, 255, 255, 0.1) inset,
        0 0 0 1px rgba(255, 255, 255, 0.16),
        0 5px 16px rgba(251, 191, 36, 0.35);
    }
  }
  @keyframes hint-fade {
    0% {
      opacity: 0;
      transform: translate(-50%, -40%);
    }
    12% {
      opacity: 1;
      transform: translate(-50%, -50%);
    }
    70% {
      opacity: 1;
      transform: translate(-50%, -50%);
    }
    100% {
      opacity: 0;
      transform: translate(-50%, -60%);
    }
  }
</style>
