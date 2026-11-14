"""Fine-tune Gemma 4 with LoRA, merge an adapter into its base, or try a model on one prompt.

One script serves both models and both machines. `--model` picks Gemma 4 26B-A4B (MoE) or
Gemma 4 31B (dense), and `--precision` picks bf16 on the Thor or 4-bit QLoRA in Colab:

    python train/lora.py --model google/gemma-4-31B-it --precision 4bit \
        --data data/train.jsonl data/agent.jsonl --output-dir runs/lora-31b --resume
    python train/lora.py --merge --model google/gemma-4-31B-it \
        --adapter runs/lora-31b --output-dir runs/merged-31b
    python train/lora.py --generate --model google/gemma-4-31B-it --adapter runs/lora-31b

Training reads two kinds of JSONL lines. Agent runs and other chats are `{"tools", "messages"}`
in the OpenAI chat format. Records from `thor-hammer-trainer` are `{"instruction", "response",
"source", "origin"}` and become one user turn and one assistant turn. The loss counts only the
tokens the model writes itself: its text, its tool calls and the tokens that end its turn.
"""

import argparse
import json
import re
import shutil
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Protocol, cast

import torch
from datasets import Dataset
from peft import LoraConfig, PeftModel
from transformers import (
    AutoConfig,
    AutoModelForCausalLM,
    AutoTokenizer,
    BitsAndBytesConfig,
    PretrainedConfig,
    PreTrainedModel,
    PreTrainedTokenizerBase,
)
from transformers.trainer_utils import get_last_checkpoint
from trl.trainer.sft_config import SFTConfig
from trl.trainer.sft_trainer import SFTTrainer

Message = dict[str, Any]
Record = dict[str, Any]

# The type `apply_chat_template` takes for tool definitions.
TemplateTools = list[dict[Any, Any] | Callable[..., Any]]

# Training and generation run on the first GPU. On the Thor it shares memory with the CPU.
GPU = torch.device("cuda", 0)

# Attention projections and the dense MLP of every language-model layer. The vision and audio
# towers use their own linear class, which LoRA cannot wrap, so the pattern names
# `language_model`. In the MoE model each layer also has 128 experts stored as two 3-D tensors
# (`experts.gate_up_proj`, `experts.down_proj`); those are parameters, not linear modules, so
# the pattern leaves them frozen and the adapter trains the shared MLP beside them.
LORA_TARGETS = r".*language_model.*\.(q_proj|k_proj|v_proj|o_proj|gate_proj|up_proj|down_proj)"

TARGET_DESCRIPTION = {
    True: "MoE model: LoRA on attention and the shared MLP; the experts stay frozen",
    False: "dense model: LoRA on attention and the MLP",
}

# Gemma 4 opens a model turn with `<|turn>model\n`. The model then writes text and tool calls,
# and stops with `<|tool_response>` (the tool's answer follows) or with `<turn|>` (end of
# turn). After the tool's answer, closed by `<tool_response|>`, the model writes again.
MODEL_WRITTEN_TEXT = re.compile(
    r"(?:<\|turn>model\n|<tool_response\|>)(.*?(?:<\|tool_response>|<turn\|>))",
    re.DOTALL,
)

# Label value that the loss skips.
IGNORED_LABEL = -100

# The chat source of thor-hammer-trainer is an export of Claude conversations. Claude output
# is not used as training data, so those records are left out.
EXCLUDED_SOURCES = {"chat"}

# Book and doc sections carry no question; this prompt stands in for one until `lasso`
# writes a real question for each section.
PASSAGE_PROMPT = "Write the documentation section {origin}."

SAMPLE_TOOLS: TemplateTools = [
    {
        "type": "function",
        "function": {
            "name": "read_file",
            "description": "Read a file in the workspace.",
            "parameters": {
                "type": "object",
                "properties": {"path": {"type": "string", "description": "File path"}},
                "required": ["path"],
            },
        },
    },
]

SAMPLE_MESSAGES: list[Message] = [
    {"role": "system", "content": "You are a coding agent. Use the tools to check facts."},
    {"role": "user", "content": "What does src/main.rs print?"},
]


