"""Asks a model the same 20 Rust tasks and checks the answers, to compare models.

    python3 compare.py MODEL_ID OUT.jsonl [SERVER]
    spark score OUT.jsonl --field text --label NAME

SERVER defaults to the agent, http://127.0.0.1:8080.

For each task: tokens/s from the server's own timing when given, otherwise
measured here; whether the first Rust block compiles and its tests pass with
`rustc --edition 2024 --test`. spark scores the answers afterwards.
"""

import json
import re
import subprocess
import sys
import tempfile
import time
import urllib.request
from pathlib import Path

KEY = (Path.home() / ".config/thor-chat/api-key").read_text().strip()
SERVER = sys.argv[3] if len(sys.argv) > 3 else "http://127.0.0.1:8080"

TASKS = [
    "Write a Rust function that returns the median of a slice of f64, with tests.",
    "Write a Rust function that parses `key = value` lines into a HashMap<String, String>, skipping blank lines and # comments, with tests.",
    "Write a Rust function that checks whether a string of brackets ()[]{} is balanced, with tests.",
    "Write a Rust function that returns the n-th Fibonacci number as u128 without recursion, with tests.",
    "Write a Rust function that splits a slice into chunks of at most n elements and returns their sums, with tests.",
    "Write a Rust function that counts word frequencies in a text and returns the top k words, with tests.",
    "Write a Rust struct for a fixed-capacity ring buffer of i32 with push and pop, with tests.",
    "Write a Rust function that parses a semantic version string like 1.2.3 into a struct, with a custom error type and tests.",
    "Write a Rust function that merges two sorted Vec<i32> into one sorted Vec, with tests.",
    "Write a Rust function that returns the longest common prefix of a list of strings, with tests.",
    "Write a Rust function that converts a Roman numeral string to u32, returning an error for invalid input, with tests.",
    "Write a Rust function that finds the two indices of numbers in a slice that add up to a target, with tests.",
    "Write a Rust function that run-length encodes a string, like aaabcc to a3b1c2, with tests.",
    "Write a Rust function that checks whether a u64 is prime, with tests.",
    "Write a Rust function that transposes a Vec<Vec<i32>> matrix, returning an error if rows differ in length, with tests.",
    "Write a Rust function that removes duplicate values from a Vec<i32> while keeping the first occurrence order, with tests.",
    "Write a Rust function that parses a CSV line with quoted fields into Vec<String>, with tests.",
    "Write a Rust function that computes the moving average of a slice with window size k, with tests.",
    "Write a Rust function that validates an IPv4 address string without using std::net, with tests.",
    "Write a Rust function that groups strings that are anagrams of each other, with tests.",
]


def ask(model, task):
    body = {"model": model, "messages": [{"role": "user", "content": task}], "max_tokens": 2048,
            "chat_template_kwargs": {"enable_thinking": False}}
    request = urllib.request.Request(SERVER + "/v1/chat/completions", data=json.dumps(body).encode(), method="POST")
    request.add_header("Content-Type", "application/json")
    request.add_header("Authorization", f"Bearer {KEY}")
    started = time.time()
    with urllib.request.urlopen(request, timeout=900) as response:
        answer = json.load(response)
    seconds = time.time() - started
    text = answer["choices"][0]["message"].get("content") or ""
    tokens = (answer.get("usage") or {}).get("completion_tokens") or 0
    timings = answer.get("timings") or {}
    speed = timings.get("predicted_per_second") or (tokens / seconds if seconds and tokens else 0)
    return text, tokens, seconds, speed


def first_rust_block(text):
    match = re.search(r"```rust[^\n]*\n(.*?)```", text, re.S)
    return match.group(1) if match else None


def compiles_and_passes(code):
    if code is None:
        return False, False
    with tempfile.TemporaryDirectory() as folder:
        source = Path(folder) / "answer.rs"
        source.write_text(code if "fn main" in code else code + "\nfn main() {}\n")
        built = subprocess.run(["rustc", "--edition", "2024", "--test", "-o", str(Path(folder) / "t"), str(source)],
                               capture_output=True, timeout=120)
        if built.returncode:
            return False, False
        ran = subprocess.run([str(Path(folder) / "t")], capture_output=True, timeout=60)
        return True, ran.returncode == 0


def main():
    model, out = sys.argv[1], Path(sys.argv[2])
    with out.open("w") as file:
        for number, task in enumerate(TASKS, 1):
            text, tokens, seconds, speed = ask(model, task)
            compiled, passed = compiles_and_passes(first_rust_block(text))
            line = {"task": number, "model": model, "text": text, "tokens": tokens, "seconds": round(seconds, 1),
                    "tok_s": round(speed, 1), "compiles": compiled, "tests_pass": passed}
            file.write(json.dumps(line) + "\n")
            file.flush()
            print(f"{number:2} {speed:6.1f} tok/s {tokens:5} tokens compiles={compiled} tests={passed}", flush=True)


if __name__ == "__main__":
    main()
