# thor-clients.sh: coding tools on the Thor's models, for bash.
#
#   source jetson-thor/clients/thor-clients.sh     # from ~/.bashrc, for example
#
# Every command asks the agent which models it serves when it starts, so nothing
# here names a model. Requests carry the key in ~/.config/thor-chat/api-key.
#
#   thor-models                  what the agent serves right now
#   claude-thor [-m NAME]        Claude Code; settings and history in ~/.claude-thor
#   opencode                     refreshes the "thor" provider in opencode.json first
#   deepcode-thor [-m NAME]      deepcode; ~/.deepcode/settings.json stays as it is
#   deepseek-thor [-m NAME]      DeepSeek Harness (dsh); its DeepSeek key stays local
#   openbatrangs --thor -m NAME  needs no wrapper
#
# An Edge-LLM engine takes prompts up to half its window and answers a longer one
# with a 413, so each tool is told that half as its context.
#
# THOR_URL      the agent's address
# THOR_CONTEXT  the context window when the server reports none (65536)
THOR_URL=${THOR_URL:-https://arpanpathak.taildb9a39.ts.net}
THOR_CLIENTS_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)

# In Claude Code's auto mode, each command needs approval from Anthropic's safety
# classifier. The Thor has no classifier, so every command in auto mode fails.
# On the first start, we copy an allow list of build, test, git and gh commands
# to ~/.claude-thor/settings.json. An existing settings file is not replaced.
thor-claude-settings() {
  local settings="$HOME/.claude-thor/settings.json"

  if [ -e "$settings" ]; then
    return
  fi

  mkdir -p "$HOME/.claude-thor"
  cp "$THOR_CLIENTS_DIR/claude-thor-settings.json" "$settings"
}

# One line per served model: its name, max_model_len (0 when unknown) and engine.
thor-served() {
  local key
  key=$(cat ~/.config/thor-chat/api-key 2>/dev/null)
  curl -sf -m 20 -H "Authorization: Bearer $key" "$THOR_URL/v1/models" 2>/dev/null |
    python3 -c 'import json,sys
for m in json.load(sys.stdin)["data"]:
    print(m["id"], m.get("max_model_len") or 0, m.get("owned_by", ""))'
}

thor-models() {
  local out
  if ! out=$(thor-served) || [ -z "$out" ]; then
    echo "cannot reach $THOR_URL" >&2
    echo "  the Thor may be down, or off the tailnet" >&2
    return 1
  fi
  printf '%s\n' "$out" | awk '{printf "  %-42s %-18s %s\n", $1, $3, ($2 > 0 ? $2 " ctx" : "")}'
}

# The longest prompt a served model takes: half the window for an Edge-LLM
# engine, the whole window otherwise.
thor-context() {
  thor-served | awk -v m="$1" '$1 == m { print ($3 == "tensorrt-edgellm" ? int($2 / 2) : $2) }'
}

# The first served model, or the one -m names.
thor-pick-model() {
  local served
  if ! served=$(thor-served) || [ -z "$served" ]; then
    echo "cannot reach $THOR_URL: no model list" >&2
    return 1
  fi
  if [ -z "$1" ]; then
    printf '%s\n' "$served" | head -1 | cut -d' ' -f1
    return
  fi
  printf '%s\n' "$served" | awk -v m="$1" '$1 == m { print $1; found = 1 } END { exit !found }' ||
    { echo "$1 is not served; served now:" >&2; printf '%s\n' "$served" | sed 's/^/  /' >&2; return 1; }
}

# Claude Code against the agent's Anthropic Messages API. Each Claude Code slot
# gets a served model found by name (Bolt for Opus and Fable, Qwen3.6 for Sonnet,
# the Nano for Haiku); a slot whose model is not served gets the starting model.
claude-thor() {
  local model="" served line context opus sonnet haiku
  case "${1:-}" in
    -m|--model) model="$2"; shift 2 ;;
    --model=*)  model="${1#--model=}"; shift ;;
  esac
  if ! served=$(thor-served) || [ -z "$served" ]; then
    echo "cannot reach $THOR_URL: no model list" >&2
    return 1
  fi

  pick() { printf '%s\n' "$served" | awk -v p="$1" 'tolower($1) ~ p { print $1; exit }'; }
  opus=$(pick bolt); sonnet=$(pick 'qwen3\\.6'); haiku=$(pick nano)
  [ -n "$model" ] || model=${opus:-$(printf '%s\n' "$served" | head -1 | cut -d' ' -f1)}
  line=$(printf '%s\n' "$served" | awk -v m="$model" '$1 == m')
  if [ -z "$line" ]; then
    echo "$model is not served; served now:" >&2
    printf '%s\n' "$served" | sed 's/^/  /' >&2
    return 1
  fi
  opus=${opus:-$model}; sonnet=${sonnet:-$model}; haiku=${haiku:-$model}

  ctx() { printf '%s\n' "$served" | awk -v m="$1" '$1 == m { print ($3 == "tensorrt-edgellm" ? int($2 / 2) : $2) }'; }
  describe() { echo "on the Thor, prompts up to $(ctx "$1") tokens"; }
  context=$(ctx "$model")
  [ "$context" -gt 0 ] 2>/dev/null || context=${THOR_CONTEXT:-65536}
  echo "claude-thor: $model, $context-token context (opus $opus, sonnet $sonnet, haiku $haiku)" >&2
  thor-claude-settings

  CLAUDE_CONFIG_DIR="$HOME/.claude-thor" \
  ANTHROPIC_BASE_URL="$THOR_URL" \
  ANTHROPIC_AUTH_TOKEN="$(cat ~/.config/thor-chat/api-key)" \
  ANTHROPIC_MODEL="$model" \
  ANTHROPIC_DEFAULT_FABLE_MODEL="$opus" \
  ANTHROPIC_DEFAULT_OPUS_MODEL="$opus" \
  ANTHROPIC_DEFAULT_SONNET_MODEL="$sonnet" \
  ANTHROPIC_DEFAULT_HAIKU_MODEL="$haiku" \
  ANTHROPIC_DEFAULT_FABLE_MODEL_NAME="$opus" \
  ANTHROPIC_DEFAULT_OPUS_MODEL_NAME="$opus" \
  ANTHROPIC_DEFAULT_SONNET_MODEL_NAME="$sonnet" \
  ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME="$haiku" \
  ANTHROPIC_DEFAULT_FABLE_MODEL_DESCRIPTION="$(describe "$opus")" \
  ANTHROPIC_DEFAULT_OPUS_MODEL_DESCRIPTION="$(describe "$opus")" \
  ANTHROPIC_DEFAULT_SONNET_MODEL_DESCRIPTION="$(describe "$sonnet")" \
  ANTHROPIC_DEFAULT_HAIKU_MODEL_DESCRIPTION="$(describe "$haiku")" \
  CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY=1 \
  CLAUDE_CODE_MAX_CONTEXT_TOKENS="$context" \
  claude "$@"
}