class TextGenerator(Protocol):
    """Structural type of a model with the `generate` method of transformers."""

    def generate(self, **kwargs: Any) -> torch.Tensor:
        """Returns the prompt token ids followed by the generated ids, shape (batch, length)."""
        ...


@dataclass(frozen=True)
class Settings:
    """Settings of one run, parsed from the command line."""

    mode: str
    """`train`, `merge` or `generate`."""

    model: str
    """Base model: a local folder or a Hugging Face id."""

    data: list[Path]
    """Training JSONL files."""

    output_dir: Path
    """Checkpoints and the adapter when training; the merged model when merging."""

    adapter: Path | None
    """Adapter folder for `merge` and `generate`."""

    precision: str
    """`bf16` loads the frozen base in bf16; `4bit` loads it in 4-bit NF4 (QLoRA)."""

    max_length: int
    """Longest example in tokens; longer ones are cut at the end."""

    epochs: float
    """Passes over the data."""

    max_steps: int
    """Optimizer steps to run; -1 runs all epochs."""

    learning_rate: float
    """Peak learning rate of the cosine schedule."""

    rank: int
    """LoRA rank; alpha is twice the rank."""

    gradient_accumulation: int
    """Examples per optimizer step; each forward pass holds one example."""

    save_steps: int
    """Checkpoint interval in optimizer steps."""

    resume: bool
    """Resume from the newest checkpoint in `output_dir` when one exists."""

    new_tokens: int
    """Tokens to generate in `generate` mode."""


@dataclass(frozen=True)
class Conversation:
    """One training example: tool definitions and chat messages."""

    tools: list[Record]
    messages: list[Message]


def argument_parser() -> argparse.ArgumentParser:
    """Returns the argument parser: `--merge` or `--generate` (default: train), model, data
    and training options."""
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])

    modes = parser.add_mutually_exclusive_group()
    modes.add_argument("--merge", dest="mode", action="store_const", const="merge")
    modes.add_argument("--generate", dest="mode", action="store_const", const="generate")
    parser.set_defaults(mode="train")

    parser.add_argument("--model", required=True, help="local folder or Hugging Face id")
    parser.add_argument("--data", nargs="*", default=[], help="training JSONL files")
    parser.add_argument("--output-dir", default="runs/lora")
    parser.add_argument("--adapter", default=None, help="adapter folder (merge, generate)")
    parser.add_argument("--precision", choices=["bf16", "4bit"], default="bf16")

    parser.add_argument("--max-length", type=int, default=4096)
    parser.add_argument("--epochs", type=float, default=1.0)
    parser.add_argument("--max-steps", type=int, default=-1)
    parser.add_argument("--learning-rate", type=float, default=1e-4)
    parser.add_argument("--rank", type=int, default=16)
    parser.add_argument("--gradient-accumulation", type=int, default=16)
    parser.add_argument("--save-steps", type=int, default=50)
    parser.add_argument("--resume", action="store_true")
    parser.add_argument("--new-tokens", type=int, default=200)

    return parser


def optional_path(value: str | None) -> Path | None:
    """Returns `value` as a `Path`, or None when `value` is None."""
    if value is None:
        return None

    return Path(value)


def parsed_settings() -> Settings:
    """Parses `sys.argv` and returns the run settings. Exits with status 2 on bad arguments."""
    arguments = argument_parser().parse_args()
    adapter = optional_path(arguments.adapter)

    return Settings(
        mode=arguments.mode,
        model=arguments.model,
        data=[Path(path) for path in arguments.data],
        output_dir=Path(arguments.output_dir),
        adapter=adapter,
        precision=arguments.precision,
        max_length=arguments.max_length,
        epochs=arguments.epochs,
        max_steps=arguments.max_steps,
        learning_rate=arguments.learning_rate,
        rank=arguments.rank,
        gradient_accumulation=arguments.gradient_accumulation,
        save_steps=arguments.save_steps,
        resume=arguments.resume,
        new_tokens=arguments.new_tokens,
    )


def json_lines(path: Path) -> list[Record]:
    """Returns the JSON objects of a JSONL file; blank lines are ignored.

    Raises `json.JSONDecodeError` on a malformed line and `OSError` when the file cannot be read.
    """
    lines = path.read_text(encoding="utf-8").splitlines()

    return [json.loads(line) for line in filter(str.strip, lines)]


