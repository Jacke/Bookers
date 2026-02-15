use crate::config::Config;
use crate::models::problem::{Problem, Solution};
use async_trait::async_trait;
use chrono::Utc;
use serde_json::Value;
use std::collections::HashMap;

/// AI Provider trait for generating solutions
#[async_trait]
pub trait SolutionProvider: Send + Sync {
    /// Generate solution for a problem
    async fn solve(&self, problem: &Problem, context: &str) -> anyhow::Result<String>;
    /// Generate a hint for a problem
    async fn hint(&self, problem: &Problem, context: &str, hint_level: u8) -> anyhow::Result<String>;
    /// Provider name
    fn name(&self) -> &'static str;
}

/// AI Solver service that manages multiple providers
pub struct AISolver {
    providers: HashMap<String, Box<dyn SolutionProvider>>,
    default_provider: String,
}

impl AISolver {
    pub fn new(_config: &Config) -> anyhow::Result<Self> {
        let mut providers: HashMap<String, Box<dyn SolutionProvider>> = HashMap::new();

        // Add OpenAI provider if API key is available
        if let Ok(key) = std::env::var("OPENAI_API_KEY") {
            providers.insert(
                "openai".to_string(),
                Box::new(OpenAIProvider::new(key)),
            );
        }

        // Add Claude provider if API key is available
        if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
            providers.insert(
                "claude".to_string(),
                Box::new(ClaudeProvider::new(key)),
            );
        }

        // Add Mistral provider if API key is available
        if let Ok(key) = std::env::var("MISTRAL_API_KEY") {
            providers.insert(
                "mistral".to_string(),
                Box::new(MistralProvider::new(key)),
            );
        }

        let default_provider = if providers.contains_key("claude") {
            "claude"
        } else if providers.contains_key("openai") {
            "openai"
        } else if providers.contains_key("mistral") {
            "mistral"
        } else {
            return Err(anyhow::anyhow!("No AI providers configured. Set OPENAI_API_KEY, ANTHROPIC_API_KEY, or MISTRAL_API_KEY"));
        }.to_string();

        Ok(Self {
            providers,
            default_provider,
        })
    }

    /// Generate solution for a problem
    pub async fn solve(
        &self,
        problem: &Problem,
        provider: Option<&str>,
        theory_context: Option<&str>,
    ) -> anyhow::Result<Solution> {
        let provider_name = provider.unwrap_or(&self.default_provider);
        let provider = self.providers
            .get(provider_name)
            .ok_or_else(|| anyhow::anyhow!("Provider {} not available", provider_name))?;

        let context = theory_context.unwrap_or("");
        let content = provider.solve(problem, context).await?;

        Ok(Solution {
            id: Solution::generate_id(&problem.id),
            problem_id: problem.id.clone(),
            provider: provider_name.to_string(),
            content: content.clone(),
            latex_formulas: extract_latex_formulas(&content),
            is_verified: false,
            rating: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
    }

    /// Generate hint for a problem
    pub async fn hint(
        &self,
        problem: &Problem,
        provider: Option<&str>,
        theory_context: Option<&str>,
        hint_level: u8,
    ) -> anyhow::Result<String> {
        let provider_name = provider.unwrap_or(&self.default_provider);
        let provider = self.providers
            .get(provider_name)
            .ok_or_else(|| anyhow::anyhow!("Provider {} not available", provider_name))?;

        let context = theory_context.unwrap_or("");
        provider.hint(problem, context, hint_level).await
    }

    /// List available providers
    pub fn available_providers(&self) -> Vec<&str> {
        self.providers.keys().map(|s| s.as_str()).collect()
    }
}

/// OpenAI GPT-4o provider
pub struct OpenAIProvider {
    api_key: String,
    client: reqwest::Client,
}

impl OpenAIProvider {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl SolutionProvider for OpenAIProvider {
    async fn solve(&self, problem: &Problem, context: &str) -> anyhow::Result<String> {
        let prompt = build_solution_prompt(&problem.content, context);

        let request_body = serde_json::json!({
            "model": "gpt-4o",
            "messages": [
                {
                    "role": "system",
                    "content": "You are an expert math teacher. Solve problems step by step, explaining each step clearly. Use LaTeX for math formulas."
                },
                {
                    "role": "user",
                    "content": prompt
                }
            ],
            "temperature": 0.3,
            "max_tokens": 4096
        });

        let response = self.client
            .post("https://api.openai.com/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&request_body)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(anyhow::anyhow!("OpenAI API error: {}", error_text));
        }

        let result: Value = response.json().await?;
        let content = result["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Invalid response format"))?
            .to_string();

        Ok(content)
    }

    async fn hint(&self, problem: &Problem, context: &str, hint_level: u8) -> anyhow::Result<String> {
        let prompt = build_hint_prompt(&problem.content, context, hint_level);

        let request_body = serde_json::json!({
            "model": "gpt-4o",
            "messages": [
                {
                    "role": "system",
                    "content": "You are an expert math teacher. Provide helpful hints without giving away the full solution. Use LaTeX for math formulas."
                },
                {
                    "role": "user",
                    "content": prompt
                }
            ],
            "temperature": 0.5,
            "max_tokens": 1024
        });

        let response = self.client
            .post("https://api.openai.com/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&request_body)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(anyhow::anyhow!("OpenAI API error: {}", error_text));
        }

        let result: Value = response.json().await?;
        let content = result["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Invalid response format"))?
            .to_string();

        Ok(content)
    }

    fn name(&self) -> &'static str {
        "openai"
    }
}

/// Claude provider
pub struct ClaudeProvider {
    api_key: String,
    client: reqwest::Client,
}

