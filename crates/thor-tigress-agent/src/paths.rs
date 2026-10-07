//! The paths this server answers. The `/v1/` ones are also llama-server's
//! own, so requests are passed on under the same path.

/// Liveness check, open to everyone.
pub const HEALTH: &str = "/health";
/// The chat page's own folder on the domain.
pub const CUB: &str = "/thor-tigress-cub";
/// Everything under this prefix needs the access key.
pub const API: &str = "/v1/";
/// OpenAI: the served models.
pub const MODELS: &str = "/v1/models";
/// OpenAI: chat.
pub const CHAT_COMPLETIONS: &str = "/v1/chat/completions";
/// Anthropic: messages, used by Claude Code.
pub const MESSAGES: &str = "/v1/messages";
/// Anthropic: token counting.
pub const COUNT_TOKENS: &str = "/v1/messages/count_tokens";