def training_records(paths: list[Path]) -> list[Record]:
    """Returns the records of all `paths`, concatenated in argument order."""
    records: list[Record] = []

    for path in paths:
        records.extend(json_lines(path))

    return records


def is_wanted(record: Record) -> bool:
    """Returns False when the record's `source` is in `EXCLUDED_SOURCES`; records without a
    `source` field are wanted."""
    return record.get("source") not in EXCLUDED_SOURCES


def template_tool_call(call: Record) -> Record:
    """Returns a copy of an OpenAI tool call with `function.arguments` as a mapping.

    The Gemma 4 template rejects string arguments; OpenAI messages carry them as a JSON string.
    Arguments that are already a mapping are returned unchanged.
    """
    function = call["function"]
    arguments = function["arguments"]
    parsed = json.loads(arguments) if isinstance(arguments, str) else arguments

    return {**call, "function": {**function, "arguments": parsed}}


def text_content(message: Message) -> str:
    """Returns the message content, or "" when it is missing or null (a tool-calling turn)."""
    content: str = message.get("content") or ""

    return content


def template_message(message: Message) -> Message:
    """Returns a copy of `message` that the Gemma 4 template accepts: string content and a
    `tool_calls` list (empty when the message has none) with mapping arguments."""
    tool_calls = [template_tool_call(call) for call in message.get("tool_calls", [])]

    return {**message, "content": text_content(message), "tool_calls": tool_calls}


def conversation_from_example(record: Record) -> Conversation:
    """Returns a `thor-hammer-trainer` record as a user turn and an assistant turn.

    An empty `instruction` (a book or doc passage) is replaced by `PASSAGE_PROMPT`.
    """
    instruction = record["instruction"] or PASSAGE_PROMPT.format(origin=record["origin"])

    messages = [
        {"role": "user", "content": instruction},
        {"role": "assistant", "content": record["response"]},
    ]

    return Conversation(tools=[], messages=messages)


def conversation_from_chat(record: Record) -> Conversation:
    """Returns a `{"tools", "messages"}` line as a `Conversation`; `tools` defaults to []."""
    messages = [template_message(message) for message in record["messages"]]
    tools: list[Record] = record.get("tools", [])

    return Conversation(tools=tools, messages=messages)


def conversation_from_record(record: Record) -> Conversation:
    """Returns a training line as a `Conversation`. Lines with `messages` are chat lines;
    all others are read as `thor-hammer-trainer` records."""
    if "messages" not in record:
        return conversation_from_example(record)

    return conversation_from_chat(record)


def model_written_spans(text: str) -> list[tuple[int, int]]:
    """Returns the [start, end) character ranges of `text` written by the model, each ending
    with its stop token (`<|tool_response>` or `<turn|>`)."""
    return [match.span(1) for match in MODEL_WRITTEN_TEXT.finditer(text)]


def is_inside(position: int, spans: list[tuple[int, int]]) -> bool:
    """Returns True when `position` lies in one of the [start, end) `spans`."""
    return any(start <= position < end for start, end in spans)


def label_for(token_id: int, token_start: int, spans: list[tuple[int, int]]) -> int:
    """Returns `token_id` when the token starts inside `spans`, otherwise `IGNORED_LABEL`."""
    return token_id if is_inside(token_start, spans) else IGNORED_LABEL


def labels_for(
    token_ids: list[int], offsets: list[tuple[int, int]], spans: list[tuple[int, int]]
) -> list[int]:
    """Returns one label per token: the id for model-written tokens, `IGNORED_LABEL` otherwise.

    `token_ids` and `offsets` must have the same length; `zip(strict=True)` raises otherwise.
    """
    return [
        label_for(token_id, start, spans)
        for token_id, (start, _end) in zip(token_ids, offsets, strict=True)
    ]


def template_tools(conversation: Conversation) -> TemplateTools | None:
    """Returns the tool definitions, or None when there are none, so the template writes no
    tool block."""
    tools: TemplateTools = [*conversation.tools]

    return tools or None


