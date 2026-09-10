//! Wire types for the Converse API as Bedrock serves it, and the translation to and from the shapes
//! the rest of this agent speaks.
//!
//! The body is the one Bedrock states for every provider it hosts rather than any single provider's
//! own, so which model a request names decides nothing about how it is built. What a provider
//! defines for itself travels in the passthrough field the service hands to the model unread.
//!
//! The agent's conversation is held in OpenAI-compatible types, because that is what the other
//! backend speaks. This module converts, in one place, rather than teaching the turn loop two
//! protocols. The differences that matter:
//!
//! - The system prompt is a top-level field, not a message with a role.
//! - Tool calls and their results are content blocks inside user and assistant turns, not a
//!   separate role with an id alongside.
//! - Arguments arrive as a JSON object, not as a string holding one.
//! - Usage counts `inputTokens` and `outputTokens` rather than prompt and completion.
//! - A streamed event is named in its frame's headers, not in a field of its own body.
//!
//! Nothing here inspects content to make a decision. Text is moved between shapes and handed on with
//! whatever label it arrived under.

use bravebot_aichat::protocol::Effort;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// How many tokens a reply may run to before it is cut off.
///
/// Chosen to be larger than any single reply a turn here produces: a tool call and its reasoning,
/// not a document. A cut-off reply is reported as one rather than silently truncated, but the
/// cheaper fix is to not hit it.
pub const MAX_TOKENS: u64 = 8_192;

/// The stop reason meaning the reply hit the ceiling rather than finishing.
pub const STOP_REASON_MAX_TOKENS: &str = "max_tokens";

/// The image formats this API takes, and the media type each one arrives as.
///
/// An attachment names a media type and the API names a format, so the two are matched here. A
/// media type absent from both this table and [`DOCUMENT_FORMATS`] is one the service refuses, and
/// refusing it locally is the same argument that stops an unconfigured tier being guessed at.
const IMAGE_FORMATS: [(&str, &str); 4] = [
    ("image/png", "png"),
    ("image/jpeg", "jpeg"),
    ("image/gif", "gif"),
    ("image/webp", "webp"),
];

/// The document formats this API takes, matched the same way.
///
/// This API sorts an attachment by what it is rather than by its media type, so what it will not
/// take as a picture it may still take as a document. The agent carries one of these, and a
/// picture block naming it is refused by the service.
const DOCUMENT_FORMATS: [(&str, &str); 1] = [("application/pdf", "pdf")];

/// A cache breakpoint: everything in front of it may be reused by the next request.
///
/// A block of its own rather than a field on another block, which is the shape this API states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CachePoint {
    #[serde(rename = "type")]
    pub kind: &'static str,
}

impl CachePoint {
    /// The only kind this API offers, and the only one worth asking for: a turn re-sends its whole
    /// history every round, and rounds are seconds apart.
    pub fn new() -> Self {
        Self { kind: "default" }
    }
}

impl Default for CachePoint {
    fn default() -> Self {
        Self::new()
    }
}

/// One block of the system prompt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SystemBlock {
    Text(String),
    CachePoint(CachePoint),
}

/// A request to Bedrock.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConverseRequest {
    /// The system prompt, hoisted out of the message list.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<Vec<SystemBlock>>,
    pub messages: Vec<ConverseMessage>,
    pub inference_config: InferenceConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_config: Option<ToolConfig>,
    /// What the service hands to the model without reading it.
    ///
    /// Absent unless somebody asked for a level, so a build nobody has asked sends the body it
    /// always sent and the model keeps its own default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additional_model_request_fields: Option<Value>,
}

/// The parameters every model this API serves takes.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InferenceConfig {
    pub max_tokens: u64,
}