impl ClaudeProvider {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl SolutionProvider for ClaudeProvider {
    async fn solve(&self, problem: &Problem, context: &str) -> anyhow::Result<String> {
        let prompt = build_solution_prompt(&problem.content, context);

        let request_body = serde_json::json!({
            "model": "claude-3-5-sonnet-20241022",
            "max_tokens": 4096,
            "messages": [
                {
                    "role": "user",
                    "content": prompt
                }
            ],
            "system": "You are an expert math teacher. Solve problems step by step, explaining each step clearly. Use LaTeX for math formulas ($...$ for inline, $$...$$ for display)."
        });

        let response = self.client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&request_body)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(anyhow::anyhow!("Claude API error: {}", error_text));
        }

        let result: Value = response.json().await?;
        let content = result["content"][0]["text"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Invalid response format"))?
            .to_string();

        Ok(content)
    }

    async fn hint(&self, problem: &Problem, context: &str, hint_level: u8) -> anyhow::Result<String> {
        let prompt = build_hint_prompt(&problem.content, context, hint_level);

        let request_body = serde_json::json!({
            "model": "claude-3-5-sonnet-20241022",
            "max_tokens": 1024,
            "messages": [
                {
                    "role": "user",
                    "content": prompt
                }
            ],
            "system": "You are an expert math teacher. Provide helpful hints without giving away the full solution. Use LaTeX for math formulas."
        });

        let response = self.client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&request_body)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(anyhow::anyhow!("Claude API error: {}", error_text));
        }

        let result: Value = response.json().await?;
        let content = result["content"][0]["text"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Invalid response format"))?
            .to_string();

        Ok(content)
    }

    fn name(&self) -> &'static str {
        "claude"
    }
}

/// Mistral provider
pub struct MistralProvider {
    api_key: String,
    client: reqwest::Client,
}

impl MistralProvider {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl SolutionProvider for MistralProvider {
    async fn solve(&self, problem: &Problem, context: &str) -> anyhow::Result<String> {
        let prompt = build_solution_prompt(&problem.content, context);

        let request_body = serde_json::json!({
            "model": "mistral-large-latest",
            "messages": [
                {
                    "role": "system",
                    "content": "You are an expert math teacher. Solve problems step by step, explaining each step clearly. Use LaTeX for math formulas."
                },
                {
                    "role": "user",
                    "content": prompt
                }
            ],
            "temperature": 0.3,
            "max_tokens": 4096
        });

        let response = self.client
            .post("https://api.mistral.ai/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&request_body)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(anyhow::anyhow!("Mistral API error: {}", error_text));
        }

        let result: Value = response.json().await?;
        let content = result["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Invalid response format"))?
            .to_string();

        Ok(content)
    }

    async fn hint(&self, problem: &Problem, context: &str, hint_level: u8) -> anyhow::Result<String> {
        let prompt = build_hint_prompt(&problem.content, context, hint_level);

        let request_body = serde_json::json!({
            "model": "mistral-large-latest",
            "messages": [
                {
                    "role": "system",
                    "content": "You are an expert math teacher. Provide helpful hints without giving away the full solution. Use LaTeX for math formulas."
                },
                {
                    "role": "user",
                    "content": prompt
                }
            ],
            "temperature": 0.5,
            "max_tokens": 1024
        });

        let response = self.client
            .post("https://api.mistral.ai/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&request_body)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(anyhow::anyhow!("Mistral API error: {}", error_text));
        }

        let result: Value = response.json().await?;
        let content = result["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Invalid response format"))?
            .to_string();

        Ok(content)
    }

    fn name(&self) -> &'static str {
        "mistral"
    }
}

/// Build the solution prompt
fn build_solution_prompt(problem: &str, context: &str) -> String {
    format!(
        r#"Solve the following math problem step by step. Explain each step clearly.

Problem:
{}

Relevant theory/context from textbook:
{}

Requirements:
1. Provide a detailed, step-by-step solution
2. Explain the reasoning behind each step
3. Use LaTeX for all mathematical expressions ($...$ for inline, $$...$$ for display math)
4. If multiple solution methods exist, show the most straightforward one
5. State the final answer clearly at the end
6. Use Russian language for the explanation (as the problem is in Russian)

Solution:"#,
        problem,
        if context.is_empty() { "None provided" } else { context }
    )
}

/// Build the hint prompt based on hint level
fn build_hint_prompt(problem: &str, context: &str, hint_level: u8) -> String {
    let level_hint = match hint_level {
        1 => "Provide a VERY minimal hint - just point in the right direction without specifics.",
        2 => "Provide a moderate hint - give a clue about the approach or formula to use.",
        3 => "Provide a strong hint - outline the steps without giving the final answer.",
        _ => "Provide a hint appropriate for the problem.",
    };

    format!(
        r#"Provide a helpful hint for the following math problem. {}

Problem:
{}

Relevant theory/context from textbook:
{}

Requirements:
1. Do NOT give the full solution
2. Do NOT give the final answer
3. Provide a hint that helps the student think in the right direction
4. Use LaTeX for any mathematical expressions ($...$ for inline)
5. Use Russian language

Hint:"#,
        level_hint,
        problem,
        if context.is_empty() { "None provided" } else { context }
    )
}

/// Extract LaTeX formulas from solution text
fn extract_latex_formulas(text: &str) -> Vec<String> {
    let mut formulas = Vec::new();
    
    // Inline math: $...$
    let inline_re = regex::Regex::new(r"\$([^$]+)\$").unwrap();
    // Display math: $$...$$
    let display_re = regex::Regex::new(r"\$\$([^$]+)\$\$").unwrap();

    for cap in inline_re.captures_iter(text) {
        formulas.push(cap[1].to_string());
    }
    for cap in display_re.captures_iter(text) {
        formulas.push(cap[1].to_string());
    }

    formulas
}
