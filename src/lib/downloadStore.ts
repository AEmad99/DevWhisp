import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { downloadModel as apiDownloadModel } from './api';

export interface ModelDownloadPayload {
  variant: string;
  pct: number;
  downloadedMB: number;
  totalMB: number;
  error?: string | null;
}

export function formatVariantDisplayName(variant: string): string {
  switch (variant) {
    case 'whisper-tiny-en':
      return 'Whisper Tiny';
    case 'whisper-base-en':
      return 'Whisper Base';
    case 'whisper-small-en':
      return 'Whisper Small';
    case 'whisper-medium-en':
      return 'Whisper Medium';
    case 'whisper-large-v3':
      return 'Whisper Large-v3';
    case 'whisper-distil-large-v3':
      return 'Distil-Whisper Large-v3';
    case 'moonshine-tiny':
      return 'Moonshine Tiny';
    default:
      return variant;
  }
}

class DownloadStore {
  variant = $state<string>('');
  displayName = $state<string>('');
  pct = $state<number>(0);
  downloadedMB = $state<number>(0);
  totalMB = $state<number>(0);
  isDownloading = $state<boolean>(false);
  isCompleted = $state<boolean>(false);
  error = $state<string | null>(null);

  private unlisten: UnlistenFn | null = null;
  private dismissTimer: number | null = null;

  init() {
    if (this.unlisten) return;

    try {
      listen<ModelDownloadPayload>('model-download-progress', (event) => {
        const payload = event.payload;
        if (!payload) return;

        if (this.dismissTimer !== null) {
          clearTimeout(this.dismissTimer);
          this.dismissTimer = null;
        }

        const pct = Math.min(100, Math.max(0, Math.round((payload.pct || 0) * 10) / 10));
        const hasError = Boolean(payload.error);
        const isDone = pct >= 100 && !hasError;

        this.variant = payload.variant || '';
        this.displayName = formatVariantDisplayName(payload.variant);
        this.pct = pct;
        this.downloadedMB = Math.max(0, Math.round(payload.downloadedMB || 0));
        this.totalMB = Math.max(0, Math.round(payload.totalMB || 0));
        this.error = payload.error || null;

        if (hasError) {
          this.isDownloading = false;
          this.isCompleted = false;
        } else if (isDone) {
          this.isDownloading = false;
          this.isCompleted = true;
          this.dismissTimer = window.setTimeout(() => {
            this.isCompleted = false;
            this.dismissTimer = null;
          }, 4000);
        } else {
          this.isDownloading = true;
          this.isCompleted = false;
        }
      })
        .then((fn) => {
          this.unlisten = fn;
        })
        .catch(() => {});
    } catch {
      /* Not in Tauri runtime */
    }
  }

  async download(variant: string): Promise<string> {
    if (this.dismissTimer !== null) {
      clearTimeout(this.dismissTimer);
      this.dismissTimer = null;
    }
    this.variant = variant;
    this.displayName = formatVariantDisplayName(variant);
    this.pct = 0;
    this.downloadedMB = 0;
    this.totalMB = 0;
    this.isDownloading = true;
    this.isCompleted = false;
    this.error = null;

    try {
      const path = await apiDownloadModel(variant);
      this.isDownloading = false;
      this.isCompleted = true;
      this.dismissTimer = window.setTimeout(() => {
        this.isCompleted = false;
        this.dismissTimer = null;
      }, 4000);
      return path;
    } catch (err: any) {
      this.isDownloading = false;
      this.isCompleted = false;
      this.error = typeof err === 'string' ? err : err?.message || 'Download failed';
      throw err;
    }
  }

  clearError() {
    this.error = null;
  }
}

export const downloadStore = new DownloadStore();
