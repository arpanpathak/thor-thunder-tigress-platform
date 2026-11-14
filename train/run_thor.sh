#!/usr/bin/env bash
#
# run_thor.sh: the overnight LoRA run on the Thor, in bf16, in the ~/finetune venv.
#
#   train/run_thor.sh 26b data/train.jsonl data/agent.jsonl
#   train/run_thor.sh 31b data/train.jsonl -- --epochs 2 --rank 32
#
# The first argument picks the model: 26b is Gemma 4 26B-A4B (MoE), 31b is Gemma 4 31B
# (dense). The JSONL files follow it; anything after `--` goes to lora.py unchanged.
#
# Window rule: a training run holds the GPU and most of the 128 GB of shared memory for
# hours, and the served models need the same memory. Long GPU jobs run on the Thor only
# inside a time window the owner of the Thor has agreed to, usually overnight, with serving
# paused for that window. This script never stops or starts a service. It checks free
# memory, refuses to start when there is not enough, and prints the commands that pause
# and resume serving, for a person to run.
#
# The run checkpoints every 50 steps and always resumes from the newest checkpoint, so a
# window that ends early loses at most 50 steps; run the same command in the next window.

set -euo pipefail

models="$HOME/models/hf"
venv="$HOME/finetune"
site="$venv/lib/python3.12/site-packages"
here="$(cd "$(dirname "$0")" && pwd)"

# The LLM engines that hold memory while serving. The agent and the web pages are small and
# stay up.
engines=(thor-edge-llm thor-qwen36-temp thor-chat)

# Free memory each model needs, in GB (estimates). The bf16 weights take 2 bytes per
# parameter: 49 GB for 26B-A4B and 59 GB for 31B on disk. The logits of a 4,096-token
# example over the 262,144-token vocabulary, with their gradients, take about 16 GB (measured
# on a 2-layer model with the real vocabulary). Activations under gradient checkpointing,
# LoRA weights and optimizer state add the rest.
case "${1:-}" in
    26b) model="$models/gemma-4-26B-A4B-it" need_gb=76 ;;
    31b) model="$models/gemma-4-31B-it" need_gb=88 ;;
    *) echo "usage: $0 26b|31b DATA.jsonl... [-- lora.py options]" >&2; exit 2 ;;
esac
name="$1"
shift

data=()
while [ $# -gt 0 ] && [ "$1" != "--" ]; do
    data+=("$1")
    shift
done
[ "${1:-}" = "--" ] && shift

if [ ${#data[@]} -eq 0 ]; then
    echo "no training files given" >&2
    exit 2
fi

# Every weight shard the index names must be on disk; a download in progress fails here.
missing=0
index="$model/model.safetensors.index.json"
for shard in $(grep -o 'model-[0-9]*-of-[0-9]*\.safetensors' "$index" | sort -u); do
    [ -f "$model/$shard" ] || missing=$((missing + 1))
done

if [ "$missing" -gt 0 ]; then
    echo "$model is missing $missing weight shards; wait for the download to finish" >&2
    exit 1
fi

active=()
for unit in "${engines[@]}"; do
    if systemctl --user is-active --quiet "$unit"; then
        active+=("$unit")
    fi
done

free_gb="$(awk '/MemAvailable/ { printf "%d", $2 / 1048576 }' /proc/meminfo)"
echo "free memory: $free_gb GB; $name needs about $need_gb GB"

if [ "$free_gb" -lt "$need_gb" ]; then
    echo "not enough free memory. Inside the agreed window, pause serving with:" >&2
    echo "    systemctl --user stop ${active[*]:-${engines[*]}}" >&2
    echo "then run this command again." >&2
    exit 1
fi

echo "when training ends, resume serving with:"
echo "    systemctl --user start ${active[*]:-thor-edge-llm thor-qwen36-temp}"

output_dir="runs/lora-$name"
mkdir -p "$output_dir"

# The Jetson torch wheel loads NVPL LAPACK and CUDA 13 libraries from the venv.
export LD_LIBRARY_PATH="$site/nvpl/lib:$site/nvidia/cu13/lib:${LD_LIBRARY_PATH:-}"
export PYTORCH_CUDA_ALLOC_CONF=expandable_segments:True

"$venv/bin/python" "$here/lora.py" \
    --model "$model" \
    --precision bf16 \
    --data "${data[@]}" \
    --output-dir "$output_dir" \
    --resume \
    "$@" 2>&1 | tee -a "$output_dir/train.log"
