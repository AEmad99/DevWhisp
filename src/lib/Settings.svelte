<script lang="ts">
  /**
   * Settings view — all preferences. Refined sectioned layout with a sticky
   * in-page nav. Everything saves on change (no Save button).
   *
   * Storage strategy:
   *   - Cosmetic/UI that persists but has no backend consequence → localStorage.
   *   - Backend-consumed (dictionary, recording mode, autostart) → IPC / plugin.
   */

  import { onMount } from 'svelte';
  import { emit } from '@tauri-apps/api/event';
  import { invoke } from '@tauri-apps/api/core';
  import { enable, disable, isEnabled } from '@tauri-apps/plugin-autostart';
  import AppIcon from './AppIcon.svelte';
  import { downloadStore } from './downloadStore';
  import {
    getAppInfo,
    getModelStatus,
    downloadModel,
    setActiveModel,
    listModelStatuses,
    getDictionary,
    addDictionaryEntry,
    removeDictionaryEntry,
    getRecordingMode,
    setRecordingMode,
    getVadSilenceMs,
    setVadSilenceMs,
    getFormatOptions,
    setFormatOptions,
    getAccelerationInfo,
    setAccelerationMode,
    listAudioDevices,
    getSelectedAudioDevice,
    setSelectedAudioDevice,
    getHotkey,
    setHotkey,
    listPredefinedHotkeys,
    type PredefinedHotkey,
    getPillSize,
    setPillSize,
    setPillPositionPreset,
    type PillPositionPreset,
    getHistoryRetentionDays,
    setHistoryRetentionDays,
    type AppInfo,
    type ModelStatus,
    type DictEntry,
    type RecordingMode,
    type AccelerationInfo,
    type IpcError,
    formatIpcError,
  } from './api';

  // ---- localStorage helpers ---------------------------------------------
  function lsGet<T>(key: string, fallback: T): T {
    try {
      const raw = window.localStorage.getItem(key);
      if (raw === null) return fallback;
      return JSON.parse(raw) as T;
    } catch {
      return fallback;
    }
  }
  function lsSet<T>(key: string, value: T): void {
    try {
      window.localStorage.setItem(key, JSON.stringify(value));
    } catch {
      /* best-effort */
    }
  }

  const LS_KEYS = {
    autoCapitalize: 'devwhisp.settings.autoCapitalize',
    appendSpace: 'devwhisp.settings.appendSpace',
    showPillOnStartup: 'devwhisp.settings.showPillOnStartup',
    pillStyle: 'devwhisp.pill.style',
    accent: 'devwhisp.settings.accent',
    fontScale: 'devwhisp.settings.fontScale',
  } as const;

  type PillStyle = { bgAlpha: number; noHalo: boolean; compact: boolean };
  const PILL_STYLE_DEFAULT: PillStyle = { bgAlpha: 0.55, noHalo: false, compact: false };

  /** Accent color presets. Keys match the CSS palette names in app.css. */
  const ACCENTS: { id: string; label: string; color: string }[] = [
    { id: 'violet', label: 'Violet', color: '#7c3aed' },
    { id: 'blue', label: 'Blue', color: '#3b82f6' },
    { id: 'cyan', label: 'Cyan', color: '#06b6d4' },
    { id: 'green', label: 'Green', color: '#10b981' },
    { id: 'pink', label: 'Pink', color: '#ec4899' },
    { id: 'orange', label: 'Orange', color: '#f97316' },
    { id: 'red', label: 'Red', color: '#ef4444' },
  ];

  /** Pill position presets — must match the Rust enum's `kebab-case` repr. */
  const POS_PRESETS: { id: PillPositionPreset; label: string }[] = [
    { id: 'top-left', label: 'Top Left' },
    { id: 'top-right', label: 'Top Right' },
    { id: 'center', label: 'Center' },
    { id: 'bottom-left', label: 'Bottom Left' },
    { id: 'bottom-right', label: 'Bottom Right' },
  ];

  /** Bounds mirrored from the Rust side (`MIN_WIDTH` / `MAX_WIDTH` in pill_window.rs). */
  const PILL_W_MIN = 160;
  const PILL_W_MAX = 360;
  const PILL_H_MIN = 36;
  const PILL_H_MAX = 80;

  // ---- Section nav ------------------------------------------------------
  const SECTIONS = [
    { id: 'general', label: 'General' },
    { id: 'recording', label: 'Recording' },
    { id: 'performance', label: 'Performance' },
    { id: 'models', label: 'Models' },
    { id: 'appearance', label: 'Appearance' },
    { id: 'text', label: 'Text' },
    { id: 'about', label: 'About' },
  ];
  function scrollTo(id: string) {
    document.getElementById(`sec-${id}`)?.scrollIntoView({ behavior: 'smooth', block: 'start' });
  }

  // ---- Reactive state ---------------------------------------------------
  let autoCapitalize = $state(true);
  let appendSpace = $state(true);
  let showPillOnStartup = $state(true);

  let autostartOn = $state(false);

  // Acceleration (Auto/CPU/GPU + live status from probe)
  let accelMode = $state<'auto' | 'cpu' | 'gpu'>('auto');
  let accelInfo = $state<AccelerationInfo | null>(null);
  let accelBusy = $state(false);
  let autostartBusy = $state(false);
  let autostartError = $state<string | null>(null);

  // Audio devices (top-tier: selection + future wiring to capture)
  let audioDevices = $state<string[]>([]);
  let selectedDevice = $state('Default');
  let audioBusy = $state(false);

  let recordingMode = $state<RecordingMode>('push-to-talk');
  let silenceMs = $state(600);

  // History auto-prune window, in days. `null` = absent (fresh install) → the
  // UI shows the default (2 days). `0` = "Never" (disabled). We keep a
  // separate `retentionLoaded` flag so the segmented control doesn't flash the
  // default before the real value arrives.
  const RETENTION_OPTIONS = [
    { days: 0, label: 'Never' },
    { days: 1, label: '1 day' },
    { days: 2, label: '2 days' },
    { days: 7, label: '7 days' },
    { days: 30, label: '30 days' },
  ] as const;
  const DEFAULT_RETENTION_DAYS = 2;
  let historyRetentionDays = $state<number>(DEFAULT_RETENTION_DAYS);
  let retentionLoaded = $state(false);
  let retentionError = $state<string | null>(null);

  // Hotkey (rebindable from this view). The user picks one from a
  // predefined list — free-form text input was removed in 0.1.3 because
  // the underlying `parse_key` had a bug that mapped every single-char
  // key to `KeyA` regardless of input, so users could not rebind at all.
  let currentHotkey = $state('Ctrl+Shift+Space');
  let predefinedHotkeys = $state<PredefinedHotkey[]>([]);
  let hotkeySaving = $state<string | null>(null); // spec currently being saved, or null
  let hotkeyError = $state<string | null>(null);

  // Pill appearance
  let pillBgAlpha = $state(PILL_STYLE_DEFAULT.bgAlpha);
  let pillNoHalo = $state(PILL_STYLE_DEFAULT.noHalo);
  let pillCompact = $state(PILL_STYLE_DEFAULT.compact);

  // Pill size + position
  let pillWidth = $state(196);
  let pillHeight = $state(48);
  let pillBusy = $state(false);

  // Accent color + font scale
  let accent = $state('violet');
  let fontScalePct = $state(100); // stored as integer percent for simpler UI

  let appInfo = $state<AppInfo | null>(null);
  let modelStatus = $state<ModelStatus | null>(null);
  let modelStatuses = $state<ModelStatus[]>([]);
  let modelError = $state<string | null>(null);
  let modelBusy = $state(false);

  let dict = $state<DictEntry[]>([]);
  let dictError = $state<string | null>(null);
  let dictBusy = $state(false);
  let newFrom = $state('');
  let newTo = $state('');

  let canAddDict = $derived(
    newFrom.trim().length > 0 &&
      newTo.length > 0 &&
      newFrom.trim().toLowerCase() !== newTo.trim().toLowerCase() &&
      !dictBusy,
  );

  function clamp(v: number, min: number, max: number) {
    return Math.max(min, Math.min(max, v));
  }

  onMount(() => {
    autoCapitalize = lsGet<boolean>(LS_KEYS.autoCapitalize, true);
    appendSpace = lsGet<boolean>(LS_KEYS.appendSpace, true);
    showPillOnStartup = lsGet<boolean>(LS_KEYS.showPillOnStartup, true);

    const savedPill = lsGet<PillStyle | null>(LS_KEYS.pillStyle, null);
    if (savedPill) {
      pillBgAlpha = clamp(savedPill.bgAlpha, 0.08, 0.9);
      pillNoHalo = savedPill.noHalo === true;
      pillCompact = savedPill.compact === true;
    }

    // Accent + font scale — pure localStorage, applied to <html> on boot
    // (main.ts / Pill.main.ts) and updated live below.
    accent = lsGet<string>(LS_KEYS.accent, 'violet');
    document.documentElement.setAttribute('data-accent', accent);
    fontScalePct = clamp(lsGet<number>(LS_KEYS.fontScale, 100), 80, 125);
    document.documentElement.style.setProperty('--font-scale', String(fontScalePct / 100));

    getAppInfo().then((i) => (appInfo = i)).catch(() => {});
    isEnabled().then((v) => (autostartOn = v)).catch(() => {});
    getRecordingMode().then((m) => {
      recordingMode = m;
      loadSilence();
    }).catch(() => {});
    // Formatting options are backend-owned (the hotkey path reads them), so
    // the persisted values are authoritative over the localStorage hint.
    getFormatOptions()
      .then((o) => {
        autoCapitalize = o.autoCapitalize;
        appendSpace = o.appendSpace;
      })
      .catch(() => {});
    refreshModel();
    refreshDict();
    getAccelerationInfo()
      .then((info) => {
        accelInfo = info;
        accelMode = (info.mode === 'cpu' || info.mode === 'gpu') ? info.mode : 'auto';
      })
      .catch(() => {});

    refreshAudioDevices();
    getSelectedAudioDevice().then((d) => { if (d) selectedDevice = d; }).catch(() => {});

    // Hotkey + pill size — fetched from backend so the UI shows the actual
    // currently-registered values, not a stale localStorage cache.
    getHotkey()
      .then((h) => {
        currentHotkey = h;
      })
      .catch(() => {});
    // Predefined hotkey list — fetched once and used to render the picker.
    listPredefinedHotkeys()
      .then((list) => {
        predefinedHotkeys = list;
      })
      .catch((e) => {
        console.warn('listPredefinedHotkeys failed', e);
      });
    getPillSize()
      .then(([w, h]) => {
        pillWidth = clamp(Math.round(w), PILL_W_MIN, PILL_W_MAX);
        pillHeight = clamp(Math.round(h), PILL_H_MIN, PILL_H_MAX);
      })
      .catch(() => {});

    // History retention: `null` means the key is absent (fresh install), so we
    // surface the default. `0` is a real value ("Never").
    getHistoryRetentionDays()
      .then((d) => {
        historyRetentionDays = d === null ? DEFAULT_RETENTION_DAYS : d;
        retentionLoaded = true;
      })
      .catch(() => {
        historyRetentionDays = DEFAULT_RETENTION_DAYS;
        retentionLoaded = true;
      });
  });

  function persistPillStyle() {
    const style: PillStyle = {
      bgAlpha: clamp(pillBgAlpha, 0.08, 0.9),
      noHalo: pillNoHalo,
      compact: pillCompact,
    };
    lsSet(LS_KEYS.pillStyle, style);
    // The pill lives in a SEPARATE window, so broadcast via a Tauri global
    // emit (a same-window DOM event would never reach it). localStorage above
    // persists across restarts; this makes the change apply live.
    void emit('pill-style', style).catch(() => {});
  }

  function onPillBgAlphaInput(event: Event) {
    const target = event.currentTarget as HTMLInputElement;
    pillBgAlpha = clamp(Number(target.value) / 100, 0.08, 0.9);
    persistPillStyle();
  }
  function onPillNoHaloChange(value: boolean) {
    pillNoHalo = value;
    persistPillStyle();
  }
  function onPillCompactChange(value: boolean) {
    pillCompact = value;
    persistPillStyle();
  }

  function persistFormat() {
    lsSet(LS_KEYS.autoCapitalize, autoCapitalize);
    lsSet(LS_KEYS.appendSpace, appendSpace);
    // Persist to the backend so the hotkey/tray transcription paths pick it up.
    void setFormatOptions(autoCapitalize, appendSpace).catch(() => {});
  }
  function onAutoCapitalizeChange(value: boolean) {
    autoCapitalize = value;
    persistFormat();
  }
  function onAppendSpaceChange(value: boolean) {
    appendSpace = value;
    persistFormat();
  }
  function onShowPillChange(value: boolean) {
    showPillOnStartup = value;
    lsSet(LS_KEYS.showPillOnStartup, value);
    // Apply live: show or hide the floating pill window now.
    void invoke(value ? 'show_pill' : 'hide_pill').catch(() => {});
  }

  async function onAutostartChange(value: boolean) {
    autostartBusy = true;
    autostartError = null;
    try {
      if (value) await enable();
      else await disable();
      autostartOn = await isEnabled();
    } catch (e) {
      autostartError = e instanceof Error ? e.message : String(e);
      // Re-sync to the real state on failure.
      try {
        autostartOn = await isEnabled();
      } catch {
        autostartOn = !value;
      }
    } finally {
      autostartBusy = false;
    }
  }

  async function onModeChange(mode: RecordingMode) {
    if (mode === recordingMode) return;
    const prev = recordingMode;
    recordingMode = mode;
    try {
      await setRecordingMode(mode);
      if (mode === 'vad') {
        loadSilence();
      }
    } catch {
      recordingMode = prev;
    }
  }

  async function loadSilence() {
    try {
      silenceMs = await getVadSilenceMs();
    } catch {
      /* keep default */
    }
  }

  async function onRetentionChange(days: number) {
    if (days === historyRetentionDays) return;
    const prev = historyRetentionDays;
    historyRetentionDays = days;
    retentionError = null;
    try {
      // Persist the literal value: `0` = "Never" (explicitly disabled),
      // `n >= 1` = keep `n` days. `null` would mean "unset" and fall back to
      // the default on next read, so we never send null here.
      await setHistoryRetentionDays(days);
      // The post-insert pruner reads config fresh on every transcription, so
      // no restart is needed for new recordings to honor the change.
    } catch (e) {
      historyRetentionDays = prev;
      retentionError = formatIpcError(e as IpcError);
    }
  }

  async function saveSilence() {
    const clamped = Math.max(100, Math.min(5000, Math.floor(silenceMs)));
    silenceMs = clamped;
    try {
      await setVadSilenceMs(clamped);
    } catch (e) {
      console.warn('setVadSilenceMs failed', e);
    }
  }

  async function onAccelModeChange(newMode: 'auto' | 'cpu' | 'gpu') {
    if (newMode === accelMode) return;
    const prev = accelMode;
    accelMode = newMode;
    accelBusy = true;
    try {
      await setAccelerationMode(newMode);
      accelInfo = await getAccelerationInfo();
    } catch (e) {
      accelMode = prev;
      console.warn('setAccelerationMode failed', e);
    } finally {
      accelBusy = false;
    }
  }

  async function refreshModel() {
    try {
      modelStatus = await getModelStatus();
      modelStatuses = await listModelStatuses();
      modelError = null;
    } catch (e) {
      modelError = formatIpcError(e as IpcError);
    }
  }

  async function downloadModelNow(variant: string) {
    modelBusy = true;
    modelError = null;
    try {
      await downloadStore.download(variant);
      await refreshModel();
    } catch (e) {
      modelError = formatIpcError(e as IpcError);
    } finally {
      modelBusy = false;
    }
  }

  async function activateModel(variant: string) {
    modelBusy = true;
    modelError = null;
    try {
      await setActiveModel(variant);
      await refreshModel();
    } catch (e) {
      modelError = formatIpcError(e as IpcError);
    } finally {
      modelBusy = false;
    }
  }

  async function refreshDict() {
    try {
      dict = await getDictionary();
      dictError = null;
    } catch (e) {
      dictError = formatIpcError(e as IpcError);
    }
  }

  async function refreshAudioDevices() {
    audioBusy = true;
    try {
      audioDevices = await listAudioDevices();
      const sel = await getSelectedAudioDevice().catch(() => null);
      if (sel && audioDevices.includes(sel)) {
        selectedDevice = sel;
      } else if (audioDevices.length > 0 && !audioDevices.includes(selectedDevice)) {
        selectedDevice = audioDevices[0];
      }
    } catch (e) {
      audioDevices = ['Default'];
    } finally {
      audioBusy = false;
    }
  }

  async function onDeviceChange() {
    audioBusy = true;
    try {
      await setSelectedAudioDevice(selectedDevice || 'Default');
    } catch {
      // best effort; UI keeps value
    } finally {
      audioBusy = false;
    }
  }

  // ---- Hotkey rebinding --------------------------------------------------
  //
  // The user picks one of the predefined hotkeys. We send the parseable
  // spec to the backend, which unregisters the previous shortcut and
  // registers the new one. If registration fails (e.g. because Windows
  // already has that combo claimed by another app), the old binding is
  // preserved and we surface the error inline.

  async function pickHotkey(spec: string) {
    if (hotkeySaving !== null) return;
    hotkeyError = null;
    hotkeySaving = spec;
    try {
      const canonical = await setHotkey(spec);
      currentHotkey = canonical;
    } catch (e) {
      hotkeyError = formatIpcError(e as IpcError);
    } finally {
      hotkeySaving = null;
    }
  }

  // ---- Pill size + position ---------------------------------------------
  // Live-update the pill window as the user drags the sliders. IPC cost
  // is trivial (one window property set per frame) so no debouncing needed.

  function onPillWidthInput(e: Event) {
    const v = Number((e.currentTarget as HTMLInputElement).value);
    if (!Number.isFinite(v)) return;
    pillWidth = clamp(v, PILL_W_MIN, PILL_W_MAX);
    void applyPillSize();
  }
  function onPillHeightInput(e: Event) {
    const v = Number((e.currentTarget as HTMLInputElement).value);
    if (!Number.isFinite(v)) return;
    pillHeight = clamp(v, PILL_H_MIN, PILL_H_MAX);
    void applyPillSize();
  }
  async function applyPillSize() {
    if (pillBusy) return;
    pillBusy = true;
    try {
      await setPillSize(pillWidth, pillHeight);
    } catch (e) {
      console.warn('setPillSize failed', e);
    } finally {
      pillBusy = false;
    }
  }

  async function applyPositionPreset(preset: PillPositionPreset) {
    if (pillBusy) return;
    pillBusy = true;
    try {
      await setPillPositionPreset(preset);
    } catch (e) {
      console.warn('setPillPositionPreset failed', e);
    } finally {
      pillBusy = false;
    }
  }

  // ---- Accent color + font scale ---------------------------------------
  function onAccentPick(id: string) {
    accent = id;
    document.documentElement.setAttribute('data-accent', id);
    lsSet(LS_KEYS.accent, id);
  }

  function onFontScaleInput(e: Event) {
    const v = Number((e.currentTarget as HTMLInputElement).value);
    if (!Number.isFinite(v)) return;
    fontScalePct = clamp(v, 80, 125);
    document.documentElement.style.setProperty('--font-scale', String(fontScalePct / 100));
    lsSet(LS_KEYS.fontScale, fontScalePct);
  }

  async function addDict() {
    if (!canAddDict) return;
    dictBusy = true;
    try {
      dict = await addDictionaryEntry(newFrom.trim(), newTo);
      newFrom = '';
      newTo = '';
      dictError = null;
    } catch (e) {
      dictError = formatIpcError(e as IpcError);
    } finally {
      dictBusy = false;
    }
  }

  async function removeDict(entry: DictEntry) {
    if (!confirm(`Remove dictionary entry "${entry.from}" → "${entry.to}"?`)) return;
    dictBusy = true;
    try {
      dict = await removeDictionaryEntry(entry.from);
      dictError = null;
    } catch (e) {
      dictError = formatIpcError(e as IpcError);
    } finally {
      dictBusy = false;
    }
  }
