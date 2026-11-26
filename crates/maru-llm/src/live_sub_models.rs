//! Native candidate prompts and strict chat streaming for isolated live-model tests.

use serde_json::json;

use crate::{LlmClient, LlmError, Message, Result};

pub(crate) fn sub_messages(source: &str, turns: &[(String, String)]) -> Vec<Message> {
    // The publisher trained these separate context blocks; no invented speaker or glossary.
    let system = "TASK: Translate Korean subtitles into English.\nSTYLE: neutral.\nTranslate only CURRENT_SOURCE. PREVIOUS_SOURCE and PREVIOUS_TRANSLATION are context only. Preserve meaning, tone, slang, profanity, uncertainty, repetitions and incomplete speech. Return only the final translation without labels or commentary.";
    let current = format!("[CURRENT_SOURCE]\n{source}");
    let user = if turns.is_empty() {
        current
    } else {
        let previous_sources = turns.iter().map(|(ko, _)| ko.as_str()).collect::<Vec<_>>().join("\n");
        let previous_translations = turns.iter().map(|(_, en)| en.as_str()).collect::<Vec<_>>().join("\n");
        format!("[PREVIOUS_SOURCE]\n{previous_sources}\n\n[PREVIOUS_TRANSLATION]\n{previous_translations}\n\n{current}")
    };
    vec![Message::system(system), Message::user(user)]
}

pub(crate) fn lmt_messages(source: &str, turns: &[(String, String)]) -> Vec<Message> {
    // The single-source instruction is native; preceding chat pairs are experimental.
    let prompt = |ko: &str| format!("Translate the following text from Korean into English:\nKorean: {ko}\nEnglish:");
    let mut messages = Vec::new();
    for (ko, en) in turns {
        messages.push(Message::user(prompt(ko)));
        messages.push(Message::assistant(en));
    }
    messages.push(Message::user(prompt(source)));
    messages
}

pub(crate) async fn stream(client: &LlmClient, source: &str, messages: &[Message], delta: &mut (dyn FnMut(&str) + Send)) -> Result<String> {
    let body = json!({
        "model": client.model_name(), "messages": messages, "stream": true,
        "temperature": 0.0, "top_k": 1, "max_tokens": (source.chars().count() as u32 * 2 + 40).min(256),
        "cache_prompt": true, "chat_template_kwargs": { "enable_thinking": false },
        "stream_options": { "include_usage": true },
    });
    let reply = client.stream_route("/v1/chat/completions", &body, false, None, delta).await?;
    // A benchmark scores a capped answer as a failure rather than as a short translation.
    if reply.capped { return Err(LlmError::Api("diagnostic chat ended with length; inspect the preserved stream".into())); }
    Ok(reply.text.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use maru_core::config::LlmConfig;
    use crate::testing::{MockResponse, spawn_mock};

    #[test]
    fn subtitle_context_keeps_sources_and_answers_aligned_and_the_current_source_unanswered() {
        let turns = vec![("이걸 타면\n위로 갈 수 있어요.".into(), "We can ride this up.".into()), ("네.".into(), "Yes.".into())];
        let body = json!(sub_messages("위로?", &turns));
        assert_eq!(body[0]["role"], "system");
        assert!(body[0]["content"].as_str().unwrap().contains("STYLE: neutral."));
        assert_eq!(body[1]["content"], "[PREVIOUS_SOURCE]\n이걸 타면\n위로 갈 수 있어요.\n네.\n\n[PREVIOUS_TRANSLATION]\nWe can ride this up.\nYes.\n\n[CURRENT_SOURCE]\n위로?");
        assert_eq!(json!(sub_messages("위로?", &[]))[1]["content"], "[CURRENT_SOURCE]\n위로?");
    }

    #[test]
    fn lmt_uses_the_published_instruction_without_a_system_and_marks_context_as_completed_chat_pairs() {
        let turns = vec![("갈까요?".into(), "Shall we go?".into())];
        let body = json!(lmt_messages("가지 마세요.", &turns));
        assert_eq!(body[0]["role"], "user");
        assert_eq!(body[0]["content"], "Translate the following text from Korean into English:\nKorean: 갈까요?\nEnglish:");
        assert_eq!(body[1]["role"], "assistant");
        assert_eq!(body[1]["content"], "Shall we go?");
        assert_eq!(body[2]["content"], "Translate the following text from Korean into English:\nKorean: 가지 마세요.\nEnglish:");
        assert_eq!(lmt_messages("가지 마세요.", &[]).len(), 1);
    }

    #[tokio::test]
    async fn diagnostic_chat_is_greedy_and_preserves_split_utf8_and_unwanted_labels_for_review() {
        let event = "data: {\"choices\":[{\"delta\":{\"content\":\"English: 안녕\"},\"finish_reason\":null}]}\n\ndata: [DONE]\n\n".as_bytes().to_vec();
        let split = event.iter().position(|&b| b >= 128).unwrap() + 1;
        let mock = spawn_mock(move |_, _| MockResponse { status: 200, chunks: vec![event[..split].to_vec(), event[split..].to_vec()], delay_ms: 0 }).await;
        let client = LlmClient::new(&mock.url, &LlmConfig { model_name: "test-alias".into(), ..Default::default() });
        let mut deltas = String::new();
        assert_eq!(stream(&client, "위로?", &sub_messages("위로?", &[]), &mut |s| deltas.push_str(s)).await.unwrap(), "English: 안녕");
        assert_eq!(deltas, "English: 안녕");
        let requests = mock.requests.lock().unwrap();
        assert_eq!(requests[0].0, "/v1/chat/completions");
        let body: serde_json::Value = serde_json::from_str(&requests[0].1).unwrap();
        assert_eq!(body["model"], "test-alias");
        assert_eq!(body["temperature"], 0.0);
        assert_eq!(body["top_k"], 1);
        assert_eq!(body["max_tokens"], 46);
        assert_eq!(body["chat_template_kwargs"]["enable_thinking"], false);
    }

    #[tokio::test]
    async fn capped_incomplete_and_invalid_chat_outputs_are_errors_with_partial_text_retained() {
        for event in ["data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n", "data: {\"choices\":[{\"delta\":{\"content\":\"partial\"},\"finish_reason\":\"length\"}]}\n\ndata: [DONE]\n", "data: not-json\n", "data: {\"error\":\"failed\"}\n"] {
            let mock = spawn_mock(move |_, _| MockResponse { status: 200, chunks: vec![event.as_bytes().to_vec()], delay_ms: 0 }).await;
            let client = LlmClient::new(&mock.url, &LlmConfig::default());
            let mut deltas = String::new();
            assert!(stream(&client, "위로?", &sub_messages("위로?", &[]), &mut |s| deltas.push_str(s)).await.is_err());
            if event.contains("partial") { assert_eq!(deltas, "partial"); }
        }
    }
}
