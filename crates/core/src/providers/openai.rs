use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};
use uuid::Uuid;

use crate::{
    config::ToolFormat,
    error::{HarnessError, Result},
    message::{ContentBlock, Message, MessageContent, Role, StopReason, TurnResponse, Usage},
    provider::{Provider, ToolDef},
};

pub struct OpenAIProvider {
    client: Client,
    api_key: String,
    model: String,
    max_tokens: u32,
    base_url: String,
    tool_format: ToolFormat,
}

impl OpenAIProvider {
    pub fn new(
        api_key: impl Into<String>,
        model: impl Into<String>,
        max_tokens: u32,
        base_url: Option<String>,
        tool_format: ToolFormat,
    ) -> Self {
        let final_url = match base_url {
            None => "https://api.openai.com/v1/chat/completions".to_string(),
            Some(mut base) => {
                if base.ends_with("/chat/completions") {
                    base
                } else {
                    if !base.ends_with('/') {
                        base.push('/');
                    }
                    if base.ends_with("/v1/") {
                        format!("{}chat/completions", base)
                    } else {
                        format!("{}v1/chat/completions", base)
                    }
                }
            }
        };
        Self {
            client: Client::new(),
            api_key: api_key.into(),
            model: model.into(),
            max_tokens,
            base_url: final_url,
            tool_format,
        }
    }

    fn auth_headers(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        builder
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
    }

