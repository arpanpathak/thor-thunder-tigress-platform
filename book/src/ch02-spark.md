<img class="plate" src="art/ch07.svg" alt="A tigress beside a dial whose needle points into the green range">

# The checker: spark

`spark` scores an answer on three things.

**The five Rust rules**

1. No `unwrap()` or `expect()`, not even in tests.
2. Errors are a hand-written enum implementing `Display` and `std::error::Error`.
3. Every `pub` item has a `///` doc comment.
4. No comments inside function bodies.
5. No index loops like `for i in 0..n`.

A rule only counts where it applies: code that handles no errors is "not
applicable" for rule 2, not a pass.

**Slop phrases** in the prose, from the slop taxonomy: filler openers, fake
importance, hedging, wrap-ups, rhythm tricks. Code is ignored.

**False claims**: prose that says the rules are followed while the code breaks
them.

```bash
spark rs crates                                   # check Rust files
spark text answer.md                              # slop in a text file
spark score RUN.jsonl --field text                # score a file of model answers
```

## Baseline

| Model | All 5 rules | No unwrap | No body comments | False claim |
|---|---|---|---|---|
| Qwen3.5-9B (llama.cpp Q4) | 0% | 14% | 0% | 41% |
| Qwen2.5-Coder-7B (TensorRT-LLM INT4) | 4% | 21% | 8% | 33% |

**Not checked yet:** whether the code compiles or its tests pass.
