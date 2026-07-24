/**
 * DevWhisp IPC surface — single source of truth for the frontend ↔ backend
 * bridge. Components MUST import from here instead of calling `invoke()`
 * directly so that:
 *   - the command name is typed (typo-proof)
 *   - the payload/response shape is documented in one place
 *   - errors come back as a structured `IpcError` discriminant, not a stringified
 *     mystery blob from Tauri
 *
 * Backend commands live in `src-tauri/src/ipc.rs`. Keep this file in sync
 * when commands are added/renamed.
 */

import { invoke } from '@tauri-apps/api/core';

// ---------------------------------------------------------------------------
// Discriminated-union error type
// ---------------------------------------------------------------------------

/**
 * All IPC failures flow through this type. `kind` is the discriminant.
 *   - `ipc`: the Tauri runtime itself rejected the call (command missing,
 *            serialization mismatch, etc.)
 *   - `backend`: the Rust handler ran and returned `Err(String)`
 *   - `network`: a transient I/O / transport failure
 *   - `unknown`: anything else — preserved raw for debugging
 */
export type IpcError =
  | { kind: 'ipc'; message: string; command: string }
  | { kind: 'backend'; message: string; command: string }
  | { kind: 'network'; message: string; command: string }
  | { kind: 'unknown'; message: string; command: string };

export function formatIpcError(err: IpcError): string {
  switch (err.kind) {
    case 'ipc':
      return `IPC error (${err.command}): ${err.message}`;
    case 'backend':
      return err.message;
    case 'network':
      return `Network error (${err.command}): ${err.message}`;
    case 'unknown':
      return `Unexpected error (${err.command}): ${err.message}`;
  }
}

/**
 * Wrap a Tauri `invoke<T>(...)` call. Normalizes the various failure modes
 * into a typed `IpcError`. The string returned by the backend (e.g. via
 * `Result<T, String>`) is rethrown as `kind: 'backend'`.
 */
export async function invokeOrThrow<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (raw) {
    // Tauri surfaces backend errors as JS strings (`Err("msg")` → `throw "msg"`).
    if (typeof raw === 'string') {
      throw { kind: 'backend', message: raw, command } satisfies IpcError;
    }
    if (raw instanceof Error) {
      const msg = raw.message || String(raw);
      if (/failed to invoke/i.test(msg) || /not found/i.test(msg)) {
        throw { kind: 'ipc', message: msg, command } satisfies IpcError;
      }
      throw { kind: 'network', message: msg, command } satisfies IpcError;
    }
    throw { kind: 'unknown', message: String(raw), command } satisfies IpcError;
  }
}

// ---------------------------------------------------------------------------
// Shared types — mirror Rust structs in src-tauri/src/ipc.rs / history/
// ---------------------------------------------------------------------------

export type AppInfo = {
  name: string;
  version: string;
  phase: string;
};

export type ModelStatus = {
  variant: string;
  /** Short UI label, e.g. "Base", "Distil-Large". */
  displayName: string;
  /** One-line description for the model picker. */
  description: string;
  ready: boolean;
  path: string;
  fileSizeMb: number;
  expectedSizeMb: number;
};

/** BridgeVoice-recommended default for general dictation. */
export const RECOMMENDED_MODEL = 'whisper-base-en';

export type HistoryEntry = {
  id: number;
  text: string;
  /** Unix epoch milliseconds. */
  created_at: number;
  duration_ms: number | null;
  source: string | null;
  language: string | null;
};

export type DictEntry = {
  from: string;
  to: string;
};

// ---------------------------------------------------------------------------
// App + STT commands (existing, pre-track C)
// ---------------------------------------------------------------------------

export const ping = (): Promise<string> => invokeOrThrow<string>('ping');

export const getAppInfo = (): Promise<AppInfo> =>
  invokeOrThrow<AppInfo>('get_app_info');

export const startListening = (): Promise<void> =>
  invokeOrThrow<void>('start_listening');

