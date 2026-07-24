<script lang="ts">
  import { downloadStore } from './downloadStore.svelte';

  const visible = $derived(
    downloadStore.isDownloading || downloadStore.isCompleted || downloadStore.error !== null
  );

  function handleRetry() {
    if (downloadStore.variant) {
      downloadStore.download(downloadStore.variant).catch(() => {});
    }
  }

  function handleDismiss() {
    downloadStore.clearError();
  }
</script>

{#if visible}
  <div
    class="download-banner"
    class:completed={downloadStore.isCompleted}
    class:error={downloadStore.error !== null}
    role="region"
    aria-label="Model download status"
  >
    <div class="banner-content">
      <div class="banner-header">
        <div class="banner-title">
          {#if downloadStore.isDownloading}
            <span class="pulse-icon" aria-hidden="true">
              <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2">
                <path d="M12 15V3m0 12l-4-4m4 4l4-4M2 17l.621 2.485A2 2 0 004.561 21h14.878a2 2 0 001.94-1.515L22 17" />
              </svg>
            </span>
            <span class="label">Downloading <strong>{downloadStore.displayName || 'Model'}</strong></span>
          {:else if downloadStore.isCompleted}
            <span class="success-icon" aria-hidden="true">✓</span>
            <span class="label"><strong>{downloadStore.displayName || 'Model'}</strong> downloaded & ready</span>
          {:else if downloadStore.error}
            <span class="error-icon" aria-hidden="true">!</span>
            <span class="label">Download failed for <strong>{downloadStore.displayName || 'Model'}</strong></span>
          {/if}
        </div>

        <div class="banner-stats">
          {#if downloadStore.isDownloading}
            <span class="pct">{downloadStore.pct.toFixed(1)}%</span>
            <span class="mb">{downloadStore.downloadedMB} / {downloadStore.totalMB} MB</span>
          {:else if downloadStore.error}
            <button class="retry-btn" onclick={handleRetry}>Retry</button>
            <button class="dismiss-btn" onclick={handleDismiss} title="Dismiss">×</button>
          {/if}
        </div>
      </div>

      {#if downloadStore.isDownloading || downloadStore.isCompleted}
        <div class="progress-track" aria-hidden="true">
          <div
            class="progress-fill"
            style:width="{downloadStore.isCompleted ? 100 : downloadStore.pct}%"
          ></div>
        </div>
      {/if}

      {#if downloadStore.error}
        <div class="error-sub">{downloadStore.error}</div>
      {/if}
    </div>
  </div>
{/if}

<style>
  .download-banner {
    background: rgba(18, 14, 28, 0.95);
    border: 1px solid var(--accent-glow, rgba(138, 92, 246, 0.3));
    border-radius: 12px;
    padding: 10px 14px;
    margin-bottom: 16px;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.35);
    backdrop-filter: blur(12px);
    -webkit-backdrop-filter: blur(12px);
    animation: banner-in 220ms ease-out;
  }
  .download-banner.completed {
    border-color: var(--ok, #10b981);
    background: rgba(10, 26, 20, 0.95);
  }
  .download-banner.error {
    border-color: var(--danger, #ef4444);
    background: rgba(28, 12, 16, 0.95);
  }

  @keyframes banner-in {
    from {
      opacity: 0;
      transform: translateY(-8px);
    }
    to {
      opacity: 1;
      transform: translateY(0);
    }
  }

  .banner-content {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .banner-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }

  .banner-title {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
    color: var(--text, #f3f4f6);
  }

  .pulse-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    color: var(--accent, #a78bfa);
    animation: icon-bounce 1.4s infinite ease-in-out;
  }
  @keyframes icon-bounce {
    0%, 100% { transform: translateY(0); }
    50% { transform: translateY(3px); }
  }

  .success-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 18px;
    height: 18px;
    border-radius: 50%;
    background: var(--ok, #10b981);
    color: #000;
    font-weight: bold;
    font-size: 11px;
  }

  .error-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 18px;
    height: 18px;
    border-radius: 50%;
    background: var(--danger, #ef4444);
    color: #fff;
    font-weight: bold;
    font-size: 11px;
  }

  .label strong {
    color: #fff;
  }

  .banner-stats {
    display: flex;
    align-items: center;
    gap: 10px;
    font-family: ui-monospace, 'JetBrains Mono', monospace;
    font-size: 12px;
  }

  .pct {
    color: var(--accent, #a78bfa);
    font-weight: 600;
  }

  .mb {
    color: var(--muted, #9ca3af);
    font-size: 11px;
  }

  .progress-track {
    height: 6px;
    width: 100%;
    background: rgba(255, 255, 255, 0.08);
    border-radius: 4px;
    overflow: hidden;
    position: relative;
  }

  .progress-fill {
    height: 100%;
    background: linear-gradient(90deg, var(--accent, #8b5cf6), #ec4899);
    border-radius: 4px;
    transition: width 150ms ease-out;
    box-shadow: 0 0 10px rgba(167, 139, 250, 0.5);
  }
  .completed .progress-fill {
    background: linear-gradient(90deg, #10b981, #34d399);
    box-shadow: 0 0 10px rgba(16, 185, 129, 0.5);
  }

  .error-sub {
    font-size: 11.5px;
    color: var(--danger, #ef4444);
    font-family: ui-monospace, 'JetBrains Mono', monospace;
    word-break: break-word;
  }

  .retry-btn {
    background: var(--danger, #ef4444);
    color: #fff;
    border: none;
    border-radius: 6px;
    padding: 3px 10px;
    font-size: 11px;
    font-weight: 500;
    cursor: pointer;
    transition: opacity 120ms ease;
  }
  .retry-btn:hover {
    opacity: 0.9;
  }

  .dismiss-btn {
    background: transparent;
    border: none;
    color: var(--muted, #9ca3af);
    font-size: 16px;
    line-height: 1;
    cursor: pointer;
    padding: 0 4px;
  }
  .dismiss-btn:hover {
    color: var(--text, #fff);
  }
</style>
