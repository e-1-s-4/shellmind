//! Ollama HTTP client (the v0.1.0 AI backend).
//!
//! Uses the local REST API (`http://localhost:11434` by default):
//!
//! * `POST /api/chat`      – explain / fix / generate,
//! * `POST /api/embed`     – batched embeddings (falls back to the legacy
//!   `POST /api/embeddings` single-text endpoint),
//! * `GET  /api/tags`      – list installed models,
//! * `POST /api/pull`      – download a model.
//!
//! All calls are synchronous with explicit timeouts; every failure is a
//! soft error the caller degrades from (offline engine takes over).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct Ollama {
    pub base: String,
    agent: ureq::Agent,
}

#[derive(Debug)]
pub enum OllamaError {
    Network(String),
    Api(String),
}

impl OllamaError {
    fn msg(&self) -> String {
        match self {
            OllamaError::Network(m) | OllamaError::Api(m) => m.clone(),
        }
    }
}

impl std::fmt::Display for OllamaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.msg())
    }
}

impl std::error::Error for OllamaError {}

#[derive(Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<ChatMessage<'a>>,
    #[serde(rename = "stream")]
    _stream: bool,
    options: ChatOptions,
}

#[derive(Serialize)]
struct ChatOptions {
    temperature: f32,
}

#[derive(Deserialize)]
struct ChatResponse {
    message: Option<ChatMessageResp>,
}

#[derive(Deserialize)]
struct ChatMessageResp {
    content: String,
}

#[derive(Serialize)]
struct EmbedRequest<'a> {
    model: &'a str,
    input: &'a [&'a str],
}

#[derive(Deserialize)]
struct EmbedResponse {
    embeddings: Option<Vec<Vec<f32>>>,
    embedding: Option<Vec<f32>>,
}

impl Ollama {
    pub fn new(base: &str, timeout_secs: u64) -> Ollama {
        Ollama {
            base: base.trim_end_matches('/').to_string(),
            agent: ureq::AgentBuilder::new()
                .timeout(std::time::Duration::from_secs(timeout_secs))
                .build(),
        }
    }

    /// Quick availability probe with a short timeout.
    pub fn ping(&self, timeout_ms: u64) -> bool {
        let fast = ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_millis(timeout_ms))
            .build();
        fast.get(&format!("{}/api/tags", self.base))
            .call()
            .map(|_| true)
            .unwrap_or(false)
    }

    /// Run a chat completion, returning the assistant message.
    pub fn chat(
        &self,
        model: &str,
        system: &str,
        user: &str,
        temperature: f32,
    ) -> Result<String, OllamaError> {
        let req = ChatRequest {
            model,
            messages: vec![
                ChatMessage {
                    role: "system",
                    content: system,
                },
                ChatMessage {
                    role: "user",
                    content: user,
                },
            ],
            _stream: false,
            options: ChatOptions { temperature },
        };
        let resp: serde_json::Value = self
            .agent
            .post(&format!("{}/api/chat", self.base))
            .send_json(serde_json::to_value(&req).map_err(|e| OllamaError::Api(e.to_string()))?)
            .map_err(|e| OllamaError::Network(e.to_string()))?
            .into_json()
            .map_err(|e| OllamaError::Api(e.to_string()))?;
        let parsed: ChatResponse =
            serde_json::from_value(resp).map_err(|e| OllamaError::Api(e.to_string()))?;
        parsed
            .message
            .map(|m| m.content)
            .ok_or_else(|| OllamaError::Api("empty response".into()))
    }

    /// Embed one or more texts. Prefers `/api/embed`, falls back to the
    /// legacy `/api/embeddings` endpoint per text.
    pub fn embed(&self, model: &str, texts: &[&str]) -> Result<Vec<Vec<f32>>, OllamaError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let req = EmbedRequest {
            model,
            input: texts,
        };
        let resp = self
            .agent
            .post(&format!("{}/api/embed", self.base))
            .send_json(serde_json::to_value(&req).map_err(|e| OllamaError::Api(e.to_string()))?)
            .map_err(|e| OllamaError::Network(e.to_string()))?;
        let parsed: EmbedResponse = resp
            .into_json()
            .map_err(|e| OllamaError::Api(e.to_string()))?;
        if let Some(emb) = parsed.embeddings {
            if !emb.is_empty() {
                return Ok(emb);
            }
        }
        // Legacy single-text endpoint fallback.
        let mut out = Vec::with_capacity(texts.len());
        for t in texts {
            let body = serde_json::json!({ "model": model, "prompt": t });
            let resp: EmbedResponse = self
                .agent
                .post(&format!("{}/api/embeddings", self.base))
                .send_json(body)
                .map_err(|e| OllamaError::Network(e.to_string()))?
                .into_json()
                .map_err(|e| OllamaError::Api(e.to_string()))?;
            let emb = resp
                .embedding
                .ok_or_else(|| OllamaError::Api("no embedding".into()))?;
            out.push(emb);
        }
        Ok(out)
    }

    /// List installed model names (e.g. `["qwen2.5-coder:3b", ...]`).
    pub fn tags(&self) -> Result<Vec<String>, OllamaError> {
        #[derive(Deserialize)]
        struct Tags {
            models: Option<Vec<TagsModel>>,
        }
        #[derive(Deserialize)]
        struct TagsModel {
            name: String,
        }
        let resp: Tags = self
            .agent
            .get(&format!("{}/api/tags", self.base))
            .call()
            .map_err(|e| OllamaError::Network(e.to_string()))?
            .into_json()
            .map_err(|e| OllamaError::Api(e.to_string()))?;
        Ok(resp.models.map(|m| m.into_iter().map(|x| x.name).collect()).unwrap_or_default())
    }

    /// Download (pull) a model. Blocks until the download finishes.
    pub fn pull(&self, model: &str) -> Result<(), OllamaError> {
        let body = serde_json::json!({ "name": model, "stream": false });
        let resp = self
            .agent
            .post(&format!("{}/api/pull", self.base))
            .timeout(std::time::Duration::from_secs(600))
            .send_json(body)
            .map_err(|e| OllamaError::Network(e.to_string()))?;
        if resp.status() < 300 {
            Ok(())
        } else {
            Err(OllamaError::Api(format!("pull failed: {}", resp.status())))
        }
    }
}