export const stopListening = (): Promise<number[]> =>
  invokeOrThrow<number[]>('stop_listening');

export const isListening = (): Promise<boolean> =>
  invokeOrThrow<boolean>('is_listening');

export const transcribeBuffer = (samples: number[]): Promise<string> =>
  invokeOrThrow<string>('transcribe_buffer', { samples });

export const getModelStatus = (): Promise<ModelStatus> =>
  invokeOrThrow<ModelStatus>('get_model_status');

export const downloadModel = (variant: string): Promise<string> =>
  invokeOrThrow<string>('download_model', { variant });

export const setActiveModel = (variant: string): Promise<void> =>
  invokeOrThrow<void>('set_active_model', { variant });

export const listModelStatuses = (): Promise<ModelStatus[]> =>
  invokeOrThrow<ModelStatus[]>('list_model_statuses');

// ---------------------------------------------------------------------------
// History commands (added by track C)
// ---------------------------------------------------------------------------

export const listHistory = (limit = 100, offset = 0): Promise<HistoryEntry[]> =>
  invokeOrThrow<HistoryEntry[]>('list_history', { limit, offset });

export const searchHistory = (query: string, limit = 50): Promise<HistoryEntry[]> =>
  invokeOrThrow<HistoryEntry[]>('search_history', { query, limit });

export const deleteHistoryEntry = (id: number): Promise<boolean> =>
  invokeOrThrow<boolean>('delete_history_entry', { id });

export const clearHistory = (): Promise<number> =>
  invokeOrThrow<number>('clear_history');

/**
 * History auto-prune retention window, in days.
 *   - `n >= 1`  → rows older than `n` days are deleted automatically.
 *   - `null`    → "Never" (auto-prune disabled). Also returned on a fresh
 *                 install where the key is unset; the UI should treat `null`
 *                 as its default (2 days) in that case.
 */
export type HistoryRetentionDays = number | null;

export const getHistoryRetentionDays = (): Promise<HistoryRetentionDays> =>
  invokeOrThrow<HistoryRetentionDays>('get_history_retention_days');

export const setHistoryRetentionDays = (
  days: HistoryRetentionDays,
): Promise<void> =>
  invokeOrThrow<void>('set_history_retention_days', { days });

// ---------------------------------------------------------------------------
// Dictionary commands (added by track C)
// ---------------------------------------------------------------------------

export const getDictionary = (): Promise<DictEntry[]> =>
  invokeOrThrow<DictEntry[]>('get_dictionary');

export const addDictionaryEntry = (from: string, to: string): Promise<DictEntry[]> =>
  invokeOrThrow<DictEntry[]>('add_dictionary_entry', { from, to });

export const removeDictionaryEntry = (from: string): Promise<DictEntry[]> =>
  invokeOrThrow<DictEntry[]>('remove_dictionary_entry', { from });

// ---------------------------------------------------------------------------
// Recording mode + re-inject
// ---------------------------------------------------------------------------

export type RecordingMode = 'push-to-talk' | 'toggle' | 'vad';

export const getRecordingMode = (): Promise<RecordingMode> =>
  invokeOrThrow<RecordingMode>('get_recording_mode');

export const setRecordingMode = (mode: RecordingMode): Promise<void> =>
  invokeOrThrow<void>('set_recording_mode', { mode });

export const getVadSilenceMs = (): Promise<number> =>
  invokeOrThrow<number>('get_vad_silence_ms');

export const setVadSilenceMs = (ms: number): Promise<void> =>
  invokeOrThrow<void>('set_vad_silence_ms', { ms });

/** Paste a previous transcription into the focused app again. */
export const reinjectText = (text: string): Promise<void> =>
  invokeOrThrow<void>('reinject_text', { text });

// ---------------------------------------------------------------------------
// Text-formatting options (persisted; read by the transcription pipeline)
// ---------------------------------------------------------------------------

export type FormatOptions = {
  autoCapitalize: boolean;
  appendSpace: boolean;
  pasteUppercase: boolean;
};

