# Qwen3.8 Flash Next on the Thor

Serving since 2026-10-18 as the default model behind `thor-tigress-agent`
(chat page, `claude-thor`, `openbatrangs --thor`, opencode `thor`).

## What runs

- Engine: vLLM 0.28.1rc1 in NVIDIA's Thor image
  `ghcr.io/nvidia-ai-iot/vllm@sha256:512bf772…` (tag `qwen3.8-next-jetson-thor`).
  [The jetson-ai-lab page](https://www.jetson-ai-lab.com/models/qwen3-8-flash-next/)
  lists vLLM only; TensorRT-LLM and Edge-LLM have no `qwen4_exp` support.
- Checkpoint: `local-inference-lab/Qwen3.8-Flash-Next-NVFP4` at revision
  `ada4da32a583a78aa47299f45a70603c950490b8` (2026-09-08). The image's MTP draft
  at `/opt/qwen38-mtp-model` links into that revision's 34 shards. The newer
  revision `6c01ac3` has 41 shards; with it the draft fails with
  `FileNotFoundError: /opt/qwen38-mtp-model/model-00001-of-00034.safetensors`
  after the 17-minute weight load.
- Unit `thor-vllm` runs `thor-tigress-serve vllm`, which reads `VLLM_*` from
  `~/.config/thor-chat/env`. The agent gets `--model 127.0.0.1:8000` first, so
  vLLM also answers requests that name no model or one nobody serves.
- Context 262,144 tokens (the model's native window), 4 sequences at once,
  prefix caching, MTP with 3 draft tokens, `qwen3` reasoning parser,
  `qwen3_xml` tool parser.
- Chat template: the checkpoint's own, copied to
  `~/.cache/vllm/qwen38-chat-template.jinja` with `high` and `max` mapped to
  `xhigh`. The original accepts only `xhigh`, `medium` and `low` and answers
  Claude Code's default effort with 400.
- `MODEL_ON_STARTUP=false`: llama-server keeps running with nothing loaded.
  The Nano and Qwen3.8 do not fit together (see Memory).

## Measured on 2026-10-18

Single request, 1,024 generated tokens, `qwen3.8-check.py`:

| thinking | first token | decode |
|---|---|---|
| off | 0.19–0.27 s | 43–45 tok/s |
| on  | 0.22–0.88 s | 31–34 tok/s |

MTP acceptance on prose: 53 %, 26 %, 15 % for draft positions 1–3, mean
accepted length 1.95–2.24.

- Weight load from NVMe: about 16 minutes, then 218 s of profiling and CUDA
  graph capture.
- Memory: 104.6 GiB weights and runtime, KV cache 7.33 GiB (267,589 tokens),
  about 2 GiB of system memory left while serving. A large build on the Thor
  can trigger the OOM killer.
- Tool call round trip (OpenAI API): passed. `/v1/messages`: returns separate
  thinking and text blocks.
- Chat with `thor_web_search` and thinking, over the public address: 2
  searches, 4 page reads, cited answer, 37 s, 12,791 prompt tokens.
- `claude-thor -p` (no flags): wrote, compiled and ran a Rust file, correct
  output, 3 turns, 30 s.
- `openbatrangs --thor` (no flags): picked Qwen3.8 and 262,144 context,
  created a bounded queue crate, fixed one compile error and one failing
  doc-test, ended with 15 unit tests and 1 doc-test passing, clippy clean,
  17 steps, 3 min 26 s.

## Not done

- Not a TensorRT-LLM baseline; scores from it are not comparable with the
  project's Stage 0 numbers.
- Fewer MTP draft tokens (1 or 2) not measured; each try costs a 16-minute
  reload.
- The 2026-10-06 checkpoint (`6c01ac3`) needs a newer image; it was deleted
  from the cache (106.4 GB).

## Back to the Nano

```bash
systemctl --user disable --now thor-vllm
# in ~/.config/thor-chat/env: comment the VLLM_MODEL line, MODEL_ON_STARTUP=true
systemctl --user restart thor-chat thor-tigress-agent
```

`~/.config/thor-chat/env.before-vllm` is the env file as it was before.