/// The tools a turn is offering, in the wrapper this API states them in.
#[derive(Debug, Clone, Serialize)]
pub struct ToolConfig {
    pub tools: Vec<ToolEntry>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ToolEntry {
    ToolSpec(ToolSpec),
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: ToolSchema,
}

/// A tool's parameters, which this API states under the notation they are written in.
#[derive(Debug, Clone, Serialize)]
pub struct ToolSchema {
    pub json: Value,
}

/// Serialised only: `role` is a fixed string this crate chooses, never one it reads back.
#[derive(Debug, Clone, Serialize)]
pub struct ConverseMessage {
    pub role: &'static str,
    pub content: Vec<Block>,
}

/// One content block on the way out.
///
/// Every member of the union names itself, which is what this API does in place of a `type` field.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Block {
    Text(String),
    Image(Image),
    Document(Document),
    ToolUse(ToolUse),
    ToolResult(ToolResult),
    CachePoint(CachePoint),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Image {
    /// The format this API names, which is a word of its own rather than a media type.
    ///
    /// Serialised only, and one of a fixed set this crate chooses from, never a value read back.
    pub format: &'static str,
    pub source: AttachmentSource,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Document {
    pub format: &'static str,
    /// What this API calls the document, which it requires and this has to invent.
    ///
    /// A data URI carries no filename, so the name is the attachment's position in the turn. It
    /// reaches the model, so it is this crate's own word and never anything read from the file.
    pub name: String,
    pub source: AttachmentSource,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AttachmentSource {
    /// The attachment itself, base64 as this API takes it over JSON.
    pub bytes: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolUse {
    pub tool_use_id: String,
    pub name: String,
    /// Absent where a call takes no arguments. Read as an empty object rather than refused, since a
    /// block that will not parse is one this drops, which loses the call the model asked for, and
    /// an empty object is what the same call arriving in pieces over a stream already becomes.
    #[serde(default = "empty_object")]
    pub input: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolResult {
    pub tool_use_id: String,
    /// A list, because this API lets one result carry several pieces. A tool here returns text.
    pub content: Vec<ToolResultBlock>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ToolResultBlock {
    Text(String),
}

/// The arguments of a call that named none.
fn empty_object() -> Value {
    json!({})
}

/// A complete reply.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConverseResponse {
    #[serde(default)]
    pub output: Option<Output>,
    /// Why the model stopped. `max_tokens` here means the reply was cut off.
    #[serde(default)]
    pub stop_reason: Option<String>,
    #[serde(default)]
    pub usage: Option<BedrockUsage>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Output {
    #[serde(default)]
    pub message: Option<ReplyMessage>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReplyMessage {
    #[serde(default)]
    pub content: Vec<ReplyBlock>,
}

/// One content block on the way back.
///
/// Read as whichever member it matches, with anything else kept whole and ignored. The union grows
/// with what the models this API fronts can produce, and a block naming something unmodelled must
/// not fail a reply that is otherwise complete.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ReplyBlock {
    Text {
        text: String,
    },
    ToolUse {
        #[serde(rename = "toolUse")]
        tool_use: ToolUse,
    },
    Other(Value),
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BedrockUsage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    /// Tokens served out of the cache, which this API reports apart from `inputTokens` rather
    /// than inside it. Counted here so a cached round does not read as a shrinking conversation.
    #[serde(default)]
    pub cache_read_input_tokens: u64,
    /// Tokens written into the cache on the way past, reported apart for the same reason.
    #[serde(default)]
    pub cache_write_input_tokens: u64,
}

/// One frame of a streamed reply.
///
/// Only the events that carry text, a tool call, or a count. The API sends several others
/// (`messageStart`, `contentBlockStop`) that say nothing this needs.
#[derive(Debug, Clone)]
pub enum StreamEvent {
    ContentBlockStart {
        index: usize,
        start: BlockStart,
    },
    ContentBlockDelta {
        index: usize,
        delta: Delta,
    },
    MessageStop {
        stop_reason: Option<String>,
    },
    /// The counts, which this API sends once at the end rather than across the reply.
    Metadata {
        usage: Option<BedrockUsage>,
    },
}

/// What a block that has just opened is.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum BlockStart {
    ToolUse {
        #[serde(rename = "toolUse")]
        tool_use: ToolUseStart,
    },
    Other(Value),
}

/// A tool call's identity, which arrives before any of its arguments do.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolUseStart {
    pub tool_use_id: String,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Delta {
    Text {
        text: String,
    },
    ToolUse {
        #[serde(rename = "toolUse")]
        tool_use: ToolUseDelta,
    },
    Other(Value),
}

/// A piece of a tool call's arguments, which arrive as JSON in fragments.
#[derive(Debug, Clone, Deserialize)]
pub struct ToolUseDelta {
    pub input: String,
}

/// The event a named frame carries, or nothing for one this does not model.
///
/// The name is a frame header rather than a field of the body, which is the framing's business and
/// not the reply's: a name this does not recognise is skipped, exactly as an unmodelled field is.
pub fn stream_event(name: &str, payload: &[u8]) -> Option<StreamEvent> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct StartBody {
        #[serde(default)]
        content_block_index: usize,
        start: BlockStart,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct DeltaBody {
        #[serde(default)]
        content_block_index: usize,
        delta: Delta,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct StopBody {
        #[serde(default)]
        stop_reason: Option<String>,
    }

    #[derive(Deserialize)]
    struct MetadataBody {
        #[serde(default)]
        usage: Option<BedrockUsage>,
    }

    match name {
        "contentBlockStart" => {
            let body: StartBody = serde_json::from_slice(payload).ok()?;
            Some(StreamEvent::ContentBlockStart {
                index: body.content_block_index,
                start: body.start,
            })
        }
        "contentBlockDelta" => {
            let body: DeltaBody = serde_json::from_slice(payload).ok()?;
            Some(StreamEvent::ContentBlockDelta {
                index: body.content_block_index,
                delta: body.delta,
            })
        }
        "messageStop" => {
            let body: StopBody = serde_json::from_slice(payload).ok()?;
            Some(StreamEvent::MessageStop {
                stop_reason: body.stop_reason,
            })
        }
        "metadata" => {
            let body: MetadataBody = serde_json::from_slice(payload).ok()?;
            Some(StreamEvent::Metadata { usage: body.usage })
        }
        _ => None,
    }
}

impl ConverseRequest {
    /// Ask for a particular amount of thinking, or leave the model to its own default.
    ///
    /// This API's own parameters have no field for it, so the level goes in the passthrough under
    /// the name the model reading it gives the field.
    pub fn with_effort(mut self, effort: Option<Effort>) -> Self {
        self.additional_model_request_fields =
            effort.map(|effort| json!({ "output_config": { "effort": effort } }));
        self
    }
}

/// Build a Bedrock request from the conversation the agent holds.
///
/// The message list is the same conversation, restated. System turns are hoisted into the top-level
/// field, tool results move from their own role into blocks on a user turn, and consecutive turns
/// that would now share a role are merged, because the API rejects two turns of the same role in a
/// row.
///
/// The model is not a parameter: Bedrock names it in the URL path rather than in the body, which is
/// one of the differences from the other backend.
pub fn request_from(
    messages: &[bravebot_aichat::protocol::Message],
    tools: Option<&[bravebot_aichat::protocol::Tool]>,
) -> ConverseRequest {
    use bravebot_aichat::protocol::Role;

    let mut system: Vec<String> = Vec::new();
    let mut converted: Vec<ConverseMessage> = Vec::new();

    for message in messages {
        match message.role {
            // Hoisted rather than sent as a turn. Several may accumulate over a session, and they
            // are joined in order, which is the order they would have been read in.
            Role::System => system.push(message.content.text()),
            Role::User => push(&mut converted, "user", user_blocks(message)),
            Role::Tool => push(&mut converted, "user", tool_result_blocks(message)),
            Role::Assistant => push(&mut converted, "assistant", assistant_blocks(message)),
        }
    }

    // Two breakpoints, which is what this conversation is shaped like. The prompt and the tool
    // schemas are the same bytes on every round of every turn, and the messages in front of the
    // last one are the same bytes they were a round ago: a turn re-sends its whole history each
    // round, so without these the service reads all of it again every time. One session grew to
    // a hundred thousand tokens over twenty-seven rounds, and paid for every one of them at full
    // price, in seconds as much as in money.
    //
    // The prefix runs tools, then system, then messages, so the breakpoint on the system prompt
    // covers the schemas too and the one on the last message covers everything.
    let system = (!system.is_empty()).then(|| {
        vec![
            SystemBlock::Text(system.join("\n\n")),
            SystemBlock::CachePoint(CachePoint::new()),
        ]
    });
    mark_the_end(&mut converted);

    ConverseRequest {
        system,
        messages: converted,
        inference_config: InferenceConfig {
            max_tokens: MAX_TOKENS,
        },
        // An empty list is no list. This API refuses `tools: []` rather than reading it as a
        // request to use none, so a turn offering nothing has to omit the field entirely.
        tool_config: tools
            .filter(|tools| !tools.is_empty())
            .map(|tools| ToolConfig {
                tools: tools.iter().map(tool_from).collect(),
            }),
        additional_model_request_fields: None,
    }
}

/// Put a breakpoint at the end of the conversation.
///
/// Rolling rather than fixed: it moves to the end on every request, so each round writes the
/// round before it into the cache and reads back everything older. A conversation ending in an
/// image or a tool call is left with the one breakpoint on the system prompt, which costs a cache
/// write and nothing else.
fn mark_the_end(messages: &mut [ConverseMessage]) {
    let Some(last) = messages.last_mut() else {
        return;
    };
    if matches!(
        last.content.last(),
        Some(Block::Text(_) | Block::ToolResult(_))
    ) {
        last.content.push(Block::CachePoint(CachePoint::new()));
    }
}

/// Add blocks to the conversation, merging into the previous turn when the role repeats.
///
/// The API refuses two consecutive turns of the same role, and this conversion creates them: two
/// tool results in a row were two `Role::Tool` messages, and both become user turns.
fn push(messages: &mut Vec<ConverseMessage>, role: &'static str, blocks: Vec<Block>) {
    if blocks.is_empty() {
        return;
    }
    match messages.last_mut() {
        Some(last) if last.role == role => last.content.extend(blocks),
        _ => messages.push(ConverseMessage {
            role,
            content: blocks,
        }),
    }
}

fn user_blocks(message: &bravebot_aichat::protocol::Message) -> Vec<Block> {
    use bravebot_aichat::protocol::{Content, Part};

    match &message.content {
        Content::Text(text) => text_block(text),
        Content::Parts(parts) => {
            let mut blocks = Vec::new();
            let mut attachments = 0;
            for part in parts {
                match part {
                    // Through the same guard a whole turn's text goes through: a part carrying no
                    // text is an empty block, which the API rejects.
                    Part::Text { text } => blocks.extend(text_block(text)),
                    // A data URI, which is the only form attachments take here:
                    // `data:<media>;base64,<data>`. Anything else is dropped rather than sent as a
                    // link, because asking the service to fetch a URL is an effect nobody endorsed.
                    Part::ImageUrl { image_url } => {
                        attachments += 1;
                        blocks.extend(attachment_block(&image_url.url, attachments));
                    }
                }
            }
            blocks
        }
    }
}

fn tool_result_blocks(message: &bravebot_aichat::protocol::Message) -> Vec<Block> {
    let Some(id) = message.tool_call_id.clone() else {
        // A result with no id cannot be matched to its call, and the API rejects one. Sent as plain
        // text it would read as something the user said, so it is dropped.
        return Vec::new();
    };
    vec![Block::ToolResult(ToolResult {
        tool_use_id: id,
        content: vec![ToolResultBlock::Text(message.content.text())],
    })]
}

fn assistant_blocks(message: &bravebot_aichat::protocol::Message) -> Vec<Block> {
    let mut blocks = text_block(&message.content.text());
    for call in message.tool_calls.iter().flatten() {
        blocks.push(Block::ToolUse(ToolUse {
            tool_use_id: call.id.clone(),
            name: call.function.name.clone(),
            // Arguments cross as a string in the other protocol and as an object here. An
            // unparseable string becomes an empty object: the call is preserved so it can still be
            // answered, which keeps the conversation well-formed.
            input: serde_json::from_str(&call.function.arguments).unwrap_or_else(|_| json!({})),
        }));
    }
    blocks
}

/// A text block, or nothing at all for empty text.
///
/// The API rejects an empty text block, and an assistant turn that only asked for tools has no text.
fn text_block(text: &str) -> Vec<Block> {
    if text.is_empty() {
        Vec::new()
    } else {
        vec![Block::Text(text.to_string())]
    }
}

/// An attachment block from a data URI, or nothing if it is not one this API takes.
///
/// A picture and a document are different members of this API's union and it takes different
/// formats for each, so which one a media type names decides which block it becomes. `position` is
/// the attachment's place in the turn, which is the only name a document can be given: a data URI
/// carries no filename.
fn attachment_block(url: &str, position: usize) -> Option<Block> {
    let rest = url.strip_prefix("data:")?;
    let (media_type, data) = rest.split_once(";base64,")?;
    if data.is_empty() {
        return None;
    }

    if let Some((_, format)) = IMAGE_FORMATS.iter().find(|(known, _)| *known == media_type) {
        return Some(Block::Image(Image {
            format,
            source: AttachmentSource {
                bytes: data.to_string(),
            },
        }));
    }

    let (_, format) = DOCUMENT_FORMATS
        .iter()
        .find(|(known, _)| *known == media_type)?;
    Some(Block::Document(Document {
        format,
        name: format!("attachment {position}"),
        source: AttachmentSource {
            bytes: data.to_string(),
        },
    }))
}

fn tool_from(tool: &bravebot_aichat::protocol::Tool) -> ToolEntry {
    ToolEntry::ToolSpec(ToolSpec {
        name: tool.function.name.clone(),
        description: tool.function.description.clone(),
        input_schema: ToolSchema {
            json: tool.function.parameters.clone(),
        },
    })
}

/// The text and the calls in a finished reply, in the shapes the agent expects back.
pub fn parts_of(blocks: &[ReplyBlock]) -> (String, Vec<bravebot_aichat::protocol::ToolCall>) {
    use bravebot_aichat::protocol::{ToolCall, ToolCallFunction};

    let mut text = String::new();
    let mut calls = Vec::new();

    for block in blocks {
        match block {
            ReplyBlock::Text { text: piece } => text.push_str(piece),
            ReplyBlock::ToolUse { tool_use } => calls.push(ToolCall {
                id: Some(tool_use.tool_use_id.clone()),
                function: ToolCallFunction {
                    name: tool_use.name.clone(),
                    // Back to a string, which is how the rest of the agent carries arguments.
                    arguments: Some(tool_use.input.to_string()),
                },
            }),
            ReplyBlock::Other(_) => {}
        }
    }

    (text, calls)
}

impl From<BedrockUsage> for bravebot_aichat::protocol::Usage {
    fn from(usage: BedrockUsage) -> Self {
        Self {
            // Everything the request carried, whoever read it. This API states `inputTokens`
            // net of the cache, so a round that hit it reports a fraction of the prompt it
            // actually sent, and a context gauge fed that figure would show a conversation
            // shrinking as it grew. What the three add up to is the prompt.
            prompt_tokens: usage.input_tokens
                + usage.cache_read_input_tokens
                + usage.cache_write_input_tokens,
            completion_tokens: usage.output_tokens,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bravebot_aichat::protocol::{
        ImageUrl, Message, Part, Tool, ToolCallRequest, ToolCallRequestFunction,
    };

    fn body_of(request: &ConverseRequest) -> Value {
        serde_json::to_value(request).expect("serialises")
    }

    /// A system turn is a top-level field here, not a message. Sent as one it would be a user turn
    /// the model reads as something the person said.
    #[test]
    fn a_system_turn_becomes_the_top_level_field() {
        let request = request_from(
            &[Message::system("be helpful"), Message::user("hello")],
            None,
        );
        let json = body_of(&request);
        assert_eq!(json["system"][0]["text"], "be helpful");
        assert_eq!(request.messages.len(), 1);
        assert_eq!(request.messages[0].role, "user");
    }

    /// A build nobody has asked for a level must send the body it always sent, or adding the
    /// field would change every request to an account that has never seen it.
    #[test]
    fn a_request_nobody_asked_a_level_of_carries_no_output_config() {
        let request = request_from(&[Message::user("hello")], None);
        assert!(
            body_of(&request)
                .get("additionalModelRequestFields")
                .is_none()
        );
    }

    /// This API states the level inside an object of its own, which is not how the other backend
    /// reads the same choice.
    #[test]
    fn a_level_is_sent_inside_the_object_this_api_states() {
        let request = request_from(&[Message::user("hello")], None).with_effort(Some(Effort::Max));
        let json = body_of(&request);
        assert_eq!(
            json["additionalModelRequestFields"]["output_config"]["effort"],
            "max"
        );
    }

    /// Several system turns accumulate over a session. They are joined in the order they would have
    /// been read, because a later instruction qualifying an earlier one depends on that order.
    #[test]
    fn several_system_turns_are_joined_in_order() {
        let request = request_from(
            &[
                Message::system("first"),
                Message::system("second"),
                Message::user("hello"),
            ],
            None,
        );
        assert_eq!(body_of(&request)["system"][0]["text"], "first\n\nsecond");
    }

    /// The API refuses two consecutive turns of the same role, and this conversion creates them:
    /// two tool results in a row are two messages that both become user turns.
    #[test]
    fn consecutive_tool_results_merge_into_one_turn() {
        let request = request_from(
            &[
                Message::user("go"),
                Message::assistant_calling(
                    "",
                    vec![
                        ToolCallRequest {
                            id: "a".into(),
                            kind: "function".into(),
                            function: ToolCallRequestFunction {
                                name: "one".into(),
                                arguments: "{}".into(),
                            },
                        },
                        ToolCallRequest {
                            id: "b".into(),
                            kind: "function".into(),
                            function: ToolCallRequestFunction {
                                name: "two".into(),
                                arguments: "{}".into(),
                            },
                        },
                    ],
                ),
                Message::tool_result("a", "first result"),
                Message::tool_result("b", "second result"),
            ],
            None,
        );
        let roles: Vec<&str> = request.messages.iter().map(|m| m.role).collect();
        assert_eq!(roles, ["user", "assistant", "user"]);
        let results = request.messages[2]
            .content
            .iter()
            .filter(|block| matches!(block, Block::ToolResult(_)))
            .count();
        assert_eq!(results, 2, "both results belong to the one turn");
    }

    /// A call and the result answering it are matched by id. Losing it would leave the model unable
    /// to tell which of several calls was answered.
    #[test]
    fn a_tool_result_keeps_the_id_of_the_call_it_answers() {
        let request = request_from(&[Message::tool_result("call-1", "the output")], None);
        match &request.messages[0].content[0] {
            Block::ToolResult(result) => {
                assert_eq!(result.tool_use_id, "call-1");
                assert_eq!(
                    result.content,
                    vec![ToolResultBlock::Text("the output".to_string())]
                );
            }
            other => panic!("expected a tool result, got {other:?}"),
        }
    }

    /// A result with no id cannot be matched to its call and the API rejects it. Sent as plain text
    /// it would read as something the user said, which is the one thing it must not become.
    #[test]
    fn a_tool_result_without_an_id_is_dropped_rather_than_sent_as_a_user_turn() {
        let orphan = Message {
            role: bravebot_aichat::protocol::Role::Tool,
            content: "output with no call".to_string().into(),
            tool_calls: None,
            tool_call_id: None,
        };
        let request = request_from(&[orphan], None);
        assert!(request.messages.is_empty());
    }

    /// Arguments are a string in one protocol and an object in the other. Sent as a string the model
    /// receives a quoted blob where a structure was expected.
    #[test]
    fn tool_arguments_cross_from_a_string_to_an_object() {
        let request = request_from(
            &[Message::assistant_calling(
                "",
                vec![ToolCallRequest {
                    id: "a".into(),
                    kind: "function".into(),
                    function: ToolCallRequestFunction {
                        name: "read".into(),
                        arguments: r#"{"path":"src/lib.rs"}"#.into(),
                    },
                }],
            )],
            None,
        );
        match &request.messages[0].content[0] {
            Block::ToolUse(call) => assert_eq!(call.input["path"], "src/lib.rs"),
            other => panic!("expected a tool call, got {other:?}"),
        }
    }

    /// A model can emit arguments that will not parse. Dropping the call would leave a conversation
    /// where the next turn answers something that was never asked, so it is kept and sent empty.
    #[test]
    fn unparseable_arguments_become_an_empty_object_rather_than_dropping_the_call() {
        let request = request_from(
            &[Message::assistant_calling(
                "",
                vec![ToolCallRequest {
                    id: "a".into(),
                    kind: "function".into(),
                    function: ToolCallRequestFunction {
                        name: "read".into(),
                        arguments: "{not json".into(),
                    },
                }],
            )],
            None,
        );
        match &request.messages[0].content[0] {
            Block::ToolUse(call) => {
                assert_eq!(call.input, json!({}));
                assert_eq!(call.tool_use_id, "a");
            }
            other => panic!("expected the call to survive, got {other:?}"),
        }
    }

    /// An assistant turn that only asked for tools has no text, and the API rejects an empty text
    /// block.
    #[test]
    fn an_assistant_turn_with_no_text_sends_no_text_block() {
        let request = request_from(
            &[Message::assistant_calling(
                "",
                vec![ToolCallRequest {
                    id: "a".into(),
                    kind: "function".into(),
                    function: ToolCallRequestFunction {
                        name: "one".into(),
                        arguments: "{}".into(),
                    },
                }],
            )],
            None,
        );
        assert_eq!(request.messages[0].content.len(), 1);
        assert!(matches!(
            request.messages[0].content[0],
            Block::ToolUse { .. }
        ));
    }

    /// An attachment arrives as a data URI and crosses as inline base64. The alternative, sending a
    /// link, asks the service to fetch a URL the conversation chose.
    #[test]
    fn an_attached_image_crosses_as_inline_data() {
        let request = request_from(
            &[Message::user_parts(vec![
                Part::Text {
                    text: "look".into(),
                },
                Part::ImageUrl {
                    image_url: ImageUrl {
                        url: "data:image/png;base64,AAAA".into(),
                    },
                },
            ])],
            None,
        );
        match &request.messages[0].content[1] {
            Block::Image(image) => {
                assert_eq!(image.format, "png", "this API names a format, not a type");
                assert_eq!(image.source.bytes, "AAAA");
            }
            other => panic!("expected an image, got {other:?}"),
        }
    }

    /// Anything that is not inline data is dropped rather than turned into a fetch: a URL in a
    /// conversation is a routing field nobody endorsed.
    #[test]
    fn an_image_that_is_not_inline_data_is_dropped() {
        for url in [
            "https://example.invalid/x.png",
            "data:image/png,notbase64",
            "data:;base64,AAAA",
            "data:image/png;base64,",
            "not a url",
        ] {
            let request = request_from(
                &[Message::user_parts(vec![
                    Part::Text {
                        text: "look".into(),
                    },
                    Part::ImageUrl {
                        image_url: ImageUrl { url: url.into() },
                    },
                ])],
                None,
            );
            assert_eq!(
                request.messages[0].content.len(),
                2,
                "{url} was sent as an image"
            );
        }
    }

    /// This API takes four image formats and refuses the rest, so a media type outside them is
    /// dropped here rather than failing at the far end for a reason nothing local explained.
    #[test]
    fn an_image_in_a_format_this_api_does_not_take_is_dropped() {
        for media_type in ["image/svg+xml", "image/bmp", "image/PNG"] {
            let request = request_from(
                &[Message::user_parts(vec![
                    Part::Text {
                        text: "look".into(),
                    },
                    Part::ImageUrl {
                        image_url: ImageUrl {
                            url: format!("data:{media_type};base64,AAAA"),
                        },
                    },
                ])],
                None,
            );
            assert_eq!(
                request.messages[0].content.len(),
                2,
                "{media_type} was sent as an image"
            );
        }
    }

    /// A picture and a document are different members of this API's union, and it refuses a PDF
    /// sent as a picture. The agent carries PDFs, so one arriving as a picture part has to cross as
    /// the document block this API states, or the turn asks about a file the model never received.
    #[test]
    fn a_pdf_crosses_as_a_document_rather_than_a_picture() {
        let request = request_from(
            &[Message::user_parts(vec![
                Part::Text {
                    text: "what does it say".into(),
                },
                Part::ImageUrl {
                    image_url: ImageUrl {
                        url: "data:application/pdf;base64,AAAA".into(),
                    },
                },
            ])],
            None,
        );
        match &request.messages[0].content[1] {
            Block::Document(document) => {
                assert_eq!(document.format, "pdf");
                assert_eq!(document.source.bytes, "AAAA");
                assert!(!document.name.is_empty(), "this API requires a name");
            }
            other => panic!("expected a document, got {other:?}"),
        }
    }

    /// A turn can be an attachment and nothing else, which is what a processor asked about a file
    /// sends. Every block dropped leaves an empty turn, and a request whose message list is empty
    /// is refused outright, so the answer is about a file nobody was shown.
    #[test]
    fn a_turn_whose_only_part_is_an_attachment_is_still_a_turn() {
        for url in [
            "data:application/pdf;base64,AAAA",
            "data:image/png;base64,AAAA",
        ] {
            let request = request_from(
                &[Message::user_parts(vec![Part::ImageUrl {
                    image_url: ImageUrl { url: url.into() },
                }])],
                None,
            );
            assert_eq!(request.messages.len(), 1, "{url} left no turn at all");
            assert_eq!(request.messages[0].content.len(), 1, "{url}");
        }
    }

    /// Dropping a file without typing anything leaves a part holding no text, and this API refuses
    /// an empty text block as readily as it refuses an empty turn.
    #[test]
    fn a_part_carrying_no_text_sends_no_block() {
        let request = request_from(
            &[Message::user_parts(vec![
                Part::Text {
                    text: String::new(),
                },
                Part::ImageUrl {
                    image_url: ImageUrl {
                        url: "data:image/png;base64,AAAA".into(),
                    },
                },
            ])],
            None,
        );
        assert_eq!(request.messages[0].content.len(), 1);
        assert!(matches!(request.messages[0].content[0], Block::Image(_)));
    }

    /// A tool definition is nested under `function` in one protocol and under a spec of its own
    /// here, with the schema under a different name again.
    #[test]
    fn a_tool_definition_is_flattened() {
        let tools = vec![Tool::function(
            "read_file",
            "Read a file",
            json!({"type": "object", "properties": {"path": {"type": "string"}}}),
        )];
        let request = request_from(&[Message::user("hi")], Some(&tools));
        let spec = &body_of(&request)["toolConfig"]["tools"][0]["toolSpec"];
        assert_eq!(spec["name"], "read_file");
        assert_eq!(spec["description"], "Read a file");
        assert_eq!(spec["inputSchema"]["json"]["type"], "object");
    }

    /// A cached round sent every one of those tokens, whoever ended up reading them.
    #[test]
    fn cached_tokens_are_counted_as_the_prompt_they_were() {
        let usage: bravebot_aichat::protocol::Usage = BedrockUsage {
            input_tokens: 12,
            output_tokens: 40,
            cache_read_input_tokens: 900,
            cache_write_input_tokens: 88,
        }
        .into();
        assert_eq!(
            usage.prompt_tokens, 1000,
            "a cached round read as a smaller prompt than it sent"
        );
        assert_eq!(usage.completion_tokens, 40);
    }

    /// The prompt and the schemas are the same bytes every round, so the request says so.
    #[test]
    fn the_system_prompt_carries_a_breakpoint() {
        let request = request_from(
            &[Message::system("be helpful"), Message::user("hello")],
            None,
        );
        let system = request.system.as_deref().expect("a system block");
        assert_eq!(
            system.last(),
            Some(&SystemBlock::CachePoint(CachePoint::new())),
            "the prefix every round shares was not marked"
        );
    }

    /// Rolling, so each round writes the round before it into the cache and reads back the rest.
    #[test]
    fn the_last_block_of_the_conversation_carries_a_breakpoint() {
        let request = request_from(
            &[
                Message::user("first"),
                Message::assistant("an answer"),
                Message::user("second"),
            ],
            None,
        );
        assert_eq!(
            request.messages.last().and_then(|m| m.content.last()),
            Some(&Block::CachePoint(CachePoint::new())),
            "the end of the conversation was not marked"
        );
        assert!(
            !request.messages[0]
                .content
                .iter()
                .any(|block| matches!(block, Block::CachePoint(_))),
            "an earlier turn was marked as well, spending a breakpoint on nothing"
        );
    }

    /// A round ends on tool results as often as on words, and that is exactly the prefix the next
    /// round wants back.
    #[test]
    fn a_conversation_ending_in_a_tool_result_is_marked_too() {
        let request = request_from(&[Message::tool_result("call-1", "the output")], None);
        assert_eq!(
            request.messages[0].content.last(),
            Some(&Block::CachePoint(CachePoint::new())),
            "a round ending in a tool result was not marked"
        );
    }

    /// A reply never carries one, so reading one back must not require it.
    #[test]
    fn a_reply_without_a_breakpoint_still_parses() {
        let blocks: Vec<ReplyBlock> =
            serde_json::from_value(json!([{"text": "hello"}])).expect("parses");
        assert!(matches!(blocks.first(), Some(ReplyBlock::Text { .. })));
    }

    /// The reply's text and calls come back in the shapes the turn loop already handles.
    #[test]
    fn a_reply_is_read_back_into_text_and_calls() {
        let blocks: Vec<ReplyBlock> = serde_json::from_value(json!([
            {"text": "I will read it"},
            {"toolUse": {"toolUseId": "call-1", "name": "read_file", "input": {"path": "src/lib.rs"}}},
        ]))
        .expect("parses");

        let (text, calls) = parts_of(&blocks);
        assert_eq!(text, "I will read it");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id.as_deref(), Some("call-1"));
        assert_eq!(calls[0].function.name, "read_file");
        assert_eq!(
            calls[0].function.arguments.as_deref(),
            Some(r#"{"path":"src/lib.rs"}"#)
        );
    }

    /// A call taking no arguments may name none. Read strictly, that block matches no member of
    /// the union, is kept as an unmodelled one, and the call the model asked for is silently gone.
    #[test]
    fn a_reply_calling_a_tool_with_no_arguments_keeps_the_call() {
        let blocks: Vec<ReplyBlock> = serde_json::from_value(json!([
            {"toolUse": {"toolUseId": "call-1", "name": "list_files"}},
        ]))
        .expect("parses");
        let (_, calls) = parts_of(&blocks);
        assert_eq!(calls.len(), 1, "the call was dropped");
        assert_eq!(calls[0].function.name, "list_files");
        assert_eq!(
            calls[0].function.arguments.as_deref(),
            Some("{}"),
            "the buffered path disagreed with the streamed one"
        );
    }

    /// The union of block kinds grows with what the models this API fronts can produce. One this
    /// does not model must not fail a reply whose text and calls are all there.
    #[test]
    fn a_block_kind_this_does_not_model_leaves_the_rest_of_the_reply_readable() {
        let blocks: Vec<ReplyBlock> = serde_json::from_value(json!([
            {"reasoningContent": {"reasoningText": {"text": "thinking"}}},
            {"text": "the answer"},
        ]))
        .expect("parses");
        assert_eq!(parts_of(&blocks).0, "the answer");
    }

    /// A reply can arrive as several text blocks, and they are one answer.
    #[test]
    fn several_text_blocks_join_into_one_answer() {
        let blocks: Vec<ReplyBlock> =
            serde_json::from_value(json!([{"text": "first "}, {"text": "second"}]))
                .expect("parses");
        assert_eq!(parts_of(&blocks).0, "first second");
    }

    /// The two backends count the same thing under different names, and the turn's cost is worked
    /// out from these.
    #[test]
    fn usage_crosses_to_the_shape_the_agent_records() {
        let usage: bravebot_aichat::protocol::Usage = BedrockUsage {
            input_tokens: 100,
            output_tokens: 20,
            cache_read_input_tokens: 0,
            cache_write_input_tokens: 0,
        }
        .into();
        assert_eq!(usage.prompt_tokens, 100);
        assert_eq!(usage.completion_tokens, 20);
        assert_eq!(usage.total(), 120);
    }

    /// The body is the one this API states for every provider it hosts. A field only one of them
    /// defines, sent at the top level, is a request the rest of them refuse.
    #[test]
    fn the_request_names_no_provider_of_its_own() {
        let tools = vec![Tool::function("read_file", "Read a file", json!({}))];
        let request =
            request_from(&[Message::user("hi")], Some(&tools)).with_effort(Some(Effort::High));
        let json = body_of(&request);
        let mut keys: Vec<&str> = json
            .as_object()
            .expect("an object")
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "additionalModelRequestFields",
                "inferenceConfig",
                "messages",
                "toolConfig",
            ],
            "a field outside this API's own shape reached the top level"
        );
        assert!(
            json["inferenceConfig"]["maxTokens"].as_u64().unwrap_or(0) > 0,
            "the reply has no ceiling"
        );
    }

    /// A conversation with no tools must omit the field rather than send an empty list, which this
    /// API refuses outright.
    #[test]
    fn no_tools_omits_the_field() {
        for tools in [None, Some(&[][..])] {
            let request = request_from(&[Message::user("hi")], tools);
            assert!(
                request.tool_config.is_none(),
                "{tools:?} became a tool list"
            );
            let body = serde_json::to_string(&request).expect("serialises");
            assert!(!body.contains("tool"), "{body}");
        }
    }

    /// Events this code does not model must not fail a turn: the API sends several, and may add more.
    #[test]
    fn an_unknown_stream_event_is_ignored_rather_than_failing() {
        assert!(stream_event("somethingNew", br#"{"detail":{}}"#).is_none());
        assert!(stream_event("messageStart", br#"{"role":"assistant"}"#).is_none());
    }

    /// The events that carry a reply have to decode, since a turn is assembled from them.
    #[test]
    fn the_events_that_carry_a_reply_decode() {
        let text = stream_event(
            "contentBlockDelta",
            br#"{"contentBlockIndex":0,"delta":{"text":"hi"}}"#,
        )
        .expect("decodes");
        assert!(matches!(
            text,
            StreamEvent::ContentBlockDelta {
                index: 0,
                delta: Delta::Text { .. },
            }
        ));

        let start = stream_event(
            "contentBlockStart",
            br#"{"contentBlockIndex":1,"start":{"toolUse":{"toolUseId":"a","name":"read"}}}"#,
        )
        .expect("decodes");
        match start {
            StreamEvent::ContentBlockStart {
                index,
                start: BlockStart::ToolUse { tool_use },
            } => {
                assert_eq!(index, 1);
                assert_eq!(tool_use.tool_use_id, "a");
                assert_eq!(tool_use.name, "read");
            }
            other => panic!("expected a tool call opening, got {other:?}"),
        }

        let stop = stream_event("messageStop", br#"{"stopReason":"end_turn"}"#).expect("decodes");
        match stop {
            StreamEvent::MessageStop { stop_reason } => {
                assert_eq!(stop_reason.as_deref(), Some("end_turn"))
            }
            other => panic!("expected a stop, got {other:?}"),
        }

        let counts = stream_event(
            "metadata",
            br#"{"usage":{"inputTokens":11,"outputTokens":7,"totalTokens":18}}"#,
        )
        .expect("decodes");
        match counts {
            StreamEvent::Metadata { usage } => {
                let usage = usage.expect("usage");
                assert_eq!(usage.input_tokens, 11);
                assert_eq!(usage.output_tokens, 7);
            }
            other => panic!("expected the counts, got {other:?}"),
        }
    }

    /// Tool arguments arrive as a string of JSON in pieces, not as an object, so a delta that
    /// carried one is read as the fragment it is.
    #[test]
    fn a_tool_call_delta_carries_a_fragment_of_its_arguments() {
        let delta = stream_event(
            "contentBlockDelta",
            br#"{"contentBlockIndex":2,"delta":{"toolUse":{"input":"{\"path\""}}}"#,
        )
        .expect("decodes");
        match delta {
            StreamEvent::ContentBlockDelta {
                index,
                delta: Delta::ToolUse { tool_use },
            } => {
                assert_eq!(index, 2);
                assert_eq!(tool_use.input, r#"{"path""#);
            }
            other => panic!("expected a tool call fragment, got {other:?}"),
        }
    }

    /// A reply with nothing in it must read as one rather than failing to parse, so the turn can
    /// say so instead of reporting an unexpected response.
    #[test]
    fn a_reply_carrying_no_message_still_parses() {
        let reply: ConverseResponse =
            serde_json::from_str(r#"{"stopReason":"end_turn"}"#).expect("parses");
        assert!(reply.output.is_none());
        assert_eq!(reply.stop_reason.as_deref(), Some("end_turn"));
    }
}
