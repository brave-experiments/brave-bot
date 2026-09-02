//! Wire types for the Anthropic Messages API as Bedrock serves it, and the translation to and from
//! the shapes the rest of this agent speaks.
//!
//! The agent's conversation is held in OpenAI-compatible types, because that is what the other
//! backend speaks. This module converts, in one place, rather than teaching the turn loop two
//! protocols. The differences that matter:
//!
//! - The system prompt is a top-level field, not a message with a role.
//! - Tool calls and their results are content blocks inside user and assistant turns, not a
//!   separate role with an id alongside.
//! - Arguments arrive as a JSON object, not as a string holding one.
//! - Usage counts `input_tokens` and `output_tokens` rather than prompt and completion.
//!
//! Nothing here inspects content to make a decision. Text is moved between shapes and handed on with
//! whatever label it arrived under.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// The API version Bedrock requires in every request body.
///
/// Not a date this code cares about: it is the string that selects the request shape below, and
/// changing it would mean changing these types.
pub const ANTHROPIC_VERSION: &str = "bedrock-2023-05-31";

/// How many tokens a reply may run to before it is cut off.
///
/// Required by the API, which has no default. Chosen to be larger than any single reply a turn here
/// produces: a tool call and its reasoning, not a document. A cut-off reply is reported as one
/// rather than silently truncated, but the cheaper fix is to not hit it.
pub const MAX_TOKENS: u64 = 8_192;

/// A request to Bedrock.
#[derive(Debug, Clone, Serialize)]
pub struct InvokeRequest {
    pub anthropic_version: &'static str,
    pub max_tokens: u64,
    /// The system prompt, hoisted out of the message list.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    pub messages: Vec<BedrockMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<BedrockTool>>,
}

/// Serialised only: `role` is a fixed string this crate chooses, never one it reads back.
#[derive(Debug, Clone, Serialize)]
pub struct BedrockMessage {
    pub role: &'static str,
    pub content: Vec<Block>,
}

/// One content block, in either direction.
///
/// Untagged on the way in and tagged on the way out is not an option with one type, so every variant
/// names its own `type`, which is what the API does too.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Block {
    Text {
        text: String,
    },
    Image {
        source: ImageSource,
    },
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
    },
}

/// Where an image block's bytes are.
///
/// `kind` is owned rather than borrowed because a block is both sent and read back: the type has to
/// round-trip, and a `&'static str` cannot be deserialised into.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageSource {
    #[serde(rename = "type")]
    pub kind: String,
    pub media_type: String,
    pub data: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct BedrockTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// A complete reply.
#[derive(Debug, Clone, Deserialize)]
pub struct InvokeResponse {
    #[serde(default)]
    pub content: Vec<Block>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub usage: Option<BedrockUsage>,
    /// Why the model stopped. `max_tokens` here means the reply was cut off.
    #[serde(default)]
    pub stop_reason: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
pub struct BedrockUsage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
}

/// The stop reason meaning the reply hit the ceiling rather than finishing.
pub const STOP_REASON_MAX_TOKENS: &str = "max_tokens";