def rendered_text(
    tokenizer: PreTrainedTokenizerBase,
    messages: list[Message],
    tools: TemplateTools | None,
    add_generation_prompt: bool,
) -> str:
    """Returns the conversation rendered by the tokenizer's chat template, special tokens
    included. `add_generation_prompt` appends the opening of a model turn."""
    text = tokenizer.apply_chat_template(
        messages,
        tools=tools,
        add_generation_prompt=add_generation_prompt,
        tokenize=False,
    )

    return str(text)


def tokenized_example(
    conversation: Conversation, tokenizer: PreTrainedTokenizerBase
) -> dict[str, list[int]]:
    """Returns `input_ids` and `labels` for one conversation, untruncated.

    Labels equal the ids on model-written tokens and `IGNORED_LABEL` elsewhere. SFTTrainer
    truncates to `max_length` and drops examples with no labeled token left.
    """
    tools = template_tools(conversation)
    text = rendered_text(tokenizer, conversation.messages, tools, add_generation_prompt=False)
    spans = model_written_spans(text)

    encoding = tokenizer(text, add_special_tokens=False, return_offsets_mapping=True)
    token_ids: list[int] = encoding["input_ids"]
    offsets: list[tuple[int, int]] = encoding["offset_mapping"]

    return {"input_ids": token_ids, "labels": labels_for(token_ids, offsets, spans)}


def training_conversations(paths: list[Path]) -> list[Conversation]:
    """Returns the wanted records of `paths` as conversations, in file order."""
    records = filter(is_wanted, training_records(paths))

    return [conversation_from_record(record) for record in records]


def training_dataset(settings: Settings, tokenizer: PreTrainedTokenizerBase) -> Dataset:
    """Returns the tokenized, labeled training set with columns `input_ids` and `labels`."""
    conversations = training_conversations(settings.data)
    examples = [tokenized_example(conversation, tokenizer) for conversation in conversations]

    print(f"{len(examples)} examples from {len(settings.data)} files")

    return Dataset.from_list(examples)


def is_moe(config: PretrainedConfig) -> bool:
    """Returns True when the text config enables the mixture-of-experts block."""
    text_config = config.get_text_config()

    return bool(getattr(text_config, "enable_moe_block", False))


def four_bit_config() -> BitsAndBytesConfig:
    """Returns the QLoRA quantization: NF4 weights, double quantization, bf16 compute."""
    return BitsAndBytesConfig(
        load_in_4bit=True,
        bnb_4bit_quant_type="nf4",
        bnb_4bit_use_double_quant=True,
        bnb_4bit_compute_dtype=torch.bfloat16,
    )


def quantization_config(precision: str) -> BitsAndBytesConfig | None:
    """Returns None for `bf16` and the NF4 config for `4bit`."""
    if precision == "bf16":
        return None

    return four_bit_config()


def base_model(model_name: str, precision: str) -> PreTrainedModel:
    """Loads the model in bf16, or in NF4 when `precision` is `4bit`, onto `GPU`."""
    model: PreTrainedModel = AutoModelForCausalLM.from_pretrained(
        model_name,
        dtype=torch.bfloat16,
        quantization_config=quantization_config(precision),
        device_map={"": GPU},
    )

    return model


def lora_config(rank: int) -> LoraConfig:
    """Returns a LoRA config of the given rank (alpha 2 * rank, dropout 0.05) on `LORA_TARGETS`."""
    return LoraConfig(
        r=rank,
        lora_alpha=2 * rank,
        lora_dropout=0.05,
        target_modules=LORA_TARGETS,
        task_type="CAUSAL_LM",
    )


def sft_config(settings: Settings) -> SFTConfig:
    """Returns the trainer config: batch size 1, cosine schedule, bf16, gradient
    checkpointing, a checkpoint every `save_steps` with the newest 3 kept."""
    return SFTConfig(
        output_dir=str(settings.output_dir),
        num_train_epochs=settings.epochs,
        max_steps=settings.max_steps,
        learning_rate=settings.learning_rate,
        lr_scheduler_type="cosine",
        warmup_steps=10,
        per_device_train_batch_size=1,
        gradient_accumulation_steps=settings.gradient_accumulation,
        gradient_checkpointing=True,
        gradient_checkpointing_kwargs={"use_reentrant": False},
        bf16=True,
        max_length=settings.max_length,
        save_steps=settings.save_steps,
        save_total_limit=3,
        logging_steps=10,
        report_to="none",
    )


