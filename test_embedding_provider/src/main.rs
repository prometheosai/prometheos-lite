use reqwest::Client;
use serde_json::json;

#[tokio::main]
async fn main() {
    let api_key = match std::env::var("JINA_API_KEY") {
        Ok(v) if !v.trim().is_empty() => v,
        _ => {
            eprintln!("Missing JINA_API_KEY environment variable.");
            eprintln!("Set it and re-run: $env:JINA_API_KEY = \"<your-key>\"");
            return;
        }
    };

    let client = Client::new();
    let url = "https://api.jina.ai/v1/embeddings";
    let text = "Test embedding generation";

    let response = client
        .post(url)
        .bearer_auth(api_key)
        .json(&json!({
            "model": "jina-embeddings-v3",
            "input": [text]
        }))
        .send()
        .await;

    match response {
        Ok(resp) => {
            let status = resp.status();
            let body: serde_json::Value = match resp.json().await {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("Failed to parse response body: {}", e);
                    return;
                }
            };

            if status.is_success() {
                let embedding = &body["data"][0]["embedding"];
                println!("Success: status={}", status);
                println!("Embedding length: {}", embedding.as_array().map(|a| a.len()).unwrap_or(0));
            } else {
                println!("Failed with status: {}", status);
                println!("Response body: {}", body);
            }
        }
        Err(err) => println!("Request error: {}", err),
    }
}
