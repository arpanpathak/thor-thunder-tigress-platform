<img class="plate" src="art/ch01.svg" alt="Four tigresses in a row joined by arrows">

# Architecture

```text
 books, docs, chat export, own pairs ─► thor-hammer-trainer ─► data/train.jsonl
                                                                 │
                                         reinforcer (review) ◄───┤
                                         lasso (conversations) ◄─┘
 model answers ─► spark ─► scores
```

| Crate | Binary | Does |
|---|---|---|
| `thor-spark-safety-eval` | `spark` | scores answers for slop phrases and the five Rust rules |
| `thor-hammer-trainer` | `thor-hammer-trainer` | builds `data/train.jsonl` from licensed sources |
| `thor-tigress-reinforcer-frontend` | `reinforcer` | review page: mark slop, flag records |
| `thor-lasso-distiller` | `lasso` | turns book sections into conversations using a teacher model |

## Machines

| | yahboom | thor |
|---|---|---|
| Board | Jetson Orin NX 16 GB | Jetson AGX Thor, 128 GB |
| Role | development, data builds | models, training, shared chat |

Code is edited on yahboom and copied to the Thor with `thor-sync` (chapter 4).
