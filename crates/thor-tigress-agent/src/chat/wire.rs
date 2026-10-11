//! What travels on the wire during a chat: the arguments of a tool call, the
//! pieces of a streamed answer and the round they add up to, and the events
//! the page is sent.

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use super::MAX_CALLS;
use crate::web::{
    research::{Kind, MAX_QUERIES_PER_CALL},
    search::TimeRange,
};

/// The arguments of one `web_search` call: a sub-question, more sub-questions,
/// how to widen them, and how recent the answers must be.
#[derive(Deserialize)]
pub(super) struct SearchArguments {
    #[serde(default)]
    pub(super) query: Option<String>,
    #[serde(default)]
    pub(super) queries: Option<Vec<String>>,
    #[serde(default)]
    pub(super) kind: Option<String>,
    #[serde(default)]
    pub(super) time_range: Option<String>,
}

impl SearchArguments {
    /// The sub-questions asked for, trimmed, in order and without repeats.
    fn asked(&self) -> Vec<String> {
        let more = self.queries.as_deref().unwrap_or_default();
        let mut asked: Vec<String> = Vec::new();

        for query in self.query.iter().chain(more).map(|query| query.trim()) {
            if !query.is_empty() && !asked.iter().any(|seen| seen == query) {
                asked.push(query.to_string());
            }
        }

        asked
    }
}

/// One `web_search` call, worked out: the queries to run and the recency asked
/// for. The queries are already widened by `kind` and capped.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct SearchPlan {
    /// The searches to run, in order.
    pub(super) queries: Vec<String>,
    /// The recency to ask SearXNG for.
    pub(super) range: Option<TimeRange>,
}

/// The arguments of one `fetch_page_content_recursive` call.
#[derive(Deserialize)]
pub(super) struct FetchArguments {
    pub(super) url: String,
}

/// One streamed chunk from the engine, as far as this server reads it.
#[derive(Deserialize)]
pub(super) struct Chunk {
    #[serde(default)]
    pub(super) choices: Vec<Choice>,
}

#[derive(Deserialize)]
pub(super) struct Choice {
    #[serde(default)]
    pub(super) delta: Delta,
}

/// What one chunk adds to the reply: text, pieces of tool calls, or both.
#[derive(Deserialize, Default)]
pub(super) struct Delta {
    pub(super) content: Option<String>,
    pub(super) tool_calls: Option<Vec<CallPiece>>,
}

/// A piece of a tool call; the id, name and arguments arrive in parts.
#[derive(Deserialize)]
pub(super) struct CallPiece {
    #[serde(default)]
    pub(super) index: usize,
    pub(super) id: Option<String>,
    pub(super) function: Option<FunctionPiece>,
}

#[derive(Deserialize)]
pub(super) struct FunctionPiece {
    pub(super) name: Option<String>,
    pub(super) arguments: Option<String>,
}

/// One tool call, assembled from its pieces.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct ToolCall {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) arguments: String,
}

/// What one streamed round produced besides the tokens already forwarded.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Round {
    pub(super) content: String,
    pub(super) calls: Vec<ToolCall>,
    /// Whether the round was cut off at [`super::MAX_ROUND_CHARS`].
    pub(super) cut: bool,
}

/// A message this server adds to the conversation, in OpenAI's shape.
#[derive(Serialize)]
#[serde(tag = "role", rename_all = "lowercase")]
pub(super) enum Added<'a> {
    /// The model's turn that asked for tools.
    Assistant {
        content: &'a str,
        tool_calls: Vec<CallRecord<'a>>,
    },
    /// One tool's result.
    Tool {
        tool_call_id: &'a str,
        content: &'a str,
    },
}

/// A tool call as it is written back into the conversation.
#[derive(Serialize)]
pub(super) struct CallRecord<'a> {
    pub(super) id: &'a str,
    #[serde(rename = "type")]
    pub(super) kind: &'static str,
    pub(super) function: FunctionRecord<'a>,
}

#[derive(Serialize)]
pub(super) struct FunctionRecord<'a> {
    pub(super) name: &'a str,
    pub(super) arguments: &'a str,
}

/// An event this server adds to the stream, under a `thor` key so the page
/// can tell it from the model's own events.
#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum ThorEvent<'a> {
    /// A search ran: the query and the sources it found.
    Search {
        query: &'a str,
        results: Vec<Source<'a>>,
    },
    /// A page was read: its address, its title, and the host to show.
    Read {
        url: &'a str,
        title: &'a str,
        domain: String,
    },
    /// Something failed after the stream started.
    Error(String),
}

#[derive(Serialize)]
pub(super) struct Source<'a> {
    pub(super) title: &'a str,
    pub(super) url: &'a str,
    pub(super) domain: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) published: Option<&'a str>,
}

impl Round {
    /// Adds the text and tool-call pieces of one streamed chunk.
    pub(super) fn absorb(&mut self, chunk: Chunk) {
        let Some(Choice { delta }) = chunk.choices.into_iter().next() else {
            return;
        };

        if let Some(text) = delta.content {
            self.content.push_str(&text);
        }

        for piece in delta.tool_calls.unwrap_or_default() {
            if piece.index >= MAX_CALLS {
                continue;
            }

            if self.calls.len() <= piece.index {
                self.calls.resize_with(piece.index + 1, ToolCall::default);
            }

            self.calls[piece.index].extend(piece);
        }
    }

    /// The assistant message that asked for this round's tool calls.
    pub(super) fn as_message(&self) -> Added<'_> {
        let tool_calls = self
            .calls
            .iter()
            .map(|call| CallRecord {
                id: &call.id,
                kind: "function",
                function: FunctionRecord {
                    name: &call.name,
                    arguments: &call.arguments,
                },
            })
            .collect();

        Added::Assistant {
            content: &self.content,
            tool_calls,
        }
    }
}

impl ToolCall {
    /// Appends one streamed piece.
    pub(super) fn extend(&mut self, piece: CallPiece) {
        self.id.push_str(&piece.id.unwrap_or_default());

        if let Some(function) = piece.function {
            self.name.push_str(&function.name.unwrap_or_default());
            self.arguments
                .push_str(&function.arguments.unwrap_or_default());
        }
    }

    /// The arguments of this call, parsed.
    pub(super) fn arguments<T: DeserializeOwned>(&self) -> Option<T> {
        serde_json::from_str(&self.arguments).ok()
    }

    /// The searches one call asks for: the `query`, plus every `queries` entry,
    /// each widened by `kind`, capped at [`MAX_QUERIES_PER_CALL`] and without
    /// repeats. `None` when no query is given. An unknown `time_range` or `kind`
    /// is treated as absent.
    pub(super) fn search_plan(&self) -> Option<SearchPlan> {
        let arguments: SearchArguments = self.arguments()?;

        let base = arguments.asked();

        if base.is_empty() {
            return None;
        }

        let kind = Kind::of(arguments.kind.as_deref());
        let mut queries: Vec<String> = Vec::new();

        for query in base.iter().flat_map(|query| kind.widen(query)) {
            if queries.len() == MAX_QUERIES_PER_CALL {
                break;
            }

            if !queries.contains(&query) {
                queries.push(query);
            }
        }

        Some(SearchPlan {
            queries,
            range: arguments.time_range.as_deref().and_then(TimeRange::of),
        })
    }

    /// The `url` of a fetch, trimmed; `None` when missing or empty.
    pub(super) fn url_argument(&self) -> Option<String> {
        let arguments: FetchArguments = self.arguments()?;
        let url = arguments.url.trim();

        (!url.is_empty()).then(|| url.to_string())
    }
}