</script>

<div class="settings">
  <header class="settings-header">
    <div>
      <h1>Settings</h1>
      <p class="muted">Changes save automatically.</p>
    </div>
  </header>

  <nav class="section-nav" aria-label="Settings sections">
    {#each SECTIONS as s (s.id)}
      <button type="button" class="sec-chip" onclick={() => scrollTo(s.id)}>{s.label}</button>
    {/each}
  </nav>

  <!-- General -->
  <section class="card" id="sec-general">
    <h2>General</h2>

    <label class="row">
      <div class="row-main">
        <div class="row-title">Start at login</div>
        <div class="row-sub">Launch DevWhisp automatically when you sign in.</div>
      </div>
      <div class="row-ctl">
        {#if autostartError}<span class="muted small">failed</span>{/if}
        <input
          type="checkbox"
          checked={autostartOn}
          disabled={autostartBusy}
          onchange={(e) => onAutostartChange((e.currentTarget as HTMLInputElement).checked)}
        />
      </div>
    </label>

    <div class="divider"></div>

    <label class="row">
      <div class="row-main">
        <div class="row-title">Show pill on startup</div>
        <div class="row-sub">Display the floating DevWhisp pill when the app launches.</div>
      </div>
      <div class="row-ctl">
        <input
          type="checkbox"
          checked={showPillOnStartup}
          onchange={(e) => onShowPillChange((e.currentTarget as HTMLInputElement).checked)}
        />
      </div>
    </label>
  </section>

  <!-- Recording -->
  <section class="card" id="sec-recording">
    <h2>Recording</h2>

    <div class="row row-top row-hotkey">
      <div class="row-main">
        <div class="row-title">Hotkey</div>
        <div class="row-sub">
          Pick the key combo that arms DevWhisp. The currently active
          binding is <strong>{currentHotkey}</strong> — hold it (or tap it,
          in toggle mode) to talk.
        </div>
      </div>
      <div class="row-ctl">
        <div class="hotkey-picker" role="radiogroup" aria-label="Recording hotkey">
          {#if predefinedHotkeys.length === 0}
            <div class="muted small">Loading…</div>
          {/if}
          {#each predefinedHotkeys as hk (hk.spec)}
            {@const isCurrent = hk.label === currentHotkey}
            {@const isSaving = hotkeySaving === hk.spec}
            <button
              type="button"
              class="hotkey-option"
              class:active={isCurrent}
              disabled={hotkeySaving !== null}
              role="radio"
              aria-checked={isCurrent}
              onclick={() => pickHotkey(hk.spec)}
            >
              <span class="hotkey-option-keys">
                {#each hk.label.split('+') as k, i (k + i)}<kbd>{k}</kbd>{#if i < hk.label.split('+').length - 1}<span class="hotkey-option-plus">+</span>{/if}{/each}
              </span>
              <span class="hotkey-option-desc">{hk.description}</span>
              {#if isSaving}
                <span class="hotkey-option-state">Saving…</span>
              {:else if isCurrent}
                <span class="hotkey-option-state">Active</span>
              {/if}
            </button>
          {/each}
        </div>
        {#if hotkeyError}
          <div class="row-error">{hotkeyError}</div>
        {/if}
      </div>
    </div>

    <div class="divider"></div>

    <div class="row">
      <div class="row-main">
        <div class="row-title">Input Device</div>
        <div class="row-sub">Microphone DevWhisp listens on. Refresh if you just plugged one in.</div>
      </div>
      <div class="row-ctl">
        <select id="audio-device" bind:value={selectedDevice} disabled={audioBusy} onchange={onDeviceChange}>
          {#each audioDevices as dev}
            <option value={dev}>{dev}</option>
          {/each}
        </select>
        <button onclick={() => refreshAudioDevices()} disabled={audioBusy}>Refresh</button>
      </div>
    </div>

    <div class="divider"></div>

    <div class="row">
      <div class="row-main">
        <div class="row-title">Mode</div>
        <div class="row-sub">
          {#if recordingMode === 'toggle'}
            Tap the hotkey to start, tap again to stop.
          {:else if recordingMode === 'vad'}
            Press to start listening; auto-stops after silence and transcribes.
          {:else}
            Hold the hotkey while you speak; release to transcribe.
          {/if}
        </div>
      </div>
      <div class="row-ctl">
        <div class="segmented" role="group" aria-label="Recording mode">
          <button
            type="button"
            class:active={recordingMode === 'push-to-talk'}
            onclick={() => onModeChange('push-to-talk')}
          >Push-to-talk</button>
          <button
            type="button"
            class:active={recordingMode === 'toggle'}
            onclick={() => onModeChange('toggle')}
          >Toggle</button>
          <button
            type="button"
            class:active={recordingMode === 'vad'}
            onclick={() => onModeChange('vad')}
          >VAD</button>
        </div>
      </div>
    </div>

    {#if recordingMode === 'vad'}
      <div class="divider"></div>
      <div class="row">
        <div class="row-main">
          <div class="row-title">Silence hold-off</div>
          <div class="row-sub">Auto-stop after this much silence (ms). Default 600.</div>
        </div>
        <div class="row-ctl">
          <input
            type="number"
            bind:value={silenceMs}
            min="100"
            max="5000"
            step="50"
            style="width: 90px"
            onchange={saveSilence}
          />
          <span style="margin-left: 6px; font-size: 12px; color: var(--muted)">ms</span>
        </div>
      </div>
    {/if}

    <div class="divider"></div>
    <div class="row">
      <div class="row-main">
        <div class="row-title">Keep history for</div>
        <div class="row-sub">
          Older transcriptions are deleted automatically to free up space.
          Default 2 days. “Never” keeps everything until you clear it manually.
        </div>
      </div>
      <div class="row-ctl">
        <div class="segmented" role="group" aria-label="History retention">
          {#each RETENTION_OPTIONS as opt (opt.days)}
            <button
              type="button"
              class:active={retentionLoaded && historyRetentionDays === opt.days}
              disabled={!retentionLoaded}
              onclick={() => onRetentionChange(opt.days)}
            >{opt.label}</button>
          {/each}
        </div>
        {#if retentionError}
          <div class="row-error">{retentionError}</div>
        {/if}
      </div>
    </div>
  </section>

  <!-- Performance / GPU-CPU adaptive -->
  <section class="card" id="sec-performance">
    <h2>Performance</h2>
    <p class="muted">Hardware acceleration for supported models (Moonshine via ONNX Runtime providers). Persists and applies immediately.</p>

    <div class="row">
      <div class="row-main">
        <div class="row-title">Acceleration mode</div>
        <div class="row-sub">
          Auto prefers GPU (CUDA / DirectML) when available at runtime, falls back to CPU. Applies to ONNX sessions.
        </div>
      </div>
      <div class="row-ctl">
        <div class="segmented" role="group" aria-label="Acceleration mode">
          <button
            type="button"
            class:active={accelMode === 'auto'}
            disabled={accelBusy}
            onclick={() => onAccelModeChange('auto')}
          >Auto</button>
          <button
            type="button"
            class:active={accelMode === 'cpu'}
            disabled={accelBusy}
            onclick={() => onAccelModeChange('cpu')}
          >CPU</button>
          <button
            type="button"
            class:active={accelMode === 'gpu'}
            disabled={accelBusy}
            onclick={() => onAccelModeChange('gpu')}
          >GPU</button>
        </div>
      </div>
    </div>

    {#if accelInfo}
      <div class="divider"></div>
      <div class="row">
        <div class="row-main">
          <div class="row-title">Status</div>
          <div class="row-sub mono">detected: {accelInfo.detected} · in use: {accelInfo.inUse}</div>
        </div>
        <div class="row-ctl">
          <span class="badge" style="background: var(--accent-soft); color: var(--accent);">{accelInfo.mode}</span>
        </div>
      </div>
    {:else}
      <p class="muted small">Loading acceleration status…</p>
    {/if}
    <p class="muted small">GPU providers attempted at load: CUDA, DirectML (Windows), WebGPU; always falls back to CPU. Whisper GPU is compile-time only.</p>
  </section>

  <!-- Models -->
  <section class="card" id="sec-models">
    <h2>Models</h2>

    <p class="muted">
      On-device Whisper models (same ladder as BridgeVoice). <strong>Base</strong> is recommended
      for most machines. Larger models need more disk, RAM, and CPU time.
    </p>

    {#if modelError}
      <div class="row-error">{modelError}</div>
    {:else if modelStatuses.length > 0}
      {#each modelStatuses as m}
        <div class="model-card" class:active={m.variant === modelStatus?.variant}>
          <div class="model-info">
            <div class="model-name">
              {m.displayName || m.variant}
              {#if m.variant === 'whisper-base-en'}<span class="badge recommended">Recommended</span>{/if}
              {#if m.variant === modelStatus?.variant}<span class="badge">Active</span>{/if}
            </div>
            <div class="row-sub">{m.description || ''}</div>
            {#if m.path}<div class="row-sub mono">{m.path}</div>{/if}
          </div>
          <div class="model-size">
            {#if downloadStore.variant === m.variant && downloadStore.isDownloading}
              <span class="warn font-mono">Downloading… {downloadStore.pct.toFixed(0)}% ({downloadStore.downloadedMB}/{downloadStore.totalMB} MB)</span>
            {:else if m.ready}
              <span class="ok">● {m.fileSizeMb} MB · ready</span>
            {:else}
              <span class="muted">~{m.expectedSizeMb} MB · not downloaded</span>
            {/if}
          </div>
          <div class="model-actions">
            {#if downloadStore.variant === m.variant && downloadStore.isDownloading}
              <button disabled>Downloading…</button>
            {:else if !m.ready}
              <button onclick={() => downloadModelNow(m.variant)} disabled={modelBusy || downloadStore.isDownloading}>Download</button>
            {:else if m.variant !== modelStatus?.variant}
              <button onclick={() => activateModel(m.variant)} disabled={modelBusy || downloadStore.isDownloading}>Use this model</button>
            {/if}
          </div>
        </div>
      {/each}
      <p class="muted small">
        Models stay on disk after download. Switching reloads the engine on the next transcription.
        Moonshine Tiny needs a build with <code>--features moonshine</code>
        (<code>npm run tauri:dev:moonshine</code>.
      </p>
    {:else}
      <p class="muted">Loading model status…</p>
    {/if}
  </section>

  <!-- Appearance -->
  <section class="card" id="sec-appearance">
    <h2>Appearance</h2>
    <p class="muted">Tune how the floating pill looks. Changes apply live.</p>

    <div class="row">
      <div class="row-main">
        <div class="row-title">Accent color</div>
        <div class="row-sub">Recolors the violet accent used across the app and pill.</div>
      </div>
      <div class="row-ctl" style="gap: 8px;">
        {#each ACCENTS as a}
          <button
            type="button"
            class="accent-chip"
            class:active={accent === a.id}
            style:--chip-color={a.color}
            title={a.label}
            aria-label={a.label}
            aria-pressed={accent === a.id}
            onclick={() => onAccentPick(a.id)}
          ></button>
        {/each}
      </div>
    </div>

    <div class="divider"></div>

    <div class="row">
      <div class="row-main">
        <div class="row-title">Font size</div>
        <div class="row-sub">Scales every text element proportionally (80–125%).</div>
      </div>
      <div class="row-ctl row-slider">
        <input
          type="range"
          min="80"
          max="125"
          step="5"
          value={fontScalePct}
          oninput={onFontScaleInput}
          aria-label="Font size scale"
        />
        <span class="mono small">{fontScalePct}%</span>
      </div>
    </div>

    <div class="divider"></div>

    <div class="row">
      <div class="row-main">
        <div class="row-title">Pill width</div>
        <div class="row-sub">Resizes the floating pill window live.</div>
      </div>
      <div class="row-ctl row-slider">
        <input
          type="range"
          min={PILL_W_MIN}
          max={PILL_W_MAX}
          step="2"
          value={pillWidth}
          oninput={onPillWidthInput}
          aria-label="Pill width"
        />
        <span class="mono small">{pillWidth}px</span>
      </div>
    </div>

    <div class="divider"></div>

    <div class="row">
      <div class="row-main">
        <div class="row-title">Pill height</div>
        <div class="row-sub">Resizes the floating pill window live.</div>
      </div>
      <div class="row-ctl row-slider">
        <input
          type="range"
          min={PILL_H_MIN}
          max={PILL_H_MAX}
          step="2"
          value={pillHeight}
          oninput={onPillHeightInput}
          aria-label="Pill height"
        />
        <span class="mono small">{pillHeight}px</span>
      </div>
    </div>

    <div class="divider"></div>

    <div class="row">
      <div class="row-main">
        <div class="row-title">Snap pill to corner</div>
        <div class="row-sub">Reposition the pill instantly. Position persists across restarts.</div>
      </div>
      <div class="row-ctl" style="flex-wrap: wrap; gap: 6px; max-width: 280px; justify-content: flex-end;">
        {#each POS_PRESETS as p}
          <button
            type="button"
            class="pos-preset"
            disabled={pillBusy}
            onclick={() => applyPositionPreset(p.id)}
            aria-label={`Snap pill to ${p.label}`}
          >{p.label}</button>
        {/each}
      </div>
    </div>

    <div class="divider"></div>

    <div class="row">
      <div class="row-main">
        <div class="row-title">Pill opacity</div>
        <div class="row-sub">Higher = more solid; lower = more see-through.</div>
      </div>
      <div class="row-ctl row-slider">
        <input
          type="range"
          min="8"
          max="90"
          step="1"
          value={Math.round(pillBgAlpha * 100)}
          oninput={onPillBgAlphaInput}
          aria-label="Pill background opacity"
        />
        <span class="mono small">{Math.round(pillBgAlpha * 100)}%</span>
      </div>
    </div>

    <div class="divider"></div>

    <label class="row">
      <div class="row-main">
        <div class="row-title">Flat (no halo)</div>
        <div class="row-sub">Drop the soft glow for a minimalist look.</div>
      </div>
      <div class="row-ctl">
        <input
          type="checkbox"
          checked={pillNoHalo}
          onchange={(e) => onPillNoHaloChange((e.currentTarget as HTMLInputElement).checked)}
        />
      </div>
    </label>

    <div class="divider"></div>

    <label class="row">
      <div class="row-main">
        <div class="row-title">Compact mode</div>
        <div class="row-sub">A smaller pill for tight screens.</div>
      </div>
      <div class="row-ctl">
        <input
          type="checkbox"
          checked={pillCompact}
          onchange={(e) => onPillCompactChange((e.currentTarget as HTMLInputElement).checked)}
        />
      </div>
    </label>
  </section>

  <!-- Text & dictionary -->
  <section class="card" id="sec-text">
    <h2>Text &amp; dictionary</h2>
    <p class="muted">Applied automatically to every transcription before it's pasted.</p>

    <label class="row">
      <div class="row-main">
        <div class="row-title">Auto-capitalize first word</div>
        <div class="row-sub">"hello world" → "Hello world"</div>
      </div>
      <div class="row-ctl">
        <input
          type="checkbox"
          checked={autoCapitalize}
          onchange={(e) => onAutoCapitalizeChange((e.currentTarget as HTMLInputElement).checked)}
        />
      </div>
    </label>

    <div class="divider"></div>

    <label class="row">
      <div class="row-main">
        <div class="row-title">Append trailing space</div>
        <div class="row-sub">Always paste a space after the transcript.</div>
      </div>
      <div class="row-ctl">
        <input
          type="checkbox"
          checked={appendSpace}
          onchange={(e) => onAppendSpaceChange((e.currentTarget as HTMLInputElement).checked)}
        />
      </div>
    </label>

    <div class="divider"></div>

    <div class="subhead">Custom replacements</div>
    <p class="muted">Case-insensitive, whole-word, longest match first.</p>

    {#if dictError}<div class="row-error">{dictError}</div>{/if}

    {#if dict.length === 0}
      <p class="muted empty">No replacements yet.</p>
    {:else}
      <ul class="dict-list">
        {#each dict as entry, i (entry.from + '|' + entry.to + '|' + i)}
          <li class="dict-row">
            <code class="from">{entry.from}</code>
            <span class="arrow">→</span>
            <code class="to">{entry.to}</code>
            <button
              class="danger-outline small"
              onclick={() => removeDict(entry)}
              disabled={dictBusy}
              aria-label="Delete dictionary entry"
            >Delete</button>
          </li>
        {/each}
      </ul>
    {/if}

    <div class="dict-add">
      <input type="text" placeholder="from (e.g. devwhisp)" bind:value={newFrom} aria-label="Dictionary from" />
      <span class="arrow">→</span>
      <input type="text" placeholder="to (e.g. DevWhisp)" bind:value={newTo} aria-label="Dictionary to" />
      <button class="primary" onclick={addDict} disabled={!canAddDict}>Add</button>
    </div>
  </section>

  <!-- About -->
  <section class="card about" id="sec-about">
    <h2>About</h2>
    <div class="about-head">
      <span class="about-icon"><AppIcon size={44} /></span>
      <div>
        <div class="about-name">DevWhisp</div>
        <div class="row-sub mono">v{appInfo?.version ?? '0.1.0'}</div>
        <div class="row-sub">Local, offline voice-to-text. Adaptive CPU/GPU (Auto detects ONNX providers).</div>
      </div>
    </div>
    <div class="divider"></div>
    <p class="muted">
      Part of the <strong>Dev</strong> family — alongside <strong>DevTerm</strong> and
      <strong>DevSpace</strong>. Built with Tauri 2, Svelte 5, and whisper-rs.
    </p>
    <div class="about-links">
      <button
        type="button"
        class="link-btn"
        onclick={() => void invoke('open_external', { url: 'https://github.com/AEmad99/devwhisp' }).catch(() => {})}
      >Project page ↗</button>
    </div>
  </section>
</div>

<style>
  .settings {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .settings-header { padding: 2px 0 0; }
  h1 {
    margin: 0;
    font-size: 18px;
    font-weight: 650;
    letter-spacing: -0.02em;
    color: var(--text);
  }
  .muted { color: var(--muted); font-size: 11.5px; margin: 2px 0 0; }
  .muted.small { font-size: 10.5px; }
  .muted.empty { padding: 6px 0; }

  .section-nav {
    position: sticky;
    top: 0;
    z-index: 5;
    display: flex;
    gap: 3px;
    flex-wrap: wrap;
    padding: 5px 2px;
    margin: 0 -4px;
    background: color-mix(in srgb, var(--bg) 92%, transparent);
    backdrop-filter: blur(10px);
    border-bottom: 1px solid var(--border);
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.2);
  }
  .sec-chip {
    background: var(--card);
    border: 1px solid var(--border);
    color: var(--muted);
    padding: 3px 8px;
    border-radius: 999px;
    font-size: 10.5px;
    font-weight: 500;
    font-family: inherit;
    cursor: pointer;
    transition: all 120ms ease;
  }
  .sec-chip:hover { color: var(--text); border-color: var(--accent); }

  .card {
    background: var(--card);
    border: 1px solid var(--border);
    border-radius: var(--r-md);
    padding: 12px 14px;
    box-shadow: var(--shadow-1);
    scroll-margin-top: 48px;
  }
  .settings > .card:first-of-type {
    margin-top: 4px;
  }

  h2 {
    margin: 0 0 10px;
    font-size: 10.5px;
    font-weight: 700;
    color: var(--accent);
    text-transform: uppercase;
    letter-spacing: 0.1em;
  }
  .subhead {
    margin-top: 4px;
    font-size: 12px;
    font-weight: 600;
    color: var(--text);
  }

  .divider { height: 1px; background: var(--border); margin: 10px -14px; }

  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 4px 0;
    flex-wrap: wrap;
  }
  .row.row-top { align-items: flex-start; }
  .row-main { flex: 1 1 0; min-width: min(100%, 200px); }
  .row-title { color: var(--text); font-size: 13px; font-weight: 500; }
  .row-sub { color: var(--muted); font-size: 11px; margin-top: 1px; overflow-wrap: anywhere; }
  .row-sub.mono { font-family: ui-monospace, "JetBrains Mono", monospace; word-break: break-all; }
  .row-ctl { display: inline-flex; align-items: center; gap: 6px; flex-shrink: 0; flex: 0 1 auto; min-width: 0; }

  /* Hotkey row: always stack description above the picker list.
     This guarantees good layout at *any* window size (no side-by-side squeeze
     that squishes text to 1-char lines or causes overlap on minimized views).
     On wide screens the list simply uses the full card width below the label. */
  .row.row-hotkey {
    flex-direction: column;
    align-items: stretch;
  }
  .row.row-hotkey .row-ctl {
    display: block;
    width: 100%;
  }
  .row.row-hotkey .hotkey-picker {
    width: 100%;
    max-height: 220px;
    align-items: center; /* center the choice buttons for balanced look on wide windows; still full-width text above */
  }
  .row.row-hotkey .hotkey-option {
    max-width: 520px; /* keep individual options readable even on ultra-wide windows */
  }
  .row-slider { gap: 12px; flex: 1 1 auto; justify-content: flex-start; }
  .row-slider input[type='range'] { flex: 1 1 auto; min-width: 120px; max-width: 220px; }

  select, input[type='number'] { max-width: 100%; }
  .row-error {
    color: var(--danger);
    font-family: ui-monospace, "JetBrains Mono", monospace;
    font-size: 12px;
    margin-bottom: 8px;
  }

  /* Segmented control */
  .segmented {
    display: inline-flex;
    flex-wrap: wrap;
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: 999px;
    padding: 3px;
  }
  .segmented button {
    border: none;
    background: transparent;
    color: var(--muted);
    padding: 4px 11px;
    border-radius: 999px;
    font-size: 11px;
    font-weight: 600;
    font-family: inherit;
    cursor: pointer;
    transition: all 120ms ease;
  }
  .segmented button.active {
    background: var(--accent-deep);
    color: #fff;
    box-shadow: var(--shadow-1);
  }

  input[type='checkbox'] {
    appearance: none;
    -webkit-appearance: none;
    width: 38px;
    height: 22px;
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: 999px;
    position: relative;
    cursor: pointer;
    transition: background 150ms ease, border-color 150ms ease;
    outline: none;
  }
  input[type='checkbox']:disabled { opacity: 0.5; cursor: not-allowed; }
  input[type='checkbox']::after {
    content: '';
    position: absolute;
    top: 2px;
    left: 2px;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: var(--muted);
    transition: transform 150ms ease, background 150ms ease;
  }
  input[type='checkbox']:checked { background: var(--accent-deep); border-color: var(--accent-deep); }
  input[type='checkbox']:checked::after { transform: translateX(16px); background: white; }

  button {
    background: var(--card);
    color: var(--text);
    border: 1px solid var(--border);
    padding: 6px 12px;
    border-radius: 8px;
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    transition: all 150ms ease;
    font-family: inherit;
  }
  button:disabled { opacity: 0.5; cursor: not-allowed; }
  button.small { padding: 4px 10px; font-size: 12px; }
  button.primary { background: var(--accent-deep); border-color: var(--accent-deep); color: white; }
  button.primary:hover:not(:disabled) { background: var(--accent); border-color: var(--accent); }
  button.danger-outline { color: var(--danger); border-color: rgba(255, 92, 124, 0.4); }
  button.danger-outline:hover:not(:disabled) { background: rgba(255, 92, 124, 0.1); border-color: var(--danger); }

  kbd {
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: 5px;
    padding: 2px 7px;
    font-family: ui-monospace, "JetBrains Mono", monospace;
    font-size: 11px;
    color: var(--text);
  }
  .mono { font-family: ui-monospace, "JetBrains Mono", monospace; }

  /* Models */
  .model-card {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding: 12px 14px;
    border-radius: var(--r-md);
    border: 1px solid var(--border);
    background: var(--bg-elevated);
    margin-bottom: 8px;
  }
  .model-card.active { border-color: var(--accent); }
  .model-name { color: var(--text); font-size: 14px; font-weight: 600; display: flex; align-items: center; gap: 8px; }
  .badge {
    font-size: 9px; font-weight: 700; letter-spacing: 0.08em; text-transform: uppercase;
    padding: 2px 7px; border-radius: 999px; background: var(--accent-soft); color: var(--accent);
  }
  .badge.recommended {
    background: rgba(52, 211, 153, 0.14);
    color: var(--ok);
  }
  .model-size { font-family: ui-monospace, "JetBrains Mono", monospace; font-size: 12px; text-align: right; flex-shrink: 0; }
  .ok { color: var(--ok); }

  /* Dictionary */
  .dict-list { list-style: none; margin: 12px 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
  .dict-row {
    display: flex; align-items: center; gap: 10px; padding: 8px 12px;
    background: var(--bg-elevated); border: 1px solid var(--border); border-radius: 8px;
  }
  .dict-row code { font-family: ui-monospace, "JetBrains Mono", monospace; font-size: 13px; padding: 2px 6px; border-radius: 4px; }
  .dict-row .from { background: var(--accent-soft); color: var(--accent); }
  .dict-row .to { background: rgba(52, 211, 153, 0.12); color: var(--ok); flex: 1; }
  .dict-row .arrow, .dict-add .arrow { color: var(--muted); flex-shrink: 0; }

  .dict-add {
    display: flex; align-items: center; gap: 8px; margin-top: 14px;
    padding-top: 14px; border-top: 1px dashed var(--border);
  }
  .dict-add input[type='text'] {
    flex: 1; background: var(--bg-elevated); border: 1px solid var(--border); color: var(--text);
    border-radius: 8px; padding: 8px 12px; font-size: 13px;
    font-family: ui-monospace, "JetBrains Mono", monospace; outline: none; transition: border-color 150ms ease;
  }
  .dict-add input[type='text']:focus { border-color: var(--accent); }

  /* About */
  .about-head { display: flex; align-items: center; gap: 12px; }
  .about-icon { display: inline-flex; flex-shrink: 0; }
  .about-name { font-size: 16px; font-weight: 700; color: var(--text); letter-spacing: -0.02em; }
  .about-links { margin-top: 12px; }
  .about-links .link-btn {
    background: none; border: none; padding: 0; cursor: pointer;
    color: var(--accent); font-size: 13px; font-family: inherit;
  }
  .about-links .link-btn:hover { text-decoration: underline; }

  /* Hotkey picker (predefined options, no free-form text) */
  .hotkey-picker {
    display: flex;
    flex-direction: column;
    gap: 6px;
    max-height: 280px;
    overflow-y: auto;
    padding-right: 2px;
  }
  .hotkey-option {
    appearance: none;
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    color: var(--text);
    border-radius: 10px;
    padding: 8px 10px;
    font-family: inherit;
    text-align: left;
    cursor: pointer;
    display: grid;
    grid-template-columns: auto 1fr auto;
    align-items: center;
    gap: 10px;
    transition: border-color 150ms ease, background 150ms ease, transform 80ms ease;
    width: 100%;
    box-sizing: border-box;
  }
  .hotkey-option:hover:not(:disabled) {
    border-color: var(--accent);
    background: var(--card-hover);
  }
  .hotkey-option:active:not(:disabled) { transform: translateY(1px); }
  .hotkey-option:disabled { opacity: 0.55; cursor: progress; }
  .hotkey-option.active {
    border-color: var(--accent);
    background: var(--accent-soft);
  }
  .hotkey-option-keys {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    min-width: 80px;
    flex-shrink: 0;
  }
  .hotkey-option-plus {
    color: var(--muted);
    font-size: 12px;
    padding: 0 1px;
  }
  .hotkey-option-desc {
    color: var(--muted);
    font-size: 11px;
    line-height: 1.25;
    white-space: normal;
    overflow: hidden;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
  }
  .hotkey-option-state {
    color: var(--accent);
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  /* Accent color chips */
  .accent-chip {
    appearance: none;
    border: 2px solid transparent;
    border-radius: 999px;
    width: 26px;
    height: 26px;
    padding: 0;
    cursor: pointer;
    background: var(--chip-color, var(--accent));
    box-shadow:
      0 0 0 1px var(--border),
      inset 0 1px 0 rgba(255, 255, 255, 0.15);
    transition: transform 120ms ease, border-color 120ms ease;
  }
  .accent-chip:hover { transform: scale(1.08); }
  .accent-chip.active {
    border-color: var(--text);
    box-shadow:
      0 0 0 2px var(--accent),
      inset 0 1px 0 rgba(255, 255, 255, 0.15);
  }

  /* Position preset buttons */
  .pos-preset {
    appearance: none;
    background: var(--bg-elevated);
    color: var(--text);
    border: 1px solid var(--border);
    padding: 5px 11px;
    border-radius: 999px;
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    transition: all 140ms ease;
    font-family: inherit;
  }
  .pos-preset:hover:not(:disabled) {
    border-color: var(--accent);
    color: var(--accent);
  }
</style>