/// One frame of a streamed reply.
///
/// Only the events that carry text, a tool call, or a count. The API sends several others
/// (`message_start`, `content_block_stop`, `ping`) that say nothing this needs.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamEvent {
    ContentBlockStart {
        index: usize,
        content_block: Block,
    },
    ContentBlockDelta {
        index: usize,
        delta: Delta,
    },
    MessageStart {
        message: StreamedMessageStart,
    },
    MessageDelta {
        #[serde(default)]
        delta: MessageDeltaBody,
        #[serde(default)]
        usage: Option<BedrockUsage>,
    },
    MessageStop,
    /// Anything else the API sends, so an addition does not fail a turn.
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StreamedMessageStart {
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub usage: Option<BedrockUsage>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct MessageDeltaBody {
    #[serde(default)]
    pub stop_reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Delta {
    TextDelta {
        text: String,
    },
    /// Tool arguments arrive as JSON in pieces, which have to be concatenated before parsing.
    InputJsonDelta {
        partial_json: String,
    },
    #[serde(other)]
    Other,
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
) -> InvokeRequest {
    use bravebot_aichat::protocol::Role;

    let mut system: Vec<String> = Vec::new();
    let mut converted: Vec<BedrockMessage> = Vec::new();

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

    InvokeRequest {
        anthropic_version: ANTHROPIC_VERSION,
        max_tokens: MAX_TOKENS,
        system: (!system.is_empty()).then(|| system.join("\n\n")),
        messages: converted,
        tools: tools.map(|tools| tools.iter().map(tool_from).collect()),
    }
}

/// Add blocks to the conversation, merging into the previous turn when the role repeats.
///
/// The API refuses two consecutive turns of the same role, and this conversion creates them: two
/// tool results in a row were two `Role::Tool` messages, and both become user turns.
fn push(messages: &mut Vec<BedrockMessage>, role: &'static str, blocks: Vec<Block>) {
    if blocks.is_empty() {
        return;
    }
    match messages.last_mut() {
        Some(last) if last.role == role => last.content.extend(blocks),
        _ => messages.push(BedrockMessage {
            role,
            content: blocks,
        }),
    }
}

fn user_blocks(message: &bravebot_aichat::protocol::Message) -> Vec<Block> {
    use bravebot_aichat::protocol::{Content, Part};

    match &message.content {
        Content::Text(text) => text_block(text),
        Content::Parts(parts) => parts
            .iter()
            .filter_map(|part| match part {
                Part::Text { text } => Some(Block::Text { text: text.clone() }),
                // A data URI, which is the only form attachments take here: `data:<media>;base64,<data>`.
                // Anything else is dropped rather than sent as a link, because asking the service to
                // fetch a URL is an effect nobody endorsed.
                Part::ImageUrl { image_url } => image_block(&image_url.url),
            })
            .collect(),
    }
}

fn tool_result_blocks(message: &bravebot_aichat::protocol::Message) -> Vec<Block> {
    let Some(id) = message.tool_call_id.clone() else {
        // A result with no id cannot be matched to its call, and the API rejects one. Sent as plain
        // text it would read as something the user said, so it is dropped.
        return Vec::new();
    };
    vec![Block::ToolResult {
        tool_use_id: id,
        content: message.content.text(),
    }]
}

fn assistant_blocks(message: &bravebot_aichat::protocol::Message) -> Vec<Block> {
    let mut blocks = text_block(&message.content.text());
    for call in message.tool_calls.iter().flatten() {
        blocks.push(Block::ToolUse {
            id: call.id.clone(),
            name: call.function.name.clone(),
            // Arguments cross as a string in the other protocol and as an object here. An
            // unparseable string becomes an empty object: the call is preserved so it can still be
            // answered, which keeps the conversation well-formed.
            input: serde_json::from_str(&call.function.arguments).unwrap_or_else(|_| json!({})),
        });
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
        vec![Block::Text {
            text: text.to_string(),
        }]
    }
}

/// An image block from a data URI, or nothing if it is not one.
fn image_block(url: &str) -> Option<Block> {
    let rest = url.strip_prefix("data:")?;
    let (media_type, data) = rest.split_once(";base64,")?;
    if media_type.is_empty() || data.is_empty() {
        return None;
    }
    Some(Block::Image {
        source: ImageSource {
            kind: "base64".to_string(),
            media_type: media_type.to_string(),
            data: data.to_string(),
        },
    })
}

fn tool_from(tool: &bravebot_aichat::protocol::Tool) -> BedrockTool {
    BedrockTool {
        name: tool.function.name.clone(),
        description: tool.function.description.clone(),
        input_schema: tool.function.parameters.clone(),
    }
}

/// The text and the calls in a finished reply, in the shapes the agent expects back.
pub fn parts_of(blocks: &[Block]) -> (String, Vec<bravebot_aichat::protocol::ToolCall>) {
    use bravebot_aichat::protocol::{ToolCall, ToolCallFunction};

    let mut text = String::new();
    let mut calls = Vec::new();

    for block in blocks {
        match block {
            Block::Text { text: piece } => text.push_str(piece),
            Block::ToolUse { id, name, input } => calls.push(ToolCall {
                id: Some(id.clone()),
                function: ToolCallFunction {
                    name: name.clone(),
                    // Back to a string, which is how the rest of the agent carries arguments.
                    arguments: Some(input.to_string()),
                },
            }),
            // Neither is something a reply contains; both are ours to send.
            Block::ToolResult { .. } | Block::Image { .. } => {}
        }
    }

    (text, calls)
}

impl From<BedrockUsage> for bravebot_aichat::protocol::Usage {
    fn from(usage: BedrockUsage) -> Self {
        Self {
            prompt_tokens: usage.input_tokens,
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

    /// A system turn is a top-level field here, not a message. Sent as one it would be a user turn
    /// the model reads as something the person said.
    #[test]
    fn a_system_turn_becomes_the_top_level_field() {
        let request = request_from(
            &[Message::system("be helpful"), Message::user("hello")],
            None,
        );
        assert_eq!(request.system.as_deref(), Some("be helpful"));
        assert_eq!(request.messages.len(), 1);
        assert_eq!(request.messages[0].role, "user");
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
        assert_eq!(request.system.as_deref(), Some("first\n\nsecond"));
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
        assert_eq!(
            request.messages[2].content.len(),
            2,
            "both results belong to the one turn"
        );
    }

    /// A call and the result answering it are matched by id. Losing it would leave the model unable
    /// to tell which of several calls was answered.
    #[test]
    fn a_tool_result_keeps_the_id_of_the_call_it_answers() {
        let request = request_from(&[Message::tool_result("call-1", "the output")], None);
        match &request.messages[0].content[0] {
            Block::ToolResult {
                tool_use_id,
                content,
            } => {
                assert_eq!(tool_use_id, "call-1");
                assert_eq!(content, "the output");
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
            Block::ToolUse { input, .. } => assert_eq!(input["path"], "src/lib.rs"),
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
            Block::ToolUse { input, id, .. } => {
                assert_eq!(input, &json!({}));
                assert_eq!(id, "a");
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
        assert_eq!(request.messages[0].content.len(), 2);
        match &request.messages[0].content[1] {
            Block::Image { source } => {
                assert_eq!(source.media_type, "image/png");
                assert_eq!(source.data, "AAAA");
                assert_eq!(source.kind, "base64");
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
                1,
                "{url} was sent as an image"
            );
        }
    }

    /// A tool definition is nested under `function` in one protocol and flat here, with the schema
    /// under a different name.
    #[test]
    fn a_tool_definition_is_flattened() {
        let tools = vec![Tool::function(
            "read_file",
            "Read a file",
            json!({"type": "object", "properties": {"path": {"type": "string"}}}),
        )];
        let request = request_from(&[Message::user("hi")], Some(&tools));
        let sent = request.tools.expect("tools");
        assert_eq!(sent[0].name, "read_file");
        assert_eq!(sent[0].description, "Read a file");
        assert_eq!(sent[0].input_schema["type"], "object");
    }

    /// The reply's text and calls come back in the shapes the turn loop already handles.
    #[test]
    fn a_reply_is_read_back_into_text_and_calls() {
        let blocks = vec![
            Block::Text {
                text: "I will read it".into(),
            },
            Block::ToolUse {
                id: "call-1".into(),
                name: "read_file".into(),
                input: json!({"path": "src/lib.rs"}),
            },
        ];
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

    /// A reply can arrive as several text blocks, and they are one answer.
    #[test]
    fn several_text_blocks_join_into_one_answer() {
        let blocks = vec![
            Block::Text {
                text: "first ".into(),
            },
            Block::Text {
                text: "second".into(),
            },
        ];
        assert_eq!(parts_of(&blocks).0, "first second");
    }

    /// The two backends count the same thing under different names, and the turn's cost is worked
    /// out from these.
    #[test]
    fn usage_crosses_to_the_shape_the_agent_records() {
        let usage: bravebot_aichat::protocol::Usage = BedrockUsage {
            input_tokens: 100,
            output_tokens: 20,
        }
        .into();
        assert_eq!(usage.prompt_tokens, 100);
        assert_eq!(usage.completion_tokens, 20);
        assert_eq!(usage.total(), 120);
    }

    /// The version string selects the request shape these types describe, so it must be the one the
    /// API expects.
    #[test]
    fn the_request_names_the_api_version_bedrock_requires() {
        let request = request_from(&[Message::user("hi")], None);
        assert_eq!(request.anthropic_version, "bedrock-2023-05-31");
        assert!(request.max_tokens > 0, "the API requires a ceiling");
    }

    /// A conversation with no tools must omit the field rather than send an empty list, which the
    /// API reads as a request to use no tools at all.
    #[test]
    fn no_tools_omits_the_field() {
        let request = request_from(&[Message::user("hi")], None);
        assert!(request.tools.is_none());
        let body = serde_json::to_string(&request).expect("serialises");
        assert!(!body.contains("tools"), "{body}");
    }

    /// Events this code does not model must not fail a turn: the API sends several, and may add more.
    #[test]
    fn an_unknown_stream_event_is_ignored_rather_than_failing() {
        let event: StreamEvent =
            serde_json::from_str(r#"{"type":"something_new","detail":{}}"#).expect("decodes");
        assert!(matches!(event, StreamEvent::Other));
    }

    /// The events that carry a reply have to decode, since a turn is assembled from them.
    #[test]
    fn the_events_that_carry_a_reply_decode() {
        let text: StreamEvent = serde_json::from_str(
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hi"}}"#,
        )
        .expect("decodes");
        assert!(matches!(
            text,
            StreamEvent::ContentBlockDelta {
                delta: Delta::TextDelta { .. },
                ..
            }
        ));

        let start: StreamEvent = serde_json::from_str(
            r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"a","name":"read","input":{}}}"#,
        )
        .expect("decodes");
        assert!(matches!(
            start,
            StreamEvent::ContentBlockStart {
                content_block: Block::ToolUse { .. },
                ..
            }
        ));

        let usage: StreamEvent = serde_json::from_str(
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":7}}"#,
        )
        .expect("decodes");
        match usage {
            StreamEvent::MessageDelta { usage, delta } => {
                assert_eq!(usage.expect("usage").output_tokens, 7);
                assert_eq!(delta.stop_reason.as_deref(), Some("end_turn"));
            }
            other => panic!("expected a message delta, got {other:?}"),
        }
    }
}
