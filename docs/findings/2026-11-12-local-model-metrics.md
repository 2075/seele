# Local model metrics (llama.cpp / Ollama)

- `~/workspace/llama.cpp` is an old, unbuilt source checkout (`8cb508d0`, 2024-06-24). Nothing runs from it.
- Local inference runs on **Ollama 0.31.1** (`:11434`). Ollama starts one bundled `llama-server` runner per loaded model, bound to `127.0.0.1` on a random port (e.g. 42773, 36641) and started without `--metrics`.
- Runner `/metrics` → **501** (Prometheus endpoint disabled). Runner `/slots` → **200**, giving per-slot `is_processing`, `n_ctx`, prompt-token counts. These are internal and unstable: ports change whenever a model reloads.
- Ollama `/api/ps` → loaded models with `size_vram`, `context_length`, `expires_at`. This is stable and good for "which local models are hot".
- Loaded at probe time: `qwen3.5:4b` (Q4_K_M, fully in VRAM), `qwen3-embedding:0.6b`.

## Implications for seele
- "Local model busy" signal: poll `/api/ps` (stable), and optionally find runner ports via `pgrep -af llama-server` and poll `/slots` for `is_processing` (best effort).
- Tokens/sec and request rates for local vs remote are better taken from LiteLLM (it sees every call), not from the runners.
- If real Prometheus metrics are wanted, run a standalone `llama-server --metrics` (newer build) behind LiteLLM instead of Ollama.