# OpenCode with a "thor" provider in ~/.config/opencode/opencode.json, created on
# first use. Its model list is rewritten from the served models each start; other
# settings, such as a baseURL through a tunnel, and per-model variants are kept.
opencode() {
  local served
  if served=$(thor-served) && [ -n "$served" ]; then
    printf '%s\n' "$served" | THOR_URL="$THOR_URL" python3 -c 'import json,os,sys
path = os.path.expanduser("~/.config/opencode/opencode.json")
config = json.load(open(path)) if os.path.exists(path) else {"$schema": "https://opencode.ai/config.json"}
provider = config.setdefault("provider", {}).setdefault("thor", {})
provider.setdefault("npm", "@ai-sdk/openai-compatible")
provider.setdefault("name", "Thor agent")
provider.setdefault("options", {"baseURL": os.environ["THOR_URL"] + "/v1",
                                "apiKey": "{file:~/.config/thor-chat/api-key}"})
old = provider.get("models", {})
models = {}
for line in sys.stdin.read().split("\n"):
    if not line.strip():
        continue
    name, context, owner = (line.split(" ") + ["", ""])[:3]
    entry = dict(old.get(name, {}))
    entry["name"] = f"{name} ({owner})" if owner else name
    entry.setdefault("variants", {
        "think": {"chat_template_kwargs": {"enable_thinking": True}},
        "off": {"chat_template_kwargs": {"enable_thinking": False}},
    })
    window = int(context) // 2 if owner == "tensorrt-edgellm" else int(context)
    if window > 0:
        entry["limit"] = {"context": window, "output": min(32768, window // 4)}
    else:
        entry.setdefault("limit", {"context": 65536, "output": 16384})
    models[name] = entry
provider["models"] = models
first = next(iter(models), None)
if first:
    config["model"] = config["small_model"] = "thor/" + first
os.makedirs(os.path.dirname(path), exist_ok=True)
json.dump(config, open(path, "w"), indent=2)'
  fi
  command opencode "$@"
}

# deepcode against the agent's OpenAI API. DEEPCODE_* variables override
# ~/.deepcode/settings.json and -s replaces the project settings, so the
# DeepSeek model and key there are left as they are.
deepcode-thor() {
  local model="" settings="$HOME/.deepcode/thor-settings.json"
  case "${1:-}" in
    -m|--model) model="$2"; shift 2 ;;
  esac
  model=$(thor-pick-model "$model") || return 1

  mkdir -p "$HOME/.deepcode"
  printf '{\n  "model": "%s",\n  "contextWindow": %s,\n  "thinkingEnabled": false\n}\n' \
    "$model" "$(thor-context "$model")" > "$settings"
  echo "deepcode-thor: $model, prompts up to $(thor-context "$model") tokens" >&2

  DEEPCODE_MODEL="$model" \
  DEEPCODE_BASE_URL="$THOR_URL/v1" \
  DEEPCODE_API_KEY="$(cat ~/.config/thor-chat/api-key)" \
  deepcode -s "$settings" "$@"
}

# DeepSeek Harness (dsh) against the agent's Anthropic Messages API. The key goes
# in THOR_API_KEY, so the key dsh keeps for DeepSeek's own API is never sent to
# the Thor. Every served model is offered in /model.
deepseek-thor() {
  local model="" patch="$HOME/.dsh/thor.patch.yml" name
  case "${1:-}" in
    -m|--model) model="$2"; shift 2 ;;
  esac
  model=$(thor-pick-model "$model") || return 1

  mkdir -p "$HOME/.dsh"
  {
    printf -- '- id: llm-deepseek\n  config:\n    apiKeyEnv: THOR_API_KEY\n    models:\n'
    for name in $(thor-served | cut -d' ' -f1); do
      printf '      - id: %s\n        name: %s (Thor)\n        contextWindow: %s\n' \
        "$name" "$name" "$(thor-context "$name")"
    done
    printf -- '- id: agent-default-model\n  config:\n    provider: deepseek-official\n    model: %s\n' "$model"
  } > "$patch"
  echo "deepseek-thor: $model, prompts up to $(thor-context "$model") tokens" >&2

  DEEPSEEK_BASE_URL="$THOR_URL" \
  THOR_API_KEY="$(cat ~/.config/thor-chat/api-key)" \
  dsh --profile terminal --patch "$patch" "$@"
}
