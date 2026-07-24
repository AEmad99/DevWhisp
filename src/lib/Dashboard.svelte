<script lang="ts">
  /**
   * Home view — the polished landing surface. Shows the live listening state,
   * the hotkey, a live audio meter, a quick how-it-works, and at-a-glance
   * model + activity status. (Replaces the old M1 IPC smoke-test surface.)
   */
  import { onMount } from 'svelte';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import {
    getModelStatus,
    getRecordingMode,
    getHotkey,
    getAccelerationInfo,
    listHistory,
    downloadModel,
    RECOMMENDED_MODEL,
    type ModelStatus,
    type RecordingMode,
    type HistoryEntry,
    type AccelerationInfo,
  } from './api';
  import { downloadStore } from './downloadStore';
  import AppIcon from './AppIcon.svelte';

  type PillState = 'idle' | 'listening' | 'processing' | 'success' | 'error';

  let pillState = $state<PillState>('idle');
  let audioLevel = $state(0);
  let mode = $state<RecordingMode>('push-to-talk');
  /** Canonical display form of the active hotkey, e.g. "Ctrl+Shift+Space" or "F8". */
  let hotkeyLabel = $state('Ctrl+Shift+Space');
  /** Same string split into segments for rendering each segment as <kbd>. */
  let hotkeyKeys = $derived(hotkeyLabel.split('+').map((s) => s.trim()).filter(Boolean));
  let modelStatus = $state<ModelStatus | null>(null);
  let accelInfo = $state<AccelerationInfo | null>(null);
  let entries = $state<HistoryEntry[]>([]);
  let busy = $state(false);
  let downloadMsg = $state<string | null>(null);

  const NUM_BARS = 7;
  let bars = $state<number[]>(Array(NUM_BARS).fill(0));

  const statusLabel = $derived(
    pillState === 'listening'
      ? 'Listening…'
      : pillState === 'processing'
        ? 'Transcribing…'
        : pillState === 'success'
          ? 'Pasted ✓'
          : pillState === 'error'
            ? 'Something went wrong'
            : 'Ready to listen',
  );

  function startOfToday(now = Date.now()): number {
    const d = new Date(now);
    d.setHours(0, 0, 0, 0);
    return d.getTime();
  }
  function words(t: string): number {
    return t.trim().split(/\s+/).filter(Boolean).length;
  }
  const todayStats = $derived.by(() => {
    const cutoff = startOfToday();
    const today = entries.filter((e) => e.created_at >= cutoff);
    const w = today.reduce((s, e) => s + words(e.text), 0);
    return { count: today.length, words: w, total: entries.length };
  });

  function applyLevel(level: number) {
    audioLevel = level;
    const seed = performance.now();
    const next: number[] = [];
    for (let i = 0; i < NUM_BARS; i++) {
      const dist = Math.abs(i - (NUM_BARS - 1) / 2) / ((NUM_BARS - 1) / 2);
      const bias = 1 - dist * 0.45;
      const jitter = ((Math.sin(seed * 0.004 + i * 1.7) + 1) / 2) * 0.2;
      const v = Math.min(1, level * bias * (1.1 + jitter));
      next.push((bars[i] ?? 0) * 0.5 + v * 0.5);
    }
    bars = next;
  }

  async function refresh() {
    try {
      modelStatus = await getModelStatus();
    } catch {
      /* ignore */
    }
    try {
      accelInfo = await getAccelerationInfo();
    } catch {
      /* ignore */
    }
    try {
      mode = await getRecordingMode();
    } catch {
      /* ignore */
    }
    try {
      hotkeyLabel = await getHotkey();
    } catch {
      /* ignore */
    }
    try {
      entries = await listHistory(200, 0);
    } catch {
      /* ignore */
    }
  }

  async function downloadModelNow() {
    try {
      await downloadStore.download(RECOMMENDED_MODEL);
      await refresh();
    } catch {
      /* Handled by downloadStore */
    }
  }

  onMount(() => {
    refresh();
    let unlistenLevel: UnlistenFn | null = null;
    let unlistenState: UnlistenFn | null = null;

    // Guarded: outside a Tauri runtime `listen` throws synchronously; the
    // live meter/state are optional, so never let that take down the view.
    try {
      listen<{ level: number }>('audio-level', (e) => {
        const lvl = Math.max(0, Math.min(1, Number(e.payload?.level ?? 0)));
        applyLevel(lvl);
      })
        .then((fn) => (unlistenLevel = fn))
        .catch(() => {});

      listen<{ state: PillState }>('pill-state', (e) => {
        pillState = e.payload?.state ?? 'idle';
        if (pillState === 'success') setTimeout(refresh, 400);
      })
        .then((fn) => (unlistenState = fn))
        .catch(() => {});
    } catch {
      /* no Tauri bridge — fine */
    }

    let decayRaf: number | null = null;
    const decayStep = () => {
      if (audioLevel > 0) {
        applyLevel(Math.max(0, audioLevel - 0.02));
      }
      decayRaf = requestAnimationFrame(decayStep);
    };
    decayRaf = requestAnimationFrame(decayStep);

    return () => {
      if (unlistenLevel) unlistenLevel();
      if (unlistenState) unlistenState();
      if (decayRaf !== null) cancelAnimationFrame(decayRaf);
    };
  });
