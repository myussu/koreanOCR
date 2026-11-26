//! Plain completions for the MiLMMT diagnostic; production tasks still use chat.

use serde_json::json;

use crate::{LlmClient, LlmError, Result};

fn prompt(source: &str, turns: &[(String, String)]) -> String {
    // Completed examples are an experimental extension of Xiaomi's plain prompt.
    let mut prompt = String::new();
    for (ko, en) in turns {
        prompt.push_str(&format!("Translate this from Korean to English:\nKorean: {ko}\nEnglish: {en}\n\n"));
    }
    prompt.push_str(&format!("Translate this from Korean to English:\nKorean: {source}\nEnglish:"));
    prompt
}

pub(crate) async fn stream(client: &LlmClient, source: &str, turns: &[(String, String)], delta: &mut (dyn FnMut(&str) + Send)) -> Result<String> {
    let body = json!({
        "prompt": prompt(source, turns), "stream": true, "temperature": 0.0, "top_k": 1,
        "n_predict": (source.chars().count() as u32 * 2 + 40).min(256), "cache_prompt": true,
    });
    let reply = client.stream_route("/completion", &body, false, None, delta).await?;
    // A benchmark scores a capped answer as a failure rather than as a short translation.
    if reply.capped { return Err(LlmError::Api("MiLMMT reached the live output cap; inspect the preserved stream".into())); }
    Ok(reply.text.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{MockResponse, spawn_mock};
    use serde_json::Value;

    #[tokio::test]
    async fn the_milmmt_prompt_is_a_plain_completion_with_greedy_decoding() {
        let event = "data: {\"content\":\"Don't go. 안녕\",\"stop\":false}\n\ndata: {\"content\":\"\",\"stop\":true}\n\n".as_bytes().to_vec();
        let split = event.iter().position(|&b| b >= 128).unwrap() + 1;
        let server = spawn_mock(move |_, _| MockResponse { status: 200, chunks: vec![event[..split].to_vec(), event[split..].to_vec()], delay_ms: 0 }).await;
        let mut deltas = String::new();
        assert_eq!(stream(&LlmClient::new(&server.url, &maru_core::config::LlmConfig::default()), "가지 마세요.", &[], &mut |s| deltas.push_str(s)).await.unwrap(), "Don't go. 안녕");
        assert_eq!(deltas, "Don't go. 안녕");
        let requests = server.requests.lock().unwrap();
        assert_eq!(requests[0].0, "/completion");
        let body: Value = serde_json::from_str(&requests[0].1).unwrap();
        assert_eq!(body["prompt"], "Translate this from Korean to English:\nKorean: 가지 마세요.\nEnglish:");
        assert_eq!(body["temperature"], 0.0);
        assert_eq!(body["top_k"], 1);
        assert_eq!(body["n_predict"], 54);
        assert!(body.get("messages").is_none());
    }

    #[tokio::test]
    async fn an_incomplete_or_capped_or_invalid_completion_is_refused() {
        for event in ["data: {\"content\":\"partial\"}\n", "data: {\"content\":\"partial\",\"stop\":true,\"stop_type\":\"limit\"}\n", "data: not-json\n", "data: {\"error\":\"failed\"}\n"] {
            let server = spawn_mock(move |_, _| MockResponse { status: 200, chunks: vec![event.as_bytes().to_vec()], delay_ms: 0 }).await;
            assert!(stream(&LlmClient::new(&server.url, &maru_core::config::LlmConfig::default()), "네", &[], &mut |_| {}).await.is_err());
        }
    }

    #[tokio::test]
    async fn previous_translations_are_completed_examples_and_only_the_current_answer_is_generated() {
        let server = spawn_mock(|_, _| MockResponse { status: 200, chunks: vec![b"data: {\"content\":\"Up?\",\"stop\":true}\n\n".to_vec()], delay_ms: 0 }).await;
        let turns = vec![("이걸 타면\n위로 갈 수 있어요.".into(), "We can ride this up.".into()), ("네.".into(), "Yes.".into())];
        assert_eq!(stream(&LlmClient::new(&server.url, &maru_core::config::LlmConfig::default()), "위로?", &turns, &mut |_| {}).await.unwrap(), "Up?");
        let requests = server.requests.lock().unwrap();
        let body: Value = serde_json::from_str(&requests[0].1).unwrap();
        assert_eq!(body["prompt"], "Translate this from Korean to English:\nKorean: 이걸 타면\n위로 갈 수 있어요.\nEnglish: We can ride this up.\n\nTranslate this from Korean to English:\nKorean: 네.\nEnglish: Yes.\n\nTranslate this from Korean to English:\nKorean: 위로?\nEnglish:");
        assert_eq!(body["n_predict"], 46);
        assert_eq!(body["temperature"], 0.0);
        assert_eq!(body["top_k"], 1);
        assert!(body.get("messages").is_none());
    }
}
