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
	  import { onMount, onDestroy, tick } from 'svelte';
	  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
	  import { getCurrentWindow } from '@tauri-apps/api/window';
	  import { invoke } from '@tauri-apps/api/core';
	  import BrandMark from './BrandMark.svelte';

  type PillStateValue = 'idle' | 'listening' | 'processing' | 'success' | 'error';

  interface PillStatePayload {
    state: PillStateValue;
    message?: string;
  }

  interface AudioLevelPayload {
    level: number;
  }

  interface PartialTranscriptPayload {
    text: string;
    is_final?: boolean;
  }

  interface PillSizePayload {
    width: number;
    height: number;
  }

  /** Samples along the listening sound-wave path. */
  const WAVE_SAMPLES = 28;
  const WAVE_WIDTH = 132;
  const WAVE_HEIGHT = 30;

  let pillState = $state<PillStateValue>('idle');
  let errorMessage = $state<string | null>(null);
  let audioLevel = $state(0);

  /** Live partial (or final) transcript text. Updated by events + simulation. */
  let transcript = $state<string>('');
  /** Editable copy shown in quick-edit success UI. */
  let editableTranscript = $state<string>('');
  /** Snapshot of the received final (for "Confirm Paste" vs edited). */
  let originalFinal = $state<string>('');

  /** Target level from the backend; displayLevel eases toward it each frame. */
  let targetLevel = 0;
  let displayLevel = 0;
  let waveRaf: number | null = null;

  /** Window size (logical px) broadcast by the backend `pill-size` event. */
  let pillW = $state<number>(200);
  let pillH = $state<number>(48);

  /** Simulation timer for live partials during listening (when no real partials yet). */
  let simTimer: number | null = null;

  /** Smoothed amplitudes that drive the live sound wave. */
  let waveSamples = $state<number[]>(Array(WAVE_SAMPLES).fill(0.08));

  let unlistenLevel: UnlistenFn | null = null;
  let unlistenState: UnlistenFn | null = null;
  let unlistenStyle: UnlistenFn | null = null;
  let unlistenPartial: UnlistenFn | null = null;
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

  /** Canned phrases for realistic-looking live partial simulation. */
  const SIM_PHRASES = [
    'hey there how are you',
    'testing voice input now',
    'quick brown fox jumps',
    'this is a live transcript',
    'hello can you hear me',
    'setting up the new feature',
  ];

  function stopSim() {
    if (simTimer !== null) {
      clearInterval(simTimer);
      simTimer = null;
    }
  }

  /** Start a lightweight progressive reveal so the pill feels alive while listening.
   *  Real final transcript from backend will replace it instantly. */
  function startSim() {
    stopSim();
    transcript = '';
    const base = SIM_PHRASES[Math.floor(Math.random() * SIM_PHRASES.length)];
    const phrase = base + ' ';
    let i = 0;
    simTimer = window.setInterval(() => {
      if (pillState !== 'listening') {
        stopSim();
        return;
      }
      i += 1 + Math.floor(Math.random() * 2);
      const next = phrase.slice(0, Math.min(i, phrase.length));
      // subtle jitter so it feels streaming
      transcript = next + (Math.random() > 0.7 ? '…' : '');
      if (i > phrase.length + 6) {
        // end this sim cycle; a new one could restart but final will arrive
        stopSim();
      }
    }, 135);
  }

  // Drag state
  let dragStartX = 0;
  let dragStartY = 0;
  let isDragging = $state(false);
  let didDrag = $state(false);
  let dragUnlisten: (() => void) | null = null;

  /** Bound ref to the quick-edit textarea for auto-focus. */
  let transcriptTa = $state<HTMLTextAreaElement | null>(null);

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

  /** Auto-grow the textarea height a little for multi-line edits (capped). */
  function autoResizeTa(e?: Event) {
    const ta = (e?.currentTarget as HTMLTextAreaElement) || transcriptTa;
    if (!ta) return;
    ta.style.height = 'auto';
    const h = Math.min(ta.scrollHeight, 68);
    ta.style.height = h + 'px';
  }

  /** Keyboard niceties inside the tiny editor: Enter = paste, Esc = cancel. */
  function onTaKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      // default to edit-and-paste on Enter (user is editing)
      editAndPaste();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      cancelReview();
    }
  }

  async function confirmPaste() {
    // Use the original final (user may not have touched the field yet)
    const text = (originalFinal || editableTranscript || transcript || '').trim();
    await doPaste(text);
  }

  async function editAndPaste() {
    const text = (editableTranscript || originalFinal || transcript || '').trim();
    await doPaste(text);
  }

  async function doPaste(text: string) {
    if (!text) {
      cancelReview();
      return;
    }
    try {
      // Hide the pill FIRST so focus returns to the previously-focused
      // target app. Otherwise Ctrl+V goes into the pill's textarea
      // instead of where the user wants the text.
      try {
        await pillWindow.hide();
      } catch (e) {
        console.warn('pill hide before reinject failed', e);
      }
      // Small delay so the OS settles focus to the prior foreground window
      // before we send the synthetic paste keystroke.
      await new Promise((r) => setTimeout(r, 60));
      await invoke('reinject_text', { text });
      // Instant feedback: clear editor state and drop back to compact idle.
      transcript = '';
      editableTranscript = '';
      originalFinal = '';
      pillState = 'idle';
    } catch (e) {
      console.warn('reinject_text failed from pill', e);
      // Still drop the review so user isn't stuck.
      cancelReview();
    }
  }

  function cancelReview() {
    transcript = '';
    editableTranscript = '';
    originalFinal = '';
    // Respond instantly for polished feel.
    pillState = 'idle';
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

        // During active quick-edit review we ignore the backend's auto-idle
        // timer ONLY while the user is actually editing the textarea (focus
        // is in there) or has unsaved edits. Otherwise we let the idle
        // transition take the pill back to its compact form — leaving it
        // stuck open after a successful paste was the cause of the
        // "pill hangs forever" bug.
        if (pillState === 'success' && next === 'idle') {
          const ta = transcriptTa;
          const taHasFocus = ta && document.activeElement === ta;
          const hasUnsavedEdits =
            editableTranscript.length > 0 &&
            editableTranscript !== originalFinal;
          if (taHasFocus || hasUnsavedEdits) {
            // User is engaged — keep the editor open.
            return;
          }
          // Auto-paste already happened; fall through to clear + idle.
        }

        pillState = next;
        errorMessage = event.payload?.message ?? null;
        if (next !== 'error') {
          errorMessage = null;
        }
        if (next === 'listening') {
          transcript = '';
          editableTranscript = '';
          originalFinal = '';
          startWaveLoop();
          startSim();
        } else {
          stopWaveLoop();
          stopSim();
          if (next === 'success') {
            if (transcript) {
              editableTranscript = transcript;
              originalFinal = transcript;
            }
            // NOTE: We deliberately do NOT auto-focus the textarea here.
            // The backend already auto-pasted the text into the focused
            // target app; if we stole focus back into the pill's textarea,
            // any subsequent Ctrl+V the user typed would paste into the
            // pill instead of the target. The user can click "Edit & Paste"
            // to focus the textarea if they want to correct anything.
            tick().then(() => {
              if (transcriptTa) {
                autoResizeTa();
              }
            });
          }
          if (next === 'idle' || next === 'error') {
            transcript = '';
            editableTranscript = '';
            originalFinal = '';
          }
        }
      });
    } catch (e) {
      console.warn('pill-state listen failed', e);
    }

    // Listen for live partial transcripts (simulated during listen + real final).
    try {
      unlistenPartial = await listen<PartialTranscriptPayload>('partial-transcript', (event) => {
        const t = (event.payload?.text ?? '').trim();
        if (!t) return;
        transcript = t;
        if (event.payload?.is_final) {
          originalFinal = t;
          editableTranscript = t;
          // If we are still listening/processing visually, let success state drive the UI.
        }
      });
    } catch (e) {
      console.warn('partial-transcript listen failed', e);
    }

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
    stopSim();
    if (unlistenLevel) unlistenLevel();
    if (unlistenState) unlistenState();
    if (unlistenStyle) unlistenStyle();
    if (unlistenPartial) unlistenPartial();
    if (unlistenSize) unlistenSize();
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
    {#if transcript}
      <div class="live-text" title={transcript} aria-live="polite">
        <span class="speaking">…</span> {transcript}
      </div>
    {/if}
    <button class="close" onclick={(e) => { e.stopPropagation(); close(); }} title="Hide pill" aria-label="Hide pill">×</button>
  {:else if pillState === 'processing'}
    <div class="icon">
      <span class="spinner" aria-hidden="true"></span>
    </div>
    <div class="label">Processing…</div>
    {#if transcript}
      <div class="live-text dim" title={transcript}>{transcript}</div>
    {/if}
    <button class="close" onclick={(e) => { e.stopPropagation(); close(); }} title="Hide pill" aria-label="Hide pill">×</button>
  {:else if pillState === 'success'}
    <div class="icon">
      <span class="check" aria-hidden="true">✓</span>
    </div>
    <div class="review">
      <textarea
        bind:this={transcriptTa}
        bind:value={editableTranscript}
        class="transcript-ta"
        rows="1"
        placeholder="Your words…"
        oninput={autoResizeTa}
        onkeydown={onTaKeydown}
        aria-label="Edit transcript before pasting"
      ></textarea>
      <div class="quick-actions">
        <button
          class="action confirm"
          onclick={(e) => { e.stopPropagation(); confirmPaste(); }}
          title="Paste the text as shown (or as originally heard)"
        >
          Confirm Paste
        </button>
        <button
          class="action edit"
          onclick={(e) => { e.stopPropagation(); editAndPaste(); }}
          title="Paste the edited version"
        >
          Edit &amp; Paste
        </button>
        <button
          class="action cancel"
          onclick={(e) => { e.stopPropagation(); cancelReview(); }}
          title="Discard, do not paste"
        >
          Cancel
        </button>
      </div>
    </div>
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
    gap: 8px;
    padding: 6px 12px;
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
    flex: 1;
    min-width: 108px;
    height: 34px;
    padding: 3px 8px;
    border-radius: 10px;
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
    gap: 10px;
    padding: 0;
    cursor: pointer;
  }
  .idle-button:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
    border-radius: 999px;
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

  /* --- Live partial transcript (speaking) + quick-edit styles --------------- */

  .live-text {
    font-size: 11px;
    line-height: 1.2;
    color: var(--fg);
    max-width: 168px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    padding: 1px 6px;
    border-radius: 6px;
    background: rgba(0, 0, 0, 0.18);
    border: 1px solid rgba(255, 255, 255, 0.1);
    flex: 1;
    min-width: 0;
  }
  .live-text .speaking {
    color: var(--accent);
    font-weight: 600;
    display: inline-block;
    width: 14px;
    text-align: left;
    opacity: 0.9;
    animation: speak-pulse 1.1s ease-in-out infinite;
  }
  .live-text.dim {
    opacity: 0.75;
    font-size: 10.5px;
  }

  @keyframes speak-pulse {
    0%, 100% { opacity: 0.6; }
    50% { opacity: 1; }
  }

  .review {
    display: flex;
    flex-direction: column;
    gap: 4px;
    flex: 1;
    min-width: 150px;
    max-width: 260px;
  }

  .transcript-ta {
    appearance: none;
    font: inherit;
    font-size: 12px;
    font-weight: 500;
    line-height: 1.25;
    padding: 4px 7px;
    border-radius: 7px;
    border: 1px solid rgba(255, 255, 255, 0.22);
    background: rgba(255, 255, 255, 0.06);
    color: var(--fg);
    resize: none;
    overflow: hidden;
    min-height: 18px;
    max-height: 68px;
    width: 100%;
    box-sizing: border-box;
    box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.06);
    transition: border-color 120ms ease, box-shadow 120ms ease;
  }
  .transcript-ta:focus {
    outline: none;
    border-color: var(--accent);
    box-shadow: 0 0 0 2px rgba(196, 181, 253, 0.22);
    background: rgba(255, 255, 255, 0.09);
  }
  .transcript-ta::placeholder {
    color: var(--muted);
    opacity: 0.65;
  }

  .quick-actions {
    display: flex;
    align-items: center;
    gap: 3px;
    flex-wrap: nowrap;
  }

  .action {
    appearance: none;
    background: rgba(255, 255, 255, 0.07);
    color: var(--fg);
    border: 1px solid rgba(255, 255, 255, 0.18);
    font-size: 9.5px;
    line-height: 1;
    padding: 2px 7px;
    border-radius: 999px;
    cursor: pointer;
    white-space: nowrap;
    transition:
      background 100ms ease,
      color 100ms ease,
      border-color 100ms ease,
      transform 80ms ease;
    font-weight: 500;
    letter-spacing: 0.01em;
  }
  .action:hover {
    background: rgba(255, 255, 255, 0.14);
    border-color: rgba(255, 255, 255, 0.3);
  }
  .action:active {
    transform: translateY(0.5px);
  }
  .action.confirm {
    background: rgba(92, 255, 156, 0.14);
    border-color: rgba(92, 255, 156, 0.35);
    color: #d1f7df;
  }
  .action.confirm:hover {
    background: rgba(92, 255, 156, 0.24);
  }
  .action.edit {
    background: rgba(196, 181, 253, 0.14);
    border-color: rgba(196, 181, 253, 0.35);
  }
  .action.edit:hover {
    background: rgba(196, 181, 253, 0.22);
  }
  .action.cancel {
    color: var(--muted);
    border-color: rgba(255, 255, 255, 0.12);
    background: transparent;
    font-size: 9px;
  }
  .action.cancel:hover {
    color: var(--danger);
    background: rgba(255, 92, 124, 0.1);
    border-color: rgba(255, 92, 124, 0.3);
  }

  /* When review is visible, give the pill a touch more room without exploding size */
  .pill.success {
    padding: 8px 12px;
    height: auto;
    min-height: 56px;
  }
  .pill.success .review {
    margin-right: 2px;
  }
</style>
