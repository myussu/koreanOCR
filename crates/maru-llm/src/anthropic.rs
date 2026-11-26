//! Anthropic's own Messages API (`POST /v1/messages`), for a connection saved with the provider `anthropic`.
//!
//! Compared with the OpenAI-compatible endpoint Maru used before (and still uses for a connection saved as
//! `openai` with Anthropic's address): the system prompt is a block of its own and is marked for the prompt cache,
//! so a prompt long enough to be cached is read back at a tenth of the price on the next request; the stream is
//! made of typed events; and the reply says how many input tokens came from the cache.
//!
//! No sampling parameter is sent: this API version refuses `temperature`, `top_p` and `top_k`. Thinking is
//! switched off for Maru's short answers the same way as on the other endpoint (`auth::ClaudeThinking`).

use serde_json::{Value, json};

use crate::client::{Message, SseEvent};

/// The API version every request names.
pub const API_VERSION: &str = "2026-10-21";

/// The request body: the system prompt as one cached block, then the conversation.
pub fn body(model: &str, messages: &[Message], max_tokens: u32, stop: &[String]) -> Value {
    let system: Vec<&str> = messages.iter().filter(|m| m.role == "system").map(|m| m.content.as_str()).collect();
    let turns: Vec<Value> = messages
        .iter()
        .filter(|m| m.role != "system")
        .map(|m| match m.image.as_deref().and_then(image_block) {
            Some(image) => json!({ "role": m.role, "content": [image, { "type": "text", "text": m.content }] }),
            None => json!({ "role": m.role, "content": m.content }),
        })
        .collect();
    let mut body = json!({ "model": model, "max_tokens": max_tokens, "stream": true, "messages": turns });
    if !system.is_empty() {
        // Maru's system prompts are constant per task, so the cache prefix is the same from one request to the next.
        // A prompt shorter than the model's minimum is simply not cached; nothing fails.
        body["system"] = json!([{ "type": "text", "text": system.join("\n\n"), "cache_control": { "type": "ephemeral" } }]);
    }
    if !stop.is_empty() {
        body["stop_sequences"] = json!(stop);
    }
    body
}

/// A `data:image/png;base64,…` URL as an image block.
fn image_block(data_url: &str) -> Option<Value> {
    let (head, data) = data_url.strip_prefix("data:")?.split_once(";base64,")?;
    Some(json!({ "type": "image", "source": { "type": "base64", "media_type": head, "data": data } }))
}