    fn build_api_messages(&self, messages: &[Message]) -> Vec<ApiMessage> {
        let mut api_msgs = Vec::new();

        for msg in messages {
            match msg.role {
                Role::System => {
                    api_msgs.push(ApiMessage {
                        role: "system".to_string(),
                        content: Some(serde_json::Value::String(
                            msg.text().unwrap_or("").to_string(),
                        )),
                        tool_calls: None,
                        tool_call_id: None,
                    });
                }
                Role::User => {
                    api_msgs.push(ApiMessage {
                        role: "user".to_string(),
                        content: Some(serde_json::Value::String(
                            msg.text().unwrap_or("").to_string(),
                        )),
                        tool_calls: None,
                        tool_call_id: None,
                    });
                }
                Role::Assistant => {
                    let mut text_content = String::new();
                    let mut tool_calls = Vec::new();

                    match &msg.content {
                        MessageContent::Text(t) => {
                            text_content.push_str(t);
                        }
                        MessageContent::Blocks(blocks) => {
                            for block in blocks {
                                match block {
                                    ContentBlock::Text { text } => {
                                        text_content.push_str(text);
                                    }
                                    ContentBlock::Thought { thought } => {
                                        text_content.push_str(&format!(
                                            "\n<thought>\n{}\n</thought>\n",
                                            thought
                                        ));
                                    }
                                    ContentBlock::ToolUse { id, name, input } => {
                                        // Provide native tool_calls for providers that support them.
                                        tool_calls.push(ApiToolCallSerialize {
                                            id: id.clone(),
                                            kind: "function".to_string(),
                                            function: ApiFunctionCallSerialize {
                                                name: name.clone(),
                                                arguments: input.to_string(),
                                            },
                                        });
                                        // AND inject XML-style call into text for instruction following.
                                        text_content.push_str(&format!(
                                            "\n<tool_call>\n{{\"name\": \"{}\", \"arguments\": {}}}\n</tool_call>\n",
                                            name, input
                                        ));
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }

                    api_msgs.push(ApiMessage {
                        role: "assistant".to_string(),
                        content: if text_content.is_empty() {
                            None
                        } else {
                            Some(serde_json::Value::String(text_content))
                        },
                        tool_calls: if tool_calls.is_empty() {
                            None
                        } else {
                            Some(tool_calls)
                        },
                        tool_call_id: None,
                    });
                }
                Role::Tool => match &msg.content {
                    MessageContent::Text(t) => {
                        api_msgs.push(ApiMessage {
                            role: "tool".to_string(),
                            content: Some(serde_json::Value::String(format!(
                                "<tool_result>\n{}\n</tool_result>",
                                t
                            ))),
                            tool_calls: None,
                            tool_call_id: None,
                        });
                    }
                    MessageContent::Blocks(blocks) => {
                        for block in blocks {
                            if let ContentBlock::ToolResult {
                                tool_use_id,
                                content,
                            } = block
                            {
                                api_msgs.push(ApiMessage {
                                    role: "tool".to_string(),
                                    content: Some(serde_json::Value::String(format!(
                                        "<tool_result>\n{}\n</tool_result>",
                                        content
                                    ))),
                                    tool_calls: None,
                                    tool_call_id: Some(tool_use_id.clone()),
                                });
                            }
                        }
                    }
                },
            }
        }
        api_msgs
    }
}

#[derive(Serialize)]
struct ApiRequest<'a> {
    model: &'a str,
    messages: Vec<ApiMessage>,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<ApiTool>>,
}

#[derive(Serialize)]
struct ApiMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<ApiToolCallSerialize>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

#[derive(Serialize)]
struct ApiToolCallSerialize {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    function: ApiFunctionCallSerialize,
}

#[derive(Serialize)]
struct ApiFunctionCallSerialize {
    name: String,
    arguments: String,
}

#[derive(Serialize)]
struct ApiTool {
    #[serde(rename = "type")]
    kind: String,
    function: ApiFunction,
}

#[derive(Serialize)]
struct ApiFunction {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Deserialize)]
struct ApiResponse {
    #[allow(dead_code)]
    id: Option<String>,
    choices: Vec<ApiChoice>,
    usage: Option<ApiUsage>,
    model: Option<String>,
}

#[derive(Deserialize)]
struct ApiChoice {
    message: ApiResponseMessage,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct ApiResponseMessage {
    #[allow(dead_code)]
    role: Option<String>,
    content: Option<String>,
    tool_calls: Option<Vec<ApiToolCall>>,
}

#[derive(Deserialize)]
struct ApiToolCall {
    id: String,
    #[allow(dead_code)]
    #[serde(rename = "type")]
    kind: String,
    function: ApiFunctionCall,
}

#[derive(Deserialize)]
struct ApiFunctionCall {
    name: String,
    arguments: String,
}

#[derive(Deserialize)]
struct ApiUsage {
    prompt_tokens: Option<u32>,
    completion_tokens: Option<u32>,
}

#[derive(Deserialize)]
struct ApiErrorResponse {
    error: ApiErrorBody,
}

#[derive(Deserialize)]
struct ApiErrorBody {
    message: String,
}

#[async_trait]
impl Provider for OpenAIProvider {
    fn name(&self) -> &str {
        &self.model
    }

    async fn complete(&self, messages: &[Message]) -> Result<TurnResponse> {
        self.complete_with_tools(messages, &[]).await
    }

    async fn complete_with_tools(
        &self,
        messages: &[Message],
        tools: &[ToolDef],
    ) -> Result<TurnResponse> {
        let api_messages = self.build_api_messages(messages);

        // If ToolFormat::Xml is set, we DON'T send native tools. We want the model to
        // follow the XML instructions in the system prompt.
        let api_tools = if tools.is_empty() || self.tool_format == ToolFormat::Xml {
            None
        } else {
            Some(
                tools
                    .iter()
                    .map(|t| ApiTool {
                        kind: "function".to_string(),
                        function: ApiFunction {
                            name: t.name.clone(),
                            description: t.description.clone(),
                            parameters: t.input_schema.clone(),
                        },
                    })
                    .collect(),
            )
        };

        let body = ApiRequest {
            model: &self.model,
            messages: api_messages,
            max_tokens: self.max_tokens,
            tools: api_tools,
        };

        debug!(model = %self.model, "sending request to OpenAI-compatible API");

        let resp = self
            .auth_headers(self.client.post(&self.base_url))
            .json(&body)
            .send()
            .await
            .map_err(|e| HarnessError::Provider(e.to_string()))?;

        let status = resp.status();
        if !status.is_success() {
            let raw = resp.text().await.unwrap_or_default();
            let msg = serde_json::from_str::<ApiErrorResponse>(&raw)
                .map(|e| e.error.message)
                .unwrap_or(raw);
            warn!(status = %status, error = %msg, "OpenAI API error");
            return Err(HarnessError::Api {
                status: status.as_u16(),
                body: msg,
            });
        }

        let raw = resp
            .text()
            .await
            .map_err(|e| HarnessError::Provider(e.to_string()))?;

        let api_resp: ApiResponse = serde_json::from_str(&raw).map_err(|e| {
            warn!(body = %raw, error = %e, "failed to decode OpenAI response body");
            HarnessError::Provider(format!(
                "error decoding response body: {} - raw: {}",
                e, raw
            ))
        })?;

        let choice = api_resp
            .choices
            .first()
            .ok_or_else(|| HarnessError::Provider("No choices in response".to_string()))?;

        let mut blocks = Vec::new();
        let mut tool_calls_extracted = false;

        // 1. First, check for native tool_calls from the API.
        if let Some(tool_calls) = &choice.message.tool_calls {
            for tc in tool_calls {
                let input = serde_json::from_str(&tc.function.arguments)
                    .unwrap_or_else(|_| serde_json::Value::Object(serde_json::Map::new()));
                blocks.push(ContentBlock::ToolUse {
                    id: tc.id.clone(),
                    name: tc.function.name.clone(),
                    input,
                });
                tool_calls_extracted = true;
            }
        }

        // 2. Then, check for <tool_call> tags in the text content.
        if let Some(content) = &choice.message.content {
            if !content.is_empty() {
                let extracted = extract_xml_tool_calls(content);
                let has_tags = extracted
                    .iter()
                    .any(|p| matches!(p, ToolCallPart::ToolUse { .. }));

                if has_tags {
                    for part in extracted {
                        match part {
                            ToolCallPart::Text(t) => {
                                if !t.trim().is_empty() {
                                    blocks.push(ContentBlock::Text { text: t });
                                }
                            }
                            ToolCallPart::Thought(t) => {
                                if !t.trim().is_empty() {
                                    blocks.push(ContentBlock::Thought { thought: t });
                                }
                            }
                            ToolCallPart::ToolUse { name, input } => {
                                blocks.push(ContentBlock::ToolUse {
                                    id: format!("call_{}", &Uuid::new_v4().to_string()[..8]),
                                    name,
                                    input,
                                });
                            }
                        }
                    }
                    tool_calls_extracted = true;
                } else {
                    blocks.push(ContentBlock::Text {
                        text: content.clone(),
                    });
                }
            }
        }

        let message = if blocks.len() == 1 {
            if let ContentBlock::Text { text } = &blocks[0] {
                Message::assistant(text.clone())
            } else {
                Message {
                    role: Role::Assistant,
                    content: MessageContent::Blocks(blocks),
                }
            }
        } else {
            Message {
                role: Role::Assistant,
                content: MessageContent::Blocks(blocks),
            }
        };

        let mut stop_reason = match choice.finish_reason.as_deref() {
            Some("stop") => StopReason::EndTurn,
            Some("length") => StopReason::MaxTokens,
            Some("tool_calls") | Some("function_call") => StopReason::ToolUse,
            Some("content_filter") => StopReason::StopSequence,
            _ => StopReason::EndTurn,
        };

        // If we successfully extracted tool calls via XML/JSON but the API didn't
        // signal it (common with some local providers), force the stop reason.
        if tool_calls_extracted && stop_reason == StopReason::EndTurn {
            stop_reason = StopReason::ToolUse;
        }

        Ok(TurnResponse {
            message,
            stop_reason,
            usage: Usage {
                input_tokens: api_resp
                    .usage
                    .as_ref()
                    .and_then(|u| u.prompt_tokens)
                    .unwrap_or(0),
                output_tokens: api_resp
                    .usage
                    .as_ref()
                    .and_then(|u| u.completion_tokens)
                    .unwrap_or(0),
                cache_read_tokens: None,
                cache_write_tokens: None,
            },
            model: api_resp.model.unwrap_or_default(),
        })
    }
}

#[derive(Debug, PartialEq)]
enum ToolCallPart {
    Text(String),
    Thought(String),
    ToolUse {
        name: String,
        input: serde_json::Value,
    },
}

/// Robustly extract tool calls from <tool_call> tags or <tool_name>/<arguments> sequences.
/// Also extracts <thought> tags as Thought parts.
#[cfg(feature = "xml-tools")]
fn extract_xml_tool_calls(content: &str) -> Vec<ToolCallPart> {
    let mut parts = Vec::new();
    let mut current_pos = 0;

    loop {
        let tool_start = content[current_pos..].find("<tool_call>");
        let thought_start = content[current_pos..].find("<thought>");
        let tool_name_start = content[current_pos..].find("<tool_name>");

        let (tag_type, start_idx) = match (tool_start, thought_start, tool_name_start) {
            (Some(ts), Some(th), Some(tn)) => {
                let min = ts.min(th).min(tn);
                if min == ts {
                    ("tool", ts)
                } else if min == th {
                    ("thought", th)
                } else {
                    ("tool_seq", tn)
                }
            }
            (Some(ts), Some(th), None) => {
                if ts < th {
                    ("tool", ts)
                } else {
                    ("thought", th)
                }
            }
            (Some(ts), None, Some(tn)) => {
                if ts < tn {
                    ("tool", ts)
                } else {
                    ("tool_seq", tn)
                }
            }
            (None, Some(th), Some(tn)) => {
                if th < tn {
                    ("thought", th)
                } else {
                    ("tool_seq", tn)
                }
            }
            (Some(ts), None, None) => ("tool", ts),
            (None, Some(th), None) => ("thought", th),
            (None, None, Some(tn)) => ("tool_seq", tn),
            (None, None, None) => break,
        };

        let absolute_start = current_pos + start_idx;

        if tag_type == "tool_seq" {
            // Handle <tool_name>...<arguments>...
            let name_inner_start = absolute_start + "<tool_name>".len();
            if let Some(name_end_idx) = content[name_inner_start..].find("</tool_name>") {
                let absolute_name_end = name_inner_start + name_end_idx;
                let tool_name = content[name_inner_start..absolute_name_end]
                    .trim()
                    .to_string();

                let next_search_pos = absolute_name_end + "</tool_name>".len();
                if let Some(args_start_rel) = content[next_search_pos..].find("<arguments>") {
                    let args_inner_start = next_search_pos + args_start_rel + "<arguments>".len();
                    if let Some(args_end_idx) = content[args_inner_start..].find("</arguments>") {
                        let absolute_args_end = args_inner_start + args_end_idx;
                        let args_json = content[args_inner_start..absolute_args_end].trim();

                        // Text before the sequence
                        let before = &content[current_pos..absolute_start];
                        if !before.is_empty() {
                            parts.push(ToolCallPart::Text(before.to_string()));
                        }

                        if let Ok(input) = serde_json::from_str::<serde_json::Value>(args_json) {
                            parts.push(ToolCallPart::ToolUse {
                                name: tool_name,
                                input,
                            });
                        } else {
                            // Treat as text if JSON invalid
                            parts.push(ToolCallPart::Text(
                                content[absolute_start..absolute_args_end + "</arguments>".len()]
                                    .to_string(),
                            ));
                        }
                        current_pos = absolute_args_end + "</arguments>".len();
                        continue;
                    }
                }
            }
            // Fallback for failed sequence parsing
            parts.push(ToolCallPart::Text(
                content[current_pos..absolute_start + "<tool_name>".len()].to_string(),
            ));
            current_pos = absolute_start + "<tool_name>".len();
            continue;
        }

        let tag_open = if tag_type == "tool" {
            "<tool_call>"
        } else {
            "<thought>"
        };
        let tag_close = if tag_type == "tool" {
            "</tool_call>"
        } else {
            "</thought>"
        };
        let inner_start = absolute_start + tag_open.len();

        if let Some(end_idx) = content[inner_start..].find(tag_close) {
            let absolute_end = inner_start + end_idx;

            // Text before the tag
            let before = &content[current_pos..absolute_start];
            if !before.is_empty() {
                parts.push(ToolCallPart::Text(before.to_string()));
            }

            let inner_content = &content[inner_start..absolute_end].trim();
            if tag_type == "tool" {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(inner_content) {
                    if let Some(name) = v.get("name").and_then(|n| n.as_str()) {
                        let input = v
                            .get("arguments")
                            .or_else(|| v.get("input"))
                            .cloned()
                            .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

                        parts.push(ToolCallPart::ToolUse {
                            name: name.to_string(),
                            input,
                        });
                    }
                } else {
                    // Invalid JSON, treat whole tag as text
                    parts.push(ToolCallPart::Text(
                        content[absolute_start..absolute_end + tag_close.len()].to_string(),
                    ));
                }
            } else {
                // Thought block: extract as separate part
                parts.push(ToolCallPart::Thought(inner_content.to_string()));
            }

            current_pos = absolute_end + tag_close.len();
        } else {
            // No closing tag found, treat the rest as text and exit loop
            break;
        }
    }

    let remaining = &content[current_pos..];
    if !remaining.is_empty() {
        parts.push(ToolCallPart::Text(remaining.to_string()));
    }

    parts
}

#[cfg(not(feature = "xml-tools"))]
fn extract_xml_tool_calls(content: &str) -> Vec<ToolCallPart> {
    vec![ToolCallPart::Text(content.to_string())]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_xml_tool_calls() {
        let content = "Thought: I should echo.\n<tool_call>\n{\"name\": \"echo\", \"arguments\": {\"message\": \"hi\"}}\n</tool_call>\nEnd.";
        let res = extract_xml_tool_calls(content);
        assert_eq!(res.len(), 3);
        assert!(matches!(res[0], ToolCallPart::Text(_)));
        if let ToolCallPart::ToolUse { name, .. } = &res[1] {
            assert_eq!(name, "echo");
        } else {
            panic!("Expected ToolUse");
        }
        assert!(matches!(res[2], ToolCallPart::Text(_)));
    }

    #[test]
    fn test_extract_mixed_xml_and_thoughts() {
        let content = "I will check the dir.\n<tool_call>\n{\"name\": \"bash\", \"arguments\": {\"command\": \"ls\"}}\n</tool_call>\nDone.";
        let res = extract_xml_tool_calls(content);
        // Expecting: Text("I will check the dir."), ToolUse, Text("Done.")
        assert_eq!(res.len(), 3);
        assert_eq!(
            res[0],
            ToolCallPart::Text("I will check the dir.\n".to_string())
        );
        assert!(matches!(res[1], ToolCallPart::ToolUse { .. }));
        assert_eq!(res[2], ToolCallPart::Text("\nDone.".to_string()));
    }
}