export const getFormatOptions = (): Promise<FormatOptions> =>
  invokeOrThrow<FormatOptions>('get_format_options');

export const setFormatOptions = (
  autoCapitalize: boolean,
  appendSpace: boolean,
  pasteUppercase: boolean,
): Promise<void> =>
  invokeOrThrow<void>('set_format_options', {
    autoCapitalize,
    appendSpace,
    pasteUppercase,
  });

// Acceleration (GPU/CPU)
export type AccelerationInfo = { mode: string; detected: string; inUse: string };

export const getAccelerationInfo = (): Promise<AccelerationInfo> =>
  invokeOrThrow<AccelerationInfo>('get_acceleration_info');

export const setAccelerationMode = (mode: string): Promise<void> =>
  invokeOrThrow<void>('set_acceleration_mode', { mode });

export const listAudioDevices = (): Promise<string[]> =>
  invokeOrThrow<string[]>('list_audio_devices');

export const getSelectedAudioDevice = (): Promise<string | null> =>
  invokeOrThrow<string | null>('get_selected_audio_device');

export const setSelectedAudioDevice = (name: string): Promise<void> =>
  invokeOrThrow<void>('set_selected_audio_device', { name });

// ---------------------------------------------------------------------------
// Hotkey rebinding
// ---------------------------------------------------------------------------

/**
 * Canonical display form of the currently registered hotkey,
 * e.g. `"Ctrl+Shift+Space"` or `"F8"`. Always returns a valid string.
 */
export const getHotkey = (): Promise<string> =>
  invokeOrThrow<string>('get_hotkey');

/**
 * Set the global hotkey from a spec string. Accepted forms:
 *   - "F8", "F1".."F24"
 *   - "Ctrl+Shift+Space", "Ctrl+Alt+F9", "Alt+Space"
 *   - Named keys: Space, Enter, Tab, Escape, Backspace, Insert, Delete,
 *     Home, End, PageUp, PageDown, CapsLock, ScrollLock, Arrow keys
 *   - Modifiers: Ctrl/Control, Shift, Alt/Option, Meta/Win/Cmd/Super
 *
 * On success returns the canonical display form. On failure throws an
 * `IpcError` with `kind: 'backend'` and the old binding is preserved.
 */
export const setHotkey = (spec: string): Promise<string> =>
  invokeOrThrow<string>('set_hotkey', { spec });

/**
 * One predefined hotkey the user can pick from in the Settings UI. The
 * Settings hotkey picker in 0.1.3 uses this list instead of a free-form
 * text field because the underlying parser previously had a bug that
 * mapped every single-char key to `KeyA` regardless of input.
 */
export interface PredefinedHotkey {
  /** Parseable form, e.g. "Ctrl+Shift+Space". */
  spec: string;
  /** Canonical display form, e.g. "Ctrl+Shift+Space". */
  label: string;
  /** Short helper text shown beneath the picker item. */
  description: string;
}

/**
 * Return the curated list of predefined hotkeys. The list is short and
 * hand-picked — these are the bindings that almost always work and rarely
 * conflict with other apps.
 */
export const listPredefinedHotkeys = (): Promise<PredefinedHotkey[]> =>
  invokeOrThrow<PredefinedHotkey[]>('list_predefined_hotkeys');

// ---------------------------------------------------------------------------
// Pill customization (size + position preset)
// ---------------------------------------------------------------------------

export const setPillSize = (
  width: number,
  height: number,
): Promise<void> =>
  invokeOrThrow<void>('set_pill_size', { width, height });

export const getPillSize = (): Promise<[number, number]> =>
  invokeOrThrow<[number, number]>('get_pill_size');

export type PillPositionPreset =
  | 'top-left'
  | 'top-right'
  | 'bottom-left'
  | 'bottom-right'
  | 'center';

export const setPillPositionPreset = (
  preset: PillPositionPreset,
): Promise<void> =>
  invokeOrThrow<void>('set_pill_position_preset', { preset });