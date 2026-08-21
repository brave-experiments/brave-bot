//! Wire types for the OpenAI-compatible chat completions API.
//!
//! Only the fields this client uses are modelled. Unknown response fields are ignored
//! rather than rejected, so a server-side addition does not break the client.

use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone, Serialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<Message>,
    /// Streaming is not implemented yet; a one-shot request reads the whole body.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
}

impl ChatRequest {
    pub fn new(model: impl Into<String>, messages: Vec<Message>) -> Self {
        Self {
            model: model.into(),
            messages,
            stream: None,
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
}

impl ChatResponse {
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
    fn a_missing_model_is_tolerated() {
        let raw = r#"{"choices":[{"message":{"content":"x"}}]}"#;
        let parsed: ChatResponse = serde_json::from_str(raw).unwrap();
        assert!(parsed.model.is_none());
        assert_eq!(parsed.first_content().as_deref(), Some("x"));
    }
}
