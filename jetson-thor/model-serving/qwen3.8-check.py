#!/usr/bin/env python3
"""Measure streaming speed and check both agent APIs, using only the stdlib."""

import argparse
import json
import time
import urllib.request


def request(base, path, body):
    """Open a JSON request without sending credentials to the trial server."""
    return urllib.request.urlopen(
        urllib.request.Request(
            base + path,
            data=json.dumps(body).encode(),
            headers={"Content-Type": "application/json", "anthropic-version": "2023-06-01"},
        ),
        timeout=600,
    )


def post(base, path, body):
    """Read one complete JSON response."""
    with request(base, path, body) as response:
        return json.load(response)


def speed(base, model, thinking):
    """Report first-token latency, generation interval, and end-to-end rate."""
    body = {
        "model": model,
        "messages": [{"role": "user", "content": "Write a detailed tutorial on Rust ownership, borrowing, lifetimes and error handling, with code examples."}],
        "max_tokens": 1024,
        "temperature": 1.0 if thinking else 0.7,
        "top_p": 0.95 if thinking else 0.8,
        "top_k": 20,
        "presence_penalty": 0.0 if thinking else 1.5,
        "chat_template_kwargs": {"enable_thinking": thinking},
        "stream": True,
        "stream_options": {"include_usage": True},
    }
    started = time.perf_counter()
    first = last = None
    usage = None
    finish = None
    with request(base, "/v1/chat/completions", body) as response:
        for raw in response:
            if not raw.startswith(b"data: ") or raw.strip() == b"data: [DONE]":
                continue
            chunk = json.loads(raw[6:])
            if chunk.get("usage"):
                usage = chunk["usage"]
            for choice in chunk.get("choices", []):
                delta = choice.get("delta", {})
                if any(delta.get(key) for key in ("content", "reasoning_content", "reasoning")):
                    last = time.perf_counter()
                    if first is None:
                        first = last
                if choice.get("finish_reason"):
                    finish = choice["finish_reason"]
    elapsed = time.perf_counter() - started
    if not usage or first is None or last <= first:
        raise RuntimeError("Missing streamed tokens or usage; cannot measure throughput")
    tokens = usage["completion_tokens"]
    return {
        "thinking": thinking,
        "usage": usage,
        "finish_reason": finish,
        "ttft_seconds": round(first - started, 3),
        "elapsed_seconds": round(elapsed, 3),
        "decode_tokens_per_second_approx": round((tokens - 1) / (last - first), 2),
        "end_to_end_tokens_per_second": round(tokens / elapsed, 2),
    }


def tools(base, model):
    """Check parsed tool arguments and a second turn containing the result."""
    messages = [{"role": "user", "content": "Call lookup_trial with key thor. Then report the exact value returned by the tool."}]
    body = {
        "model": model, "messages": messages, "max_tokens": 1024,
        "chat_template_kwargs": {"enable_thinking": False},
        "tools": [{"type": "function", "function": {
            "name": "lookup_trial", "description": "Look up a trial value.",
            "parameters": {"type": "object", "properties": {"key": {"type": "string"}}, "required": ["key"]},
        }}],
        "tool_choice": "auto",
    }
    assistant = post(base, "/v1/chat/completions", body)["choices"][0]["message"]
    calls = assistant.get("tool_calls", [])
    if len(calls) != 1:
        raise RuntimeError(f"Expected one parsed tool call: {assistant}")
    call = calls[0]
    if call["function"]["name"] != "lookup_trial" or json.loads(call["function"]["arguments"]) != {"key": "thor"}:
        raise RuntimeError(f"Incorrect tool arguments: {call}")
    messages.extend([assistant, {"role": "tool", "tool_call_id": call["id"], "content": "trial-value-7391"}])
    body["tool_choice"] = "none"
    answer = post(base, "/v1/chat/completions", body)["choices"][0]["message"]["content"]
    if "trial-value-7391" not in answer:
        raise RuntimeError(f"Tool result lost on second turn: {answer}")
    return {"tool_round_trip": "passed", "answer": answer}


def main():
    """Run API probes and repeated sequential speed samples."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", default="http://127.0.0.1:8000")
    parser.add_argument("--runs", type=int, default=3)
    args = parser.parse_args()
    model = "Qwen3.8-Flash-Next"
    with urllib.request.urlopen(args.base + "/v1/models", timeout=10) as response:
        print(json.dumps({"models": json.load(response)}), flush=True)
    for thinking in (False, True):
        for run in range(args.runs):
            print(json.dumps({"run": run + 1, **speed(args.base, model, thinking)}), flush=True)
    print(json.dumps(tools(args.base, model)), flush=True)
    answer = post(args.base, "/v1/messages", {
        "model": model, "max_tokens": 1024,
        "messages": [{"role": "user", "content": "Reply with exactly: thor-ready"}],
    })
    if answer.get("type") != "message" or not answer.get("content"):
        raise RuntimeError(f"Invalid Anthropic Messages response: {answer}")
    print(json.dumps({"anthropic_messages": answer}), flush=True)


if __name__ == "__main__":
    main()
