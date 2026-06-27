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
    listHistory,
    downloadModel,
    type ModelStatus,
    type RecordingMode,
    type HistoryEntry,
  } from './api';
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
    busy = true;
    downloadMsg = 'Downloading…';
    try {
      await downloadModel('whisper-tiny-en');
      downloadMsg = null;
      await refresh();
    } catch {
      downloadMsg = 'Download failed — check your connection.';
    } finally {
      busy = false;
    }
  }

  onMount(() => {
    refresh();
    let unlistenLevel: UnlistenFn | null = null;
    let unlistenState: UnlistenFn | null = null;
    let decay: number | null = null;

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

    decay = window.setInterval(() => {
      if (audioLevel > 0) applyLevel(Math.max(0, audioLevel - 0.08));
    }, 90);

    return () => {
      if (unlistenLevel) unlistenLevel();
      if (unlistenState) unlistenState();
      if (decay !== null) window.clearInterval(decay);
    };
  });
</script>

<div class="home">
  <header class="hero">
    <span class="hero-mark" aria-hidden="true"><AppIcon size={52} /></span>
    <div>
      <h1>DevWhisp</h1>
      <p class="tagline">Talk instead of type — local, offline, on your CPU.</p>
    </div>
  </header>

  <!-- Live status hero -->
  <section class="status-card" data-state={pillState}>
    <div class="status-top">
      <span class="status-dot" class:live={pillState === 'listening'}></span>
      <span class="status-label">{statusLabel}</span>
    </div>
    <div class="status-bars" aria-hidden="true">
      {#each bars as mag, i (i)}
        <span class="sbar" style:--mag={mag.toFixed(3)}></span>
      {/each}
    </div>
    <div class="status-foot">
      <span>
        {#if mode === 'toggle'}Tap{:else if mode === 'vad'}Speak (auto){:else}Hold{/if}
        {#each hotkeyKeys as k, i (k + i)}<kbd>{k}</kbd>{#if i < hotkeyKeys.length - 1}<span class="plus">+</span>{/if}{/each}
        to talk
      </span>
      <span class="mode-chip">{mode === 'toggle' ? 'Toggle' : mode === 'vad' ? 'VAD' : 'Push-to-talk'}</span>
    </div>
  </section>

  <!-- How it works -->
  <section class="steps">
    <div class="step"><span class="num">1</span><div><strong>Hold the hotkey</strong><p>Anywhere — any app, any text field.</p></div></div>
    <div class="step"><span class="num">2</span><div><strong>Speak naturally</strong><p>The pill shows it's listening.</p></div></div>
    <div class="step"><span class="num">3</span><div><strong>Release</strong><p>Your words paste where the cursor is.</p></div></div>
  </section>

  <!-- At-a-glance cards -->
  <div class="grid">
    <section class="mini">
      <div class="mini-label">Model</div>
      {#if modelStatus?.ready}
        <div class="mini-value ok">● {modelStatus.variant}</div>
        <div class="mini-sub">{modelStatus.fileSizeMb} MB · downloaded · CPU</div>
      {:else if modelStatus}
        <div class="mini-value warn">incomplete</div>
        <button class="link" onclick={downloadModelNow} disabled={busy}>Re-download</button>
        {#if downloadMsg}<div class="mini-sub">{downloadMsg}</div>{/if}
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
  .home { display: flex; flex-direction: column; gap: 18px; }

  .hero { display: flex; align-items: center; gap: 16px; padding: 8px 0 4px; }
  .hero-mark { display: inline-flex; flex-shrink: 0; filter: drop-shadow(0 4px 16px rgba(124, 58, 237, 0.45)); }
  h1 {
    margin: 0; font-size: 26px; font-weight: 700; letter-spacing: -0.02em;
    background: var(--brand-gradient); -webkit-background-clip: text; background-clip: text;
    -webkit-text-fill-color: transparent;
  }
  .tagline { margin: 3px 0 0; color: var(--muted); font-size: 13px; }

  /* Status hero */
  .status-card {
    position: relative;
    overflow: hidden;
    border: 1px solid var(--border);
    border-radius: var(--r-xl);
    padding: 22px 24px;
    background:
      radial-gradient(120% 120% at 0% 0%, rgba(124, 58, 237, 0.18), transparent 55%),
      linear-gradient(180deg, var(--card-2), var(--card));
    box-shadow: var(--shadow-2);
    transition: box-shadow 200ms ease, border-color 200ms ease;
  }
  .status-card[data-state='listening'] { border-color: rgba(196, 181, 253, 0.5); box-shadow: var(--shadow-accent); }
  .status-card[data-state='success'] { border-color: rgba(92, 255, 156, 0.4); }
  .status-card[data-state='error'] { border-color: rgba(255, 92, 124, 0.5); }

  .status-top { display: flex; align-items: center; gap: 10px; margin-bottom: 16px; }
  .status-dot { width: 10px; height: 10px; border-radius: 50%; background: var(--muted); }
  .status-dot.live { background: var(--danger); box-shadow: 0 0 10px var(--danger); animation: pulse 1.2s ease-in-out infinite; }
  @keyframes pulse { 0%,100% { opacity: 1; transform: scale(1); } 50% { opacity: 0.5; transform: scale(0.8); } }
  .status-label { font-size: 18px; font-weight: 600; color: var(--text); letter-spacing: -0.01em; }

  .status-bars { display: flex; align-items: flex-end; gap: 6px; height: 56px; margin-bottom: 18px; }
  .sbar {
    flex: 1; border-radius: 4px; align-self: stretch;
    background: rgba(196, 181, 253, 0.16);
    transform-origin: center; transform: scaleY(calc(0.12 + var(--mag, 0) * 0.88));
    transition: transform 70ms ease-out;
  }
  .status-card[data-state='listening'] .sbar {
    background: linear-gradient(180deg, var(--accent), var(--accent-deep));
    box-shadow: 0 0 8px rgba(196, 181, 253, 0.35);
  }

  .status-foot { display: flex; align-items: center; justify-content: space-between; gap: 12px; color: var(--muted); font-size: 13px; flex-wrap: wrap; }
  .status-foot kbd {
    background: var(--bg-elevated); border: 1px solid var(--border-strong); border-bottom-width: 2px;
    border-radius: 5px; padding: 2px 7px; margin: 0 2px;
    font-family: ui-monospace, "JetBrains Mono", monospace; font-size: 11px; color: var(--text);
  }
  .mode-chip {
    font-size: 11px; font-weight: 600; text-transform: uppercase; letter-spacing: 0.08em;
    padding: 4px 10px; border-radius: 999px; background: var(--accent-soft); color: var(--accent);
  }

  /* Steps */
  .steps { display: grid; grid-template-columns: repeat(3, 1fr); gap: 10px; }
  .step {
    display: flex; gap: 10px; align-items: flex-start;
    background: var(--card); border: 1px solid var(--border); border-radius: var(--r-md); padding: 14px;
  }
  .step .num {
    flex-shrink: 0; width: 22px; height: 22px; border-radius: 50%;
    display: inline-flex; align-items: center; justify-content: center;
    font-size: 12px; font-weight: 700; color: #fff; background: var(--brand-gradient);
  }
  .step strong { font-size: 13px; color: var(--text); }
  .step p { margin: 2px 0 0; font-size: 11.5px; color: var(--muted); line-height: 1.4; }

  /* Mini cards */
  .grid { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; }
  .mini { background: var(--card); border: 1px solid var(--border); border-radius: var(--r-md); padding: 16px 18px; }
  .mini-label { font-size: 11px; font-weight: 700; text-transform: uppercase; letter-spacing: 0.1em; color: var(--accent); margin-bottom: 8px; }
  .mini-value { font-size: 22px; font-weight: 700; color: var(--text); letter-spacing: -0.01em; }
  .mini-value.ok { color: var(--ok); font-size: 16px; }
  .mini-value.warn { color: var(--warn); font-size: 16px; }
  .mini-value.muted { color: var(--muted); font-size: 16px; }
  .mini-sub { margin-top: 4px; font-size: 11px; color: var(--muted); font-family: ui-monospace, "JetBrains Mono", monospace; }
  .link { background: none; border: none; color: var(--accent); font-size: 12px; cursor: pointer; padding: 4px 0 0; font-family: inherit; }
  .link:hover:not(:disabled) { text-decoration: underline; }
  .link:disabled { opacity: 0.5; cursor: not-allowed; }
</style>
