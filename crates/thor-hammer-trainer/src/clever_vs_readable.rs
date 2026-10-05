//! The clever-vs-readable set: verified rewrites of hard-to-read code.
//!
//! ## Format
//!
//! The set comes as two JSONL files built and verified by `build.py`:
//!
//! ```text
//!   clever_vs_readable_sft.jsonl  {messages: [system, user, assistant], meta}   74 records
//!   clever_vs_readable_dpo.jsonl  {prompt, chosen, rejected, meta}              37 pairs
//! ```
//!
//! An SFT record becomes an [`Example`]: the user message is the instruction and
//! the assistant message the response. The system message is the same rule text
//! in every record, so it is dropped here; the training script supplies one
//! system prompt for the whole run.
//!
//! A DPO record is a **preference pair**: one prompt, a chosen answer and a
//! rejected answer. It does not fit the instruction/response shape, so it is
//! passed through unchanged to `preferences.jsonl`, in the conversational format
//! TRL's DPO trainer reads.

use serde::{Deserialize, Serialize};

use crate::{
    error::DataError,
    example::{Example, SkipReason, Source},
};

/// The three roles of a chat message.
#[derive(Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum Role {
    System,
    User,
    Assistant,
}

/// One message of a chat-format record.
#[derive(Deserialize, Serialize)]
struct ChatMessage {
    role: Role,
    content: String,
}

/// The metadata `build.py` attaches to every record.
#[derive(Deserialize)]
struct Metadata {
    /// The entry id, such as `rs-kadane-tuple-fold`.
    id: String,
    /// `refactor` or `write`: the two prompts built from one entry.
    #[serde(default)]
    task_type: String,
}

/// One record of `clever_vs_readable_sft.jsonl`.
#[derive(Deserialize)]
struct SftRecord {
    messages: Vec<ChatMessage>,
    meta: Metadata,
}

/// One record of `clever_vs_readable_dpo.jsonl`, written back out unchanged.
#[derive(Deserialize, Serialize)]
pub struct PreferencePair {
    prompt: Vec<ChatMessage>,
    chosen: Vec<ChatMessage>,
    rejected: Vec<ChatMessage>,
    meta: serde_json::Value,
}

impl SftRecord {
    /// The content of the first message with `role`, if there is one.
    fn content_of(&self, role: Role) -> Option<&str> {
        self.messages
            .iter()
            .find(|message| message.role == role)
            .map(|message| message.content.as_str())
    }
}

/// Every SFT record as an example. A record without a user or an assistant
/// message is recorded as [`SkipReason::EmptyTurn`].
pub fn examples(
    sft_jsonl: &str,
    skip_reasons: &mut Vec<SkipReason>,
) -> Result<Vec<Example>, DataError> {
    let mut examples = Vec::new();
    for line in sft_jsonl
        .lines()
        .filter(|line| !line.trim().is_empty())
    {
        let record: SftRecord = serde_json::from_str(line)?;
        let origin = format!(
            "clever_vs_readable_sft.jsonl#{}/{}",
            record.meta.id, record.meta.task_type
        );
        let (Some(instruction), Some(response)) = (
            record.content_of(Role::User),
            record.content_of(Role::Assistant),
        ) else {
            skip_reasons.push(SkipReason::EmptyTurn);
            continue;
        };
        examples.push(Example {
            instruction: instruction.trim().to_string(),
            response: response.trim().to_string(),
            source: Source::CleverVsReadable,
            origin,
        });
    }
    Ok(examples)
}

/// Every DPO record, parsed to check its shape.
pub fn preference_pairs(dpo_jsonl: &str) -> Result<Vec<PreferencePair>, DataError> {
    let mut pairs = Vec::new();
    for line in dpo_jsonl
        .lines()
        .filter(|line| !line.trim().is_empty())
    {
        pairs.push(serde_json::from_str(line)?);
    }
    Ok(pairs)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SFT: &str = r#"{"messages": [{"role": "system", "content": "Rules."}, {"role": "user", "content": "Rewrite this."}, {"role": "assistant", "content": "Done."}], "meta": {"id": "rs-x", "task_type": "refactor"}}
{"messages": [{"role": "system", "content": "Rules."}, {"role": "user", "content": "No answer."}], "meta": {"id": "rs-y"}}
"#;

    #[test]
    fn user_and_assistant_become_an_example() -> Result<(), DataError> {
        let mut skip_reasons = Vec::new();
        let examples = examples(SFT, &mut skip_reasons)?;
        let expected = Example {
            instruction: "Rewrite this.".to_string(),
            response: "Done.".to_string(),
            source: Source::CleverVsReadable,
            origin: "clever_vs_readable_sft.jsonl#rs-x/refactor".to_string(),
        };
        assert_eq!(examples, [expected]);
        assert_eq!(skip_reasons, [SkipReason::EmptyTurn]);
        Ok(())
    }

    #[test]
    fn preference_pairs_round_trip() -> Result<(), DataError> {
        let line = r#"{"prompt":[{"role":"user","content":"Q"}],"chosen":[{"role":"assistant","content":"good"}],"rejected":[{"role":"assistant","content":"bad"}],"meta":{"id":"rs-x"}}"#;
        let pairs = preference_pairs(line)?;
        assert_eq!(serde_json::to_string(&pairs)?, format!("[{line}]"));
        Ok(())
    }
}
