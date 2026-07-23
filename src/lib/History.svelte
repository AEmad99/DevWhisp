<script lang="ts">
  /**
   * History view — shows the persisted transcription list, with search,
   * per-row copy / delete, and a clear-all affordance. All persistence goes
   * through src/lib/api.ts (track C IPC commands).
   */

  import { onMount } from 'svelte';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import { writeText } from '@tauri-apps/plugin-clipboard-manager';
  import {
    listHistory,
    searchHistory,
    deleteHistoryEntry,
    clearHistory,
    reinjectText,
    type HistoryEntry,
    type IpcError,
    formatIpcError,
  } from './api';

  let entries = $state<HistoryEntry[]>([]);
  let searchQuery = $state('');
  let loading = $state(false);
  let busy = $state(false);
  let error = $state<string | null>(null);
  let copiedId = $state<number | null>(null);
  let expandedId = $state<number | null>(null);
  let pastedId = $state<number | null>(null);

  function toggleExpand(id: number) {
    expandedId = expandedId === id ? null : id;
  }

  async function reinject(entry: HistoryEntry) {
    try {
      await reinjectText(entry.text);
      pastedId = entry.id;
      window.setTimeout(() => {
        if (pastedId === entry.id) pastedId = null;
      }, 1500);
    } catch (e) {
      error = formatIpcError(e as IpcError);
    }
  }

  /** Tracks the active search so we can ignore stale results. */
  let searchToken = 0;

  // ---- Time filter (All / Today / This week) -------------------------------
  type TimeFilter = 'all' | 'today' | 'week';
  const FILTERS: { id: TimeFilter; label: string }[] = [
    { id: 'all', label: 'All' },
    { id: 'today', label: 'Today' },
    { id: 'week', label: 'This week' },
  ];
  let filter = $state<TimeFilter>('all');

  function startOfToday(now = Date.now()): number {
    const d = new Date(now);
    d.setHours(0, 0, 0, 0);
    return d.getTime();
  }

  /** Entries after applying the active time filter (search results included). */
  let visibleEntries = $derived.by(() => {
    if (filter === 'all') return entries;
    const now = Date.now();
    const cutoff = filter === 'today' ? startOfToday(now) : now - 7 * 86_400_000;
    return entries.filter((e) => e.created_at >= cutoff);
  });

  function wordCount(text: string): number {
    return text.trim().split(/\s+/).filter(Boolean).length;
  }

  /** Footer stats over the currently-visible entries. */
  let stats = $derived.by(() => {
    let words = 0;
    let durMs = 0;
    for (const e of visibleEntries) {
      words += wordCount(e.text);
      if (e.duration_ms && e.duration_ms > 0) durMs += e.duration_ms;
    }
    const minutes = durMs / 60_000;
    const wpm = minutes > 0 ? Math.round(words / minutes) : 0;
    return { count: visibleEntries.length, words, wpm };
  });

  async function refresh() {
    loading = true;
    error = null;
    try {
      entries = await listHistory(100, 0);
    } catch (e) {
      error = formatIpcError(e as IpcError);
      entries = [];
    } finally {
      loading = false;
    }
  }

  async function runSearch(query: string) {
    const trimmed = query.trim();
    if (!trimmed) {
      await refresh();
      return;
    }
    const myToken = ++searchToken;
    try {
      const results = await searchHistory(trimmed, 50);
      if (myToken !== searchToken) return; // stale
      entries = results;
    } catch (e) {
      error = formatIpcError(e as IpcError);
    }
  }

  let searchTimer: number | null = null;
  function onSearchInput(value: string) {
    searchQuery = value;
    if (searchTimer !== null) window.clearTimeout(searchTimer);
    searchTimer = window.setTimeout(() => {
      runSearch(searchQuery);
    }, 200);
  }

  async function copyEntry(entry: HistoryEntry) {
    try {
      await writeText(entry.text);
      copiedId = entry.id;
      window.setTimeout(() => {
        if (copiedId === entry.id) copiedId = null;
      }, 1500);
    } catch (e) {
      error = formatIpcError(e as IpcError);
    }
  }

  async function deleteEntry(entry: HistoryEntry) {
    if (!confirm(`Delete this transcription?\n\n"${truncate(entry.text, 60)}"`)) {
      return;
    }
    busy = true;
    try {
      await deleteHistoryEntry(entry.id);
      entries = entries.filter((e) => e.id !== entry.id);
    } catch (e) {
      error = formatIpcError(e as IpcError);
    } finally {
      busy = false;
    }
  }

  async function clearAll() {
    if (!confirm(`Delete all ${entries.length} transcriptions? This cannot be undone.`)) {
      return;
    }
    busy = true;
    try {
      await clearHistory();
      entries = [];
    } catch (e) {
      error = formatIpcError(e as IpcError);
    } finally {
      busy = false;
    }
  }

  function truncate(s: string, n: number): string {
    if (s.length <= n) return s;
    return s.slice(0, n) + '…';
  }

  /**
   * "2 min ago" style for the last hour, then "today/yesterday",
   * then absolute date. Keeps the UI scannable without an extra date picker.
   */
  function relativeTime(epochMs: number, now = Date.now()): string {
    const diff = Math.max(0, now - epochMs);
    const sec = Math.floor(diff / 1000);
    if (sec < 5) return 'just now';
    if (sec < 60) return `${sec} sec ago`;
    const min = Math.floor(sec / 60);
    if (min < 60) return `${min} min ago`;
    const hr = Math.floor(min / 60);
    if (hr < 24) return `${hr} hr ago`;
    const day = Math.floor(hr / 24);
    if (day === 1) return 'yesterday';
    if (day < 7) return `${day} days ago`;
    return new Date(epochMs).toLocaleDateString();
  }

  function durationLabel(ms: number | null): string {
    if (ms === null || ms === undefined) return '–';
    if (ms < 1000) return `${ms} ms`;
    return `${(ms / 1000).toFixed(1)}s`;
  }

  onMount(() => {
    refresh();
    let unlistenState: UnlistenFn | null = null;
    try {
      listen<{ state: string }>('pill-state', (e) => {
        if (e.payload?.state === 'success' && !searchQuery.trim()) {
          window.setTimeout(refresh, 400);
        }
      })
        .then((fn) => (unlistenState = fn))
        .catch(() => {});
    } catch {
      /* no Tauri bridge */
    }
    return () => {
      if (unlistenState) unlistenState();
    };
  });