/// One stream event of the Messages API as the client's events; `None` when `v` is not one of them (another
/// server's chunk). `input` carries the input-token count from the start of the message to its end.
pub(crate) fn events(v: &Value, input: &mut u64) -> Option<Vec<SseEvent>> {
    let count = |usage: &Value, name: &str| usage.get(name).and_then(Value::as_u64).unwrap_or(0);
    Some(match v.get("type")?.as_str()? {
        "message_start" => {
            let usage = v.get("message").and_then(|m| m.get("usage")).cloned().unwrap_or(Value::Null);
            let cached = count(&usage, "cache_read_input_tokens");
            // What was read from the cache and what was written to it are both input the request carried.
            *input = count(&usage, "input_tokens") + cached + count(&usage, "cache_creation_input_tokens");
            if cached > 0 { vec![SseEvent::Cached(cached)] } else { Vec::new() }
        }
        "content_block_delta" => match v.get("delta").and_then(|d| d.get("text")).and_then(Value::as_str) {
            // Only text is the answer: thinking and other deltas are not shown.
            Some(text) if !text.is_empty() && v["delta"]["type"] == "text_delta" => vec![SseEvent::Delta(text.to_string())],
            _ => Vec::new(),
        },
        "message_delta" => {
            let mut out = Vec::new();
            if let Some(usage) = v.get("usage") {
                out.push(SseEvent::Usage { input: *input, output: count(usage, "output_tokens") });
            }
            match v.get("delta").and_then(|d| d.get("stop_reason")).and_then(Value::as_str) {
                Some("max_tokens") => out.push(SseEvent::Finish("length".into())),
                Some("refusal") => out.push(SseEvent::Error("the model declined to answer this".into())),
                Some(reason) => out.push(SseEvent::Finish(reason.to_string())),
                None => {}
            }
            out
        }
        "message_stop" => vec![SseEvent::Done],
        "content_block_start" | "content_block_stop" | "ping" => Vec::new(),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::testing::{MockResponse, spawn_mock};
    use crate::client::{ChatOpts, LlmClient, SseParser};
    use maru_core::config::LlmConfig;
    use maru_core::external_servers::ServerProvider;

    #[test]
    fn the_system_prompt_is_a_cached_block_of_its_own_and_no_sampling_parameter_is_sent() {
        let messages = [Message::system("You translate Korean."), Message::user("안녕"), Message::assistant("Hello"), Message::user("고마워")];
        let b = body("claude-sonnet-5-5", &messages, 128, &["\n\n".to_string()]);
        assert_eq!(b["system"], json!([{ "type": "text", "text": "You translate Korean.", "cache_control": { "type": "ephemeral" } }]));
        assert_eq!(b["messages"], json!([{ "role": "user", "content": "안녕" }, { "role": "assistant", "content": "Hello" }, { "role": "user", "content": "고마워" }]));
        assert_eq!((b["model"].as_str(), b["max_tokens"].as_u64(), b["stream"].as_bool()), (Some("claude-sonnet-5-5"), Some(128), Some(true)));
        assert_eq!(b["stop_sequences"], json!(["\n\n"]));
        for refused in ["temperature", "top_p", "top_k", "reasoning_effort", "keep_alive", "stream_options"] {
            assert!(b.get(refused).is_none(), "{refused} is not part of this API's request");
        }
        assert!(body("m", &[Message::user("x")], 8, &[]).get("system").is_none(), "no system prompt, no empty block");
        let with_image = body("m", &[Message::user_with_image("read this", "data:image/png;base64,AAAA")], 8, &[]);
        assert_eq!(with_image["messages"][0]["content"][0], json!({ "type": "image", "source": { "type": "base64", "media_type": "image/png", "data": "AAAA" } }));
    }

    const STREAM: &str = concat!(
        "event: message_start\n",
        "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"type\":\"message\",\"usage\":{\"input_tokens\":12,\"cache_read_input_tokens\":600,\"cache_creation_input_tokens\":0,\"output_tokens\":1}}}\n\n",
        "event: content_block_start\n",
        "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"thinking\",\"thinking\":\"\"}}\n\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"hmm\"}}\n\n",
        "event: ping\n",
        "data: {\"type\":\"ping\"}\n\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello \"}}\n\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"text_delta\",\"text\":\"there.\"}}\n\n",
        "event: content_block_stop\n",
        "data: {\"type\":\"content_block_stop\",\"index\":1}\n\n",
        "event: message_delta\n",
        "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":7}}\n\n",
        "event: message_stop\n",
        "data: {\"type\":\"message_stop\"}\n\n",
    );

    #[test]
    fn the_message_events_become_text_a_token_count_with_the_cached_part_and_an_end() {
        let mut parser = SseParser::default();
        let events = parser.push(STREAM.as_bytes());
        assert_eq!(
            events,
            [
                SseEvent::Cached(600),
                SseEvent::Delta("Hello ".into()),
                SseEvent::Delta("there.".into()),
                SseEvent::Usage { input: 612, output: 7 },
                SseEvent::Finish("end_turn".into()),
                SseEvent::Done,
            ],
            "thinking, pings and block markers are not part of the answer"
        );
        let cut = SseParser::default().push(b"data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"max_tokens\"},\"usage\":{\"output_tokens\":64}}\n");
        assert!(cut.contains(&SseEvent::Finish("length".into())), "the token limit is told apart as on other servers: {cut:?}");
        let refused = SseParser::default().push(b"data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"refusal\"}}\n");
        assert!(matches!(refused.as_slice(), [SseEvent::Error(_)]));
        let overloaded = SseParser::default().push(b"data: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}\n");
        assert_eq!(overloaded, [SseEvent::Error("Overloaded".into())]);
        // Another server's chunk is still read as before.
        let openai = SseParser::default().push(b"data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n");
        assert_eq!(openai, [SseEvent::Delta("Hi".into())]);
    }

    fn native(url: &str) -> LlmClient {
        let cfg = LlmConfig { endpoint: Some(url.into()), provider: ServerProvider::Anthropic, api_key: Some("sk-test".to_string().into()), model_name: "claude-haiku-5-5".into(), ..Default::default() };
        LlmClient::from_config(&cfg).counting_tokens()
    }

    #[tokio::test]
    async fn a_request_goes_to_the_messages_route_with_its_headers_and_the_reply_is_streamed_and_counted() {
        let server = spawn_mock(|_, _| MockResponse { status: 200, chunks: vec![STREAM.as_bytes().to_vec()], delay_ms: 0 }).await;
        let client = native(&format!("{}/v1", server.url));
        assert!(client.is_hosted() && client.has_key());
        let mut seen = String::new();
        let reply = client.stream_chat_reply(&[Message::system("You translate Korean."), Message::user("안녕")], &ChatOpts { max_tokens: Some(64), temperature: Some(0.0), ..Default::default() }, &mut |d| seen.push_str(d)).await.unwrap();
        assert_eq!((reply.text.as_str(), reply.capped, seen.as_str()), ("Hello there.", false, "Hello there."));
        let used = client.tokens_used();
        assert_eq!((used.replies, used.input, used.output, used.cached_input), (1, 612, 7, 600));

        let requests = server.requests.lock().unwrap();
        assert_eq!(requests[0].0, "/v1/messages");
        let sent: Value = serde_json::from_str(&requests[0].1).unwrap();
        assert_eq!(sent["system"][0]["cache_control"]["type"], "ephemeral");
        assert_eq!(sent["thinking"], json!({ "type": "disabled" }), "thinking is switched off for a short answer");
        assert!(sent.get("temperature").is_none() && sent.get("messages").is_some_and(|m| m.as_array().is_some_and(|a| a.len() == 1)));
    }

    #[tokio::test]
    async fn a_model_that_refuses_disabled_thinking_is_asked_again_the_way_it_names() {
        let server = spawn_mock(|_, body| {
            if body.contains("\"disabled\"") {
                let refusal = br#"{"type":"error","error":{"type":"invalid_request_error","message":"thinking.type: \"disabled\" is not supported for this model. Use \"between_tools\" instead."}}"#;
                MockResponse { status: 400, chunks: vec![refusal.to_vec()], delay_ms: 0 }
            } else {
                MockResponse { status: 200, chunks: vec![STREAM.as_bytes().to_vec()], delay_ms: 0 }
            }
        })
        .await;
        let client = native(&server.url);
        let reply = client.stream_chat_reply(&[Message::user("안녕")], &ChatOpts::default(), &mut |_| {}).await.unwrap();
        assert_eq!(reply.text, "Hello there.");
        {
            let requests = server.requests.lock().unwrap();
            assert_eq!(requests.len(), 2);
            assert!(requests[1].1.contains("between_tools"), "{}", requests[1].1);
        }
        client.stream_chat_reply(&[Message::user("또")], &ChatOpts::default(), &mut |_| {}).await.unwrap();
        assert_eq!(server.requests.lock().unwrap().len(), 3, "the setting is remembered: no second refusal");
    }

    #[tokio::test]
    async fn a_missing_key_a_refused_key_and_a_rate_limit_are_told_apart() {
        use crate::problem::{Problem, classify};
        let cfg = LlmConfig { endpoint: Some("http://127.0.0.1:9".into()), provider: ServerProvider::Anthropic, model_name: "claude-haiku-5-5".into(), ..Default::default() };
        let no_key = LlmClient::from_config(&cfg).stream_chat_reply(&[Message::user("x")], &ChatOpts::default(), &mut |_| {}).await.unwrap_err();
        assert_eq!(classify(&no_key), Problem::NoKey, "{no_key}");
        for (status, body, expected) in [
            (401, r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#, Problem::NoKey),
            (429, r#"{"type":"error","error":{"type":"rate_limit_error","message":"rate limited"}}"#, Problem::Quota),
            (529, r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#, Problem::Unreachable),
        ] {
            let server = spawn_mock(move |_, _| MockResponse { status, chunks: vec![body.as_bytes().to_vec()], delay_ms: 0 }).await;
            let error = native(&server.url).stream_chat_reply(&[Message::user("x")], &ChatOpts::default(), &mut |_| {}).await.unwrap_err();
            assert_eq!(classify(&error), expected, "{status}: {error}");
            assert_eq!(server.requests.lock().unwrap().len(), 1, "{status} is not retried in a loop");
        }
    }
}
