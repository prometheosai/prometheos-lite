//! Manual local endpoint integration test.
//!
//! This test is ignored by default and never runs in normal CI.
//! It validates that a real local OpenAI-compatible endpoint (such as
//! Ollama or LM Studio) can receive a chat completion request and
//! return a non-empty response.
//!
//! ## Running manually
//!
//! ```bash
//! PROMETHEOS_TEST_LOCAL_BASE_URL=http://localhost:11434 \
//! PROMETHEOS_TEST_LOCAL_MODEL=ornith \
//! cargo test local_openai_compatible_endpoint -- --ignored
//! ```
//!
//! If the env vars are not set the test will fail with a clear message.

use crate::llm::client::LlmClient;

#[tokio::test]
#[ignore]
async fn local_openai_compatible_endpoint_smoke_test() {
    let base_url = std::env::var("PROMETHEOS_TEST_LOCAL_BASE_URL")
        .expect("PROMETHEOS_TEST_LOCAL_BASE_URL must be set (e.g. http://localhost:11434)");

    let model = std::env::var("PROMETHEOS_TEST_LOCAL_MODEL")
        .expect("PROMETHEOS_TEST_LOCAL_MODEL must be set (e.g. ornith or llama3.2)");

    let client = LlmClient::new(&base_url, &model)
        .expect("should build LlmClient")
        .with_retries(1);

    let result = client
        .generate("Say hello in exactly one short sentence.")
        .await;

    match result {
        Ok(text) => {
            assert!(!text.is_empty(), "model returned an empty response");
            eprintln!(
                "[integration] local endpoint smoke test passed — response length: {}",
                text.len()
            );
        }
        Err(e) => {
            panic!(
                "local endpoint smoke test failed.\n\
                 base_url: {}\n\
                 model:    {}\n\
                 error:    {}\n\
                 \n\
                 Make sure the local endpoint is running and the model is available.",
                base_url, model, e
            );
        }
    }
}
