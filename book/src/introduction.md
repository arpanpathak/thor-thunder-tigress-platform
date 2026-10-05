# Introduction

<img class="cover" src="art/cover.png" alt="Book cover: the title Thor Thunder Tigress above a tigress in a winged helm holding a war hammer">

Thor Thunder Tigress fine-tunes a local language model on an NVIDIA Jetson so it
stops writing AI slop, in prose and in Rust.

## What exists

| Part | State |
|---|---|
| `spark`: scores answers for slop and five Rust rules | built |
| `thor-hammer-trainer`: builds the training set | built |
| `reinforcer`: review page for the training set | built |
| `lasso`: conversations from books, questions by a teacher model | built, not yet run |
| Jetson Thor: SSH, sync, local models, shared web chat | running |
| Fine-tuning and before/after scoring | planned |

## Repositories

| Repository | Contents |
|---|---|
| [thor-thunder-tigress-platform](https://github.com/arpanpathak/thor-thunder-tigress-platform) | the crates, this book, `jetson-thor/` |
| [local-copilot-codebuddy](https://github.com/arpanpathak/local-copilot-codebuddy) | terminal coding assistant |
| [openbatrangs](https://github.com/arpanpathak/openbatrangs) | agentic coding CLI for Ollama |
| [thor-sync](https://github.com/arpanpathak/thor-sync) | keeps project folders copied to the Jetson |

## Reading this book locally

```bash
mdbook serve book    # http://localhost:3000
```