</script>

<div class="history">
  <header class="hist-header">
    <div>
      <h1>History</h1>
      <p class="muted">Your transcriptions, newest first.</p>
    </div>
    <div class="header-actions">
      <button onclick={clearAll} disabled={busy || entries.length === 0} class="danger">
        Clear all
      </button>
    </div>
  </header>

  <section class="card search-card">
    <input
      type="search"
      placeholder="Search transcriptions…"
      value={searchQuery}
      oninput={(e) => onSearchInput((e.currentTarget as HTMLInputElement).value)}
      aria-label="Search history"
    />
    {#if loading}
      <span class="muted">Loading…</span>
    {:else}
      <span class="muted">
        {visibleEntries.length} {visibleEntries.length === 1 ? 'entry' : 'entries'}
      </span>
    {/if}
  </section>

  <div class="filter-chips" role="group" aria-label="Filter by time">
    {#each FILTERS as f (f.id)}
      <button
        type="button"
        class="chip"
        class:active={filter === f.id}
        aria-pressed={filter === f.id}
        onclick={() => (filter = f.id)}
      >
        {f.label}
      </button>
    {/each}
  </div>

  {#if error}
    <section class="card error">
      <strong>Error:</strong> {error}
    </section>
  {/if}

  {#if !loading && visibleEntries.length === 0}
    <section class="card empty">
      <div class="empty-icon" aria-hidden="true">
        <!-- Mic icon, faded. -->
        <svg viewBox="0 0 24 24" width="48" height="48" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
          <rect x="9" y="3" width="6" height="12" rx="3" />
          <path d="M5 11a7 7 0 0 0 14 0" />
          <line x1="12" y1="18" x2="12" y2="22" />
          <line x1="8" y1="22" x2="16" y2="22" />
        </svg>
      </div>
      {#if filter !== 'all'}
        <p>No transcriptions in this range. Try <strong>All</strong>.</p>
      {:else}
        <p>No transcriptions yet. Press <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>Space</kbd> to start.</p>
      {/if}
    </section>
  {:else}
    <ul class="entry-list" aria-label="Transcription history">
      {#each visibleEntries as entry (entry.id)}
        <li class="entry" class:expanded={expandedId === entry.id}>
          <button
            type="button"
            class="entry-main"
            onclick={() => toggleExpand(entry.id)}
            aria-expanded={expandedId === entry.id}
            title="Click to expand"
          >
            <div class="entry-meta">
              <span class="time">{relativeTime(entry.created_at)}</span>
              <span class="dot">·</span>
              <span class="duration">{durationLabel(entry.duration_ms)}</span>
              <span class="dot">·</span>
              <span class="wc">{wordCount(entry.text)}w</span>
              {#if entry.source}
                <span class="source-tag">{entry.source}</span>
              {/if}
            </div>
            <div class="entry-text" class:full={expandedId === entry.id}>
              {expandedId === entry.id ? entry.text : truncate(entry.text, 80)}
            </div>
          </button>
          <div class="entry-actions">
            <button onclick={() => reinject(entry)} aria-label="Paste again" title="Paste into the focused app again">
              {pastedId === entry.id ? '✓ Pasted' : 'Paste'}
            </button>
            <button onclick={() => copyEntry(entry)} aria-label="Copy transcription">
              {copiedId === entry.id ? '✓ Copied' : 'Copy'}
            </button>
            <button onclick={() => deleteEntry(entry)} disabled={busy} class="danger-outline" aria-label="Delete entry">
              Delete
            </button>
          </div>
        </li>
      {/each}
    </ul>

    <footer class="hist-footer" aria-label="History statistics">
      <span><strong>{stats.count}</strong> {stats.count === 1 ? 'transcription' : 'transcriptions'}</span>
      <span class="dot">·</span>
      <span><strong>{stats.words.toLocaleString()}</strong> words</span>
      <span class="dot">·</span>
      <span>avg <strong>{stats.wpm}</strong> wpm</span>
    </footer>
  {/if}
</div>

<style>
  .history {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .hist-header {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: 12px;
    flex-wrap: wrap;
    padding: 2px 0;
  }

  h1 {
    margin: 0;
    font-size: 18px;
    font-weight: 650;
    letter-spacing: -0.02em;
    color: var(--text);
  }
  .muted { color: var(--muted); font-size: 11.5px; margin: 2px 0 0; }

  .card {
    background: var(--card);
    border: 1px solid var(--border);
    border-radius: var(--r-md);
    padding: 10px 12px;
  }

  .search-card {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .search-card input {
    flex: 1;
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    color: var(--text);
    border-radius: 7px;
    padding: 7px 10px;
    font-size: 13px;
    font-family: inherit;
    outline: none;
    transition: border-color 140ms ease;
  }
  .search-card input:focus { border-color: var(--accent); }

  .filter-chips {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
  }
  .chip {
    background: var(--card);
    border: 1px solid var(--border);
    color: var(--muted);
    padding: 4px 10px;
    border-radius: 999px;
    font-size: 11px;
    font-weight: 500;
    font-family: inherit;
    cursor: pointer;
    transition: all 120ms ease;
  }
  .chip:hover { color: var(--text); border-color: var(--accent); }
  .chip.active {
    background: var(--accent-soft);
    border-color: var(--accent);
    color: var(--accent);
  }

  .hist-footer {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 10px 2px 2px;
    border-top: 1px solid var(--border);
    margin-top: 2px;
    color: var(--muted);
    font-size: 11px;
    font-family: ui-monospace, "JetBrains Mono", monospace;
  }
  .hist-footer strong { color: var(--text); font-weight: 600; }
  .hist-footer .dot { opacity: 0.5; }

  .entry-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .entry {
    background: var(--card);
    border: 1px solid var(--border);
    border-radius: var(--r-md);
    padding: 10px 12px;
    display: flex;
    gap: 10px;
    align-items: flex-start;
    transition: border-color 140ms ease, background 140ms ease;
  }
  .entry:hover { border-color: color-mix(in srgb, var(--accent) 35%, transparent); }

  .entry-main {
    flex: 1;
    min-width: 0;
    background: transparent;
    border: none;
    padding: 0;
    text-align: left;
    cursor: pointer;
    font: inherit;
    color: inherit;
    display: block;
    width: 100%;
  }
  .entry-meta {
    display: flex;
    align-items: center;
    gap: 5px;
    font-size: 10.5px;
    color: var(--muted);
    font-family: ui-monospace, "JetBrains Mono", monospace;
    margin-bottom: 4px;
  }
  .entry-meta .time { color: var(--accent); }
  .entry-meta .dot { opacity: 0.5; }
  .source-tag {
    margin-left: 4px;
    padding: 1px 5px;
    border-radius: 999px;
    background: var(--accent-soft);
    color: var(--accent);
    font-size: 9px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }
  .entry-text {
    font-size: 13px;
    line-height: 1.4;
    color: var(--text);
    overflow-wrap: break-word;
  }
  .entry-text.full { white-space: pre-wrap; }
  .entry-meta .wc { color: var(--faint); }
  .entry.expanded { border-color: var(--accent); background: var(--card-hover); }

  .entry-actions {
    display: flex;
    gap: 4px;
    flex-shrink: 0;
  }

  button {
    background: var(--card);
    color: var(--text);
    border: 1px solid var(--border);
    padding: 5px 9px;
    border-radius: 6px;
    font-size: 11px;
    font-weight: 500;
    cursor: pointer;
    transition: all 130ms ease;
    font-family: inherit;
  }
  button:hover:not(:disabled) { border-color: var(--accent); }
  button:disabled { opacity: 0.5; cursor: not-allowed; }
  button.danger {
    background: var(--danger);
    border-color: var(--danger);
    color: white;
  }
  button.danger:disabled { background: var(--danger); }
  button.danger-outline {
    color: var(--danger);
    border-color: rgba(255, 107, 138, 0.4);
  }
  button.danger-outline:hover:not(:disabled) {
    background: rgba(255, 107, 138, 0.1);
    border-color: var(--danger);
  }

  .empty {
    text-align: center;
    padding: 36px 16px;
  }
  .empty-icon {
    color: var(--muted);
    opacity: 0.3;
    margin-bottom: 10px;
  }
  .empty p {
    margin: 0;
    color: var(--muted);
    font-size: 13px;
  }
  .empty kbd {
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 1px 5px;
    font-family: ui-monospace, "JetBrains Mono", monospace;
    font-size: 10.5px;
    color: var(--text);
  }

  .error {
    border-color: var(--danger);
    background: rgba(255, 107, 138, 0.06);
    color: var(--danger);
    font-family: ui-monospace, "JetBrains Mono", monospace;
    font-size: 12px;
    padding: 10px 12px;
    border-left-width: 3px;
  }
  .error strong { color: var(--danger); }
</style>