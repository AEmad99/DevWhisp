# Building DevWhisp — CPU and GPU

DevWhisp transcribes with whisper.cpp (via `whisper-rs`). The default build is
**CPU-only and runs on any PC**. GPU acceleration is an **opt-in build flag** —
the app uses the GPU when one is present and falls back to CPU otherwise.

## Default — CPU (works on any PC)

```bash
npm install
npm run tauri:build        # → src-tauri/target/release/bundle/nsis/
```

This is what ships in the one-click offline installer. The CPU path is tuned:

- **Multi-threaded** — uses up to 8 worker threads (was hard-pinned to 4).
- **Dynamic encoder context** (`audio_ctx`) — the whisper encoder normally
  processes a fixed 30 s window regardless of clip length; we scale it to the
  actual utterance (+margin), which is a large speedup for short voice
  commands without affecting accuracy (no real speech is ever truncated).
- **Warm start** — the model is preloaded at launch, so the first
  transcription is instant instead of paying the ~1 s load cost.

## GPU builds (opt-in, much faster)

GPU support is compiled into whisper.cpp, so it requires the matching SDK at
**build time**. Pick one:

### Vulkan — recommended (any GPU, CPU fallback in one binary)

Works with NVIDIA, AMD, and Intel GPUs, and **falls back to CPU** when no GPU
is available — so a single Vulkan build is portable *and* fast.

1. Install the [Vulkan SDK](https://vulkan.lunarg.com/) (provides `glslc`).
2. Build:
   ```bash
   npm run tauri:build:vulkan
   ```

### CUDA — NVIDIA only, maximum speed

Fastest on NVIDIA GPUs (e.g. an RTX 2060), but the resulting build needs the
CUDA runtime on the target machine, so it is **not** portable to non-NVIDIA
PCs. Use it for a personal/NVIDIA build.

1. Install the [CUDA Toolkit](https://developer.nvidia.com/cuda-downloads)
   (provides `nvcc`).
2. Build:
   ```bash
   npm run tauri:build:cuda
   ```

> The app auto-detects the GPU at runtime (`use_gpu` is enabled by the GPU
> features) and transparently falls back to CPU if the device is unavailable.
> No code or settings change is needed — only the build flag.

## Which should I use?

| Goal | Build |
|---|---|
| Ship to anyone, offline, no GPU assumptions | `tauri:build` (default, CPU) |
| Fast on my machine, still works if GPU absent | `tauri:build:vulkan` |
| Absolute max speed on an NVIDIA box | `tauri:build:cuda` |