def newest_checkpoint(output_dir: Path) -> str | None:
    """Returns the newest `checkpoint-N` folder in `output_dir`, or None when the folder is
    missing or holds no checkpoint."""
    if not output_dir.is_dir():
        return None

    checkpoint: str | None = get_last_checkpoint(str(output_dir))

    return checkpoint


def resume_point(settings: Settings) -> str | None:
    """Returns the checkpoint to resume from, or None without `--resume` or without a checkpoint."""
    if not settings.resume:
        return None

    return newest_checkpoint(settings.output_dir)


def train(settings: Settings) -> None:
    """Trains a LoRA adapter and saves it with the tokenizer in `output_dir`. Resumes from the
    newest checkpoint when `resume` is set."""
    tokenizer = AutoTokenizer.from_pretrained(settings.model)
    dataset = training_dataset(settings, tokenizer)

    config = AutoConfig.from_pretrained(settings.model)
    print(TARGET_DESCRIPTION[is_moe(config)])

    trainer = SFTTrainer(
        model=base_model(settings.model, settings.precision),
        args=sft_config(settings),
        train_dataset=dataset,
        processing_class=tokenizer,
        peft_config=lora_config(settings.rank),
    )

    checkpoint = resume_point(settings)
    print(f"resuming from {checkpoint}" if checkpoint else "starting from the base model")

    trainer.train(resume_from_checkpoint=checkpoint)
    trainer.save_model(str(settings.output_dir))
    print(f"adapter saved in {settings.output_dir}")


def with_adapter(model: PreTrainedModel, adapter: Path | None) -> PreTrainedModel | PeftModel:
    """Returns `model` with the adapter at `adapter` loaded, or `model` when `adapter` is None."""
    if adapter is None:
        return model

    return PeftModel.from_pretrained(model, str(adapter))


def text_generator(model: PreTrainedModel | PeftModel) -> TextGenerator:
    """Returns `model` typed as a `TextGenerator`.

    Every causal language model in transformers has `generate`; the static type of a loaded
    model does not declare it. The cast performs no runtime check.
    """
    return cast(TextGenerator, model)


def copy_processor_config(model_name: str, output_dir: Path) -> None:
    """Copies `processor_config.json` from a local base model folder into `output_dir`.

    Quantizing tools read it to rebuild the multimodal model. Does nothing when `model_name`
    is a Hugging Face id or the file is absent.
    """
    source = Path(model_name) / "processor_config.json"

    if not source.is_file():
        return

    shutil.copy(source, output_dir / source.name)


def merge(settings: Settings) -> None:
    """Merges `adapter` into the bf16 base and saves the model, tokenizer and processor config
    in `output_dir`."""
    base = base_model(settings.model, "bf16")
    adapted = PeftModel.from_pretrained(base, str(settings.adapter))

    merged = adapted.merge_and_unload()
    merged.save_pretrained(str(settings.output_dir))

    tokenizer = AutoTokenizer.from_pretrained(settings.model)
    tokenizer.save_pretrained(str(settings.output_dir))
    copy_processor_config(settings.model, settings.output_dir)

    print(f"merged model saved in {settings.output_dir}")


def generate(settings: Settings) -> None:
    """Prints the greedy answer to `SAMPLE_MESSAGES` with `SAMPLE_TOOLS`, special tokens
    included."""
    tokenizer = AutoTokenizer.from_pretrained(settings.model)
    model = with_adapter(base_model(settings.model, settings.precision), settings.adapter)

    prompt = rendered_text(tokenizer, SAMPLE_MESSAGES, SAMPLE_TOOLS, add_generation_prompt=True)
    inputs = tokenizer(prompt, add_special_tokens=False, return_tensors="pt").to(GPU)
    prompt_length = inputs["input_ids"].shape[1]

    output = text_generator(model).generate(
        **inputs,
        max_new_tokens=settings.new_tokens,
        do_sample=False,
    )

    print(tokenizer.decode(output[0][prompt_length:]))


MODES: dict[str, Callable[[Settings], None]] = {
    "train": train,
    "merge": merge,
    "generate": generate,
}


def main() -> None:
    """Runs the mode chosen on the command line."""
    settings = parsed_settings()

    MODES[settings.mode](settings)


if __name__ == "__main__":
    main()