</script>

<div class="home">
  <header class="hero">
    <span class="hero-mark" aria-hidden="true"><AppIcon size={40} /></span>
    <div>
      <h1>DevWhisp</h1>
      <p class="tagline">Voice → text, local &amp; offline.</p>
    </div>
  </header>

  <section class="status-card" data-state={pillState}>
    <div class="status-top">
      <span class="status-dot" class:live={pillState === 'listening'}></span>
      <span class="status-label">{statusLabel}</span>
      <span class="mode-chip">{mode === 'toggle' ? 'Toggle' : mode === 'vad' ? 'VAD' : 'PTT'}</span>
    </div>
    <div class="status-bars" aria-hidden="true">
      {#each bars as mag, i (i)}
        <span class="sbar" style:--mag={mag.toFixed(3)}></span>
      {/each}
    </div>
    <div class="status-foot">
      <span>
        {#if mode === 'toggle'}Tap{:else if mode === 'vad'}Speak{:else}Hold{/if}
        {#each hotkeyKeys as k, i (k + i)}<kbd>{k}</kbd>{#if i < hotkeyKeys.length - 1}<span class="plus">+</span>{/if}{/each}
      </span>
    </div>
  </section>

  <section class="steps">
    <div class="step"><span class="num">1</span><div><strong>Hold hotkey</strong><p>Any app, any field.</p></div></div>
    <div class="step"><span class="num">2</span><div><strong>Speak</strong><p>Pill shows listening.</p></div></div>
    <div class="step"><span class="num">3</span><div><strong>Release</strong><p>Text pastes at cursor.</p></div></div>
  </section>

  <div class="grid">
    <section class="mini">
      <div class="mini-label">Model</div>
      {#if downloadStore.isDownloading}
        <div class="mini-value warn">Downloading… {downloadStore.pct.toFixed(0)}%</div>
        <div class="mini-sub">{downloadStore.downloadedMB} / {downloadStore.totalMB} MB</div>
      {:else if modelStatus?.ready}
        <div class="mini-value ok">● {modelStatus.displayName || modelStatus.variant}</div>
        <div class="mini-sub">{modelStatus.fileSizeMb} MB · {accelInfo?.inUse ?? 'CPU'}</div>
      {:else if modelStatus}
        <div class="mini-value warn">incomplete</div>
        <button class="link" onclick={downloadModelNow} disabled={downloadStore.isDownloading}>Re-download</button>
        {#if downloadStore.error}<div class="mini-sub danger">{downloadStore.error}</div>{/if}
      {:else}
        <div class="mini-value muted">checking…</div>
      {/if}
    </section>

    <section class="mini">
      <div class="mini-label">Today</div>
      <div class="mini-value">{todayStats.count}</div>
      <div class="mini-sub">{todayStats.words.toLocaleString()} words · {todayStats.total} total</div>
    </section>
  </div>
</div>

<style>
  .home { display: flex; flex-direction: column; gap: 12px; }

  .hero { display: flex; align-items: center; gap: 12px; padding: 2px 0; }
  .hero-mark { display: inline-flex; flex-shrink: 0; }
  h1 {
    margin: 0; font-size: 20px; font-weight: 700; letter-spacing: -0.03em;
    color: var(--text);
  }
  .tagline { margin: 2px 0 0; color: var(--muted); font-size: 12px; }

  .status-card {
    position: relative;
    overflow: hidden;
    border: 1px solid var(--border);
    border-radius: var(--r-lg);
    padding: 14px 16px;
    background: var(--card);
    box-shadow: var(--shadow-1);
    transition: border-color 180ms ease;
  }
  .status-card[data-state='listening'] { border-color: var(--accent); }
  .status-card[data-state='success'] { border-color: rgba(52, 211, 153, 0.4); }
  .status-card[data-state='error'] { border-color: rgba(248, 113, 113, 0.4); }

  .status-top { display: flex; align-items: center; gap: 8px; margin-bottom: 10px; }
  .status-dot { width: 8px; height: 8px; border-radius: 50%; background: var(--muted); flex-shrink: 0; }
  .status-dot.live { background: var(--danger); animation: pulse 1.2s ease-in-out infinite; }
  @keyframes pulse { 0%,100% { opacity: 1; transform: scale(1); } 50% { opacity: 0.5; transform: scale(0.8); } }
  .status-label { font-size: 15px; font-weight: 600; color: var(--text); letter-spacing: -0.01em; flex: 1; min-width: 0; }

  .status-bars { display: flex; align-items: center; gap: 4px; height: 36px; margin-bottom: 10px; padding: 4px 6px; border-radius: 8px; background: rgba(0,0,0,0.3); border: 1px solid rgba(255,255,255,0.06); }
  .sbar {
    flex: 1; border-radius: 3px; height: 100%;
    background: rgba(255, 255, 255, 0.12);
    transform-origin: center; transform: scaleY(calc(0.12 + var(--mag, 0) * 0.88));
    transition: transform 70ms ease-out;
  }
  .status-card[data-state='listening'] .sbar {
    background: var(--accent);
  }

  .status-foot { display: flex; align-items: center; gap: 8px; color: var(--muted); font-size: 12px; flex-wrap: wrap; }
  .status-foot kbd {
    background: var(--bg-elevated); border: 1px solid var(--border-strong);
    border-radius: 4px; padding: 1px 6px; margin: 0 1px;
    font-family: ui-monospace, "JetBrains Mono", monospace; font-size: 10.5px; color: var(--text);
  }
  .mode-chip {
    font-size: 10px; font-weight: 650; text-transform: uppercase; letter-spacing: 0.06em;
    padding: 3px 8px; border-radius: 999px; background: var(--accent-soft); color: var(--accent); flex-shrink: 0;
  }

  .steps { display: grid; grid-template-columns: repeat(3, 1fr); gap: 8px; }
  .step {
    display: flex; gap: 8px; align-items: flex-start;
    background: var(--card); border: 1px solid var(--border); border-radius: var(--r-md); padding: 10px;
  }
  .step .num {
    flex-shrink: 0; width: 18px; height: 18px; border-radius: 50%;
    display: inline-flex; align-items: center; justify-content: center;
    font-size: 10px; font-weight: 700; color: #fff; background: var(--accent-deep);
  }
  .step strong { font-size: 12px; color: var(--text); display: block; }
  .step p { margin: 1px 0 0; font-size: 11px; color: var(--muted); line-height: 1.35; }

  .grid { display: grid; grid-template-columns: 1fr 1fr; gap: 8px; }
  .mini { background: var(--card); border: 1px solid var(--border); border-radius: var(--r-md); padding: 12px 14px; }
  .mini-label { font-size: 10px; font-weight: 700; text-transform: uppercase; letter-spacing: 0.1em; color: var(--accent); margin-bottom: 6px; }
  .mini-value { font-size: 18px; font-weight: 700; color: var(--text); letter-spacing: -0.02em; }
  .mini-value.ok { color: var(--ok); font-size: 14px; }
  .mini-value.warn { color: var(--warn); font-size: 14px; }
  .mini-value.muted { color: var(--muted); font-size: 14px; }
  .mini-sub { margin-top: 3px; font-size: 10.5px; color: var(--muted); font-family: ui-monospace, "JetBrains Mono", monospace; }
  .link { background: none; border: none; color: var(--accent); font-size: 11.5px; cursor: pointer; padding: 3px 0 0; font-family: inherit; }
  .link:hover:not(:disabled) { text-decoration: underline; }
  .link:disabled { opacity: 0.5; cursor: not-allowed; }

  @media (max-width: 520px) {
    .steps { grid-template-columns: 1fr; }
    .grid { grid-template-columns: 1fr; }
  }
</style>
