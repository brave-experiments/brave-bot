//! Wire types for the OpenAI-compatible chat completions API.
//!
//! Only the fields this client uses are modelled. Unknown response fields are ignored
//! rather than rejected, so a server-side addition does not break the client.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
        }
    }
}

/// A tool the model may call, in OpenAI's function-tool shape.
#[derive(Debug, Clone, Serialize)]
pub struct Tool {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub function: ToolFunction,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolFunction {
    pub name: String,
    pub description: String,
    /// JSON Schema for the arguments.
    pub parameters: Value,
}

impl Tool {
    pub fn function(
        name: impl Into<String>,
        description: impl Into<String>,
        parameters: Value,
    ) -> Self {
        Self {
            kind: "function",
            function: ToolFunction {
                name: name.into(),
                description: description.into(),
                parameters,
            },
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<Message>,
    /// Streaming is not implemented yet; a one-shot request reads the whole body.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Tool>>,
}

impl ChatRequest {
    pub fn new(model: impl Into<String>, messages: Vec<Message>) -> Self {
        Self {
            model: model.into(),
            messages,
            stream: None,
            tools: None,
        }
    }

    pub fn with_tools(mut self, tools: Vec<Tool>) -> Self {
        self.tools = if tools.is_empty() { None } else { Some(tools) };
        self
    }
}

/// A tool call the model asked for.
///
/// The arguments are model output, so they are untrusted. Nothing here may be treated as
/// routing without a human endorsement.
#[derive(Debug, Clone, Deserialize)]
pub struct ToolCall {
    #[serde(default)]
    pub id: Option<String>,
    pub function: ToolCallFunction,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ToolCallFunction {
    pub name: String,
    /// A JSON object, delivered as a string by the API.
    #[serde(default)]
    pub arguments: Option<String>,
}

impl ToolCall {
    /// Parse the arguments.
    ///
    /// Parsing changes representation, not trust: the values remain model-supplied and
    /// must still pass the gates before they can direct anything.
    pub fn arguments(&self) -> Result<Value, serde_json::Error> {
        match self.function.arguments.as_deref() {
            Some(raw) if !raw.trim().is_empty() => serde_json::from_str(raw),
            _ => Ok(serde_json::json!({})),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ChatResponse {
    /// The model actually used. Absent on some error shapes, so it is optional.
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
pub struct Choice {
    #[serde(default)]
    pub message: Option<ResponseMessage>,
    #[serde(default)]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ResponseMessage {
    #[serde(default)]
    pub content: Option<String>,
    /// Present when the model asked to call tools.
    #[serde(default)]
    pub tool_calls: Option<Vec<ToolCall>>,
}

impl ChatResponse {
    /// Tool calls from the first choice, if the model asked for any.
    pub fn tool_calls(&self) -> &[ToolCall] {
        self.choices
            .first()
            .and_then(|c| c.message.as_ref())
            .and_then(|m| m.tool_calls.as_deref())
            .unwrap_or(&[])
    }

    /// The first choice's text, if the response carried any.
    pub fn first_content(&self) -> Option<String> {
        self.choices.first()?.message.as_ref()?.content.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_serialises_to_the_expected_shape() {
        let request = ChatRequest::new("automatic", vec![Message::user("hello")]);
        let json = serde_json::to_value(&request).unwrap();

        assert_eq!(json["model"], "automatic");
        assert_eq!(json["messages"][0]["role"], "user");
        assert_eq!(json["messages"][0]["content"], "hello");
        // Omitted rather than sent as null, since the server has its own default.
        assert!(json.get("stream").is_none());
    }

    #[test]
    fn roles_serialise_lowercase() {
        let request = ChatRequest::new(
            "automatic",
            vec![
                Message::system("be brief"),
                Message::user("hi"),
                Message::assistant("hello"),
            ],
        );
        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["messages"][0]["role"], "system");
        assert_eq!(json["messages"][1]["role"], "user");
        assert_eq!(json["messages"][2]["role"], "assistant");
    }

    #[test]
    fn a_response_yields_its_first_choice() {
        let raw = r#"{
            "model": "some-model",
            "choices": [
                {"message": {"role": "assistant", "content": "the answer"}, "finish_reason": "stop"}
            ]
        }"#;
        let parsed: ChatResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(parsed.model.as_deref(), Some("some-model"));
        assert_eq!(parsed.first_content().as_deref(), Some("the answer"));
    }

    /// The server may substitute a model, so the reported one must be readable and is
    /// not assumed to match the request.
    #[test]
    fn the_reported_model_is_preserved() {
        let raw = r#"{"model":"substituted-model","choices":[{"message":{"content":"x"}}]}"#;
        let parsed: ChatResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(parsed.model.as_deref(), Some("substituted-model"));
    }

    /// Unknown fields must not break decoding, or a server-side addition would take the
    /// client down.
    #[test]
    fn unknown_fields_are_ignored() {
        let raw = r#"{
            "model": "m",
            "id": "chatcmpl-123",
            "usage": {"prompt_tokens": 1, "completion_tokens": 2},
            "some_future_field": {"nested": true},
            "choices": [{"message": {"content": "hi"}, "index": 0}]
        }"#;
        let parsed: ChatResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(parsed.first_content().as_deref(), Some("hi"));
    }

    #[test]
    fn an_empty_choices_array_has_no_content() {
        let parsed: ChatResponse = serde_json::from_str(r#"{"choices":[]}"#).unwrap();
        assert!(parsed.first_content().is_none());
    }

    #[test]
    fn a_choice_without_content_has_no_content() {
        let raw = r#"{"choices":[{"message":{"role":"assistant"}}]}"#;
        let parsed: ChatResponse = serde_json::from_str(raw).unwrap();
        assert!(parsed.first_content().is_none());
    }

    #[test]
    fn tools_serialise_in_the_openai_function_shape() {
        let request = ChatRequest::new("automatic", vec![Message::user("hi")]).with_tools(vec![
            Tool::function(
                "read_file",
                "read a file",
                serde_json::json!({"type": "object", "properties": {"path": {"type": "string"}}}),
            ),
        ]);
        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["tools"][0]["type"], "function");
        assert_eq!(json["tools"][0]["function"]["name"], "read_file");
        assert_eq!(json["tools"][0]["function"]["parameters"]["type"], "object");
    }

    /// An empty tool list is omitted rather than sent, since some servers reject [].
    #[test]
    fn an_empty_tool_list_is_omitted() {
        let request = ChatRequest::new("automatic", vec![Message::user("hi")]).with_tools(vec![]);
        let json = serde_json::to_value(&request).unwrap();
        assert!(json.get("tools").is_none());
    }

    #[test]
    fn tool_calls_are_parsed_from_a_response() {
        let raw = r#"{"choices":[{"message":{"role":"assistant","tool_calls":[
            {"id":"call_1","type":"function","function":{"name":"read_file","arguments":"{\"path\":\"main.rs\"}"}}
        ]}}]}"#;
        let parsed: ChatResponse = serde_json::from_str(raw).unwrap();
        let calls = parsed.tool_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "read_file");
        assert_eq!(calls[0].arguments().unwrap()["path"], "main.rs");
    }

    #[test]
    fn a_response_without_tool_calls_has_none() {
        let raw = r#"{"choices":[{"message":{"content":"just text"}}]}"#;
        let parsed: ChatResponse = serde_json::from_str(raw).unwrap();
        assert!(parsed.tool_calls().is_empty());
    }

    /// Some models send empty or absent arguments for a no-argument tool.
    #[test]
    fn empty_tool_arguments_parse_as_an_empty_object() {
        let raw = r#"{"choices":[{"message":{"tool_calls":[
            {"function":{"name":"list","arguments":""}}
        ]}}]}"#;
        let parsed: ChatResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(
            parsed.tool_calls()[0].arguments().unwrap(),
            serde_json::json!({})
        );
    }

    #[test]
    fn a_missing_model_is_tolerated() {
        let raw = r#"{"choices":[{"message":{"content":"x"}}]}"#;
        let parsed: ChatResponse = serde_json::from_str(raw).unwrap();
        assert!(parsed.model.is_none());
        assert_eq!(parsed.first_content().as_deref(), Some("x"));
    }
}
