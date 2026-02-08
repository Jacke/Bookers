use serde::{Deserialize, Serialize};
use std::process::Command;
use crate::services::parser::TextbookParser;
use crate::services::cache::AIParseCache;
use crate::services::retry::{retry_with_backoff, RetryConfig};

/// Hybrid parser: AI (Mistral) + Regex fallback
pub struct HybridParser {
    api_key: Option<String>,
    regex_parser: TextbookParser,
    cache: AIParseCache,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ParsedSubProblem {
    pub letter: String,
    pub content: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ParsedProblem {
    pub number: String,
    pub content: String,
    pub sub_problems: Vec<ParsedSubProblem>,
    /// Does this problem continue from previous page?
    pub continues_from_prev: bool,
    /// Does this problem continue to next page?
    pub continues_to_next: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AIParseResult {
    pub problems: Vec<ParsedProblem>,
}

/// Cross-page analysis result
#[derive(Debug)]
pub struct CrossPageAnalysis {
    /// Problem number that continues from previous page
    pub continued_problem: Option<String>,
    /// Problem number that continues to next page
    pub incomplete_problem: Option<String>,
    /// Is last problem complete (ends with . ; or ) ?
    pub last_problem_complete: bool,
}

impl HybridParser {
    pub fn new(api_key: Option<String>) -> Self {
        Self {
            api_key,
            regex_parser: TextbookParser::new(),
            cache: AIParseCache::new(),
        }
    }

    /// Main parse method - tries AI first, falls back to regex
    pub async fn parse_text(&self, text: &str, page_num: Option<u32>) -> anyhow::Result<AIParseResult> {
        // Check cache first
        if let Some(cached) = self.cache.get(text).await {
            log::info!("✅ Cache hit for page {:?}", page_num);
            return Ok(cached);
        }
        
        // Try AI parser first if API key available
        if let Some(ref _key) = self.api_key {
            match self.ai_parse_with_retry(text).await {
                Ok(result) => {
                    log::info!("✅ AI parser successfully found {} problems", result.problems.len());
                    // Cache the result
                    self.cache.set(text, result.clone()).await;
                    return Ok(result);
                }
                Err(e) => {
                    log::warn!("⚠️ AI parser failed, falling back to regex: {}", e);
                }
            }
        }
        
        // Fallback to regex parser
        log::info!("Using regex parser for page {:?}", page_num);
        let regex_result = self.regex_parser.parse(text, "unknown", page_num.unwrap_or(1));
        
        let problems = regex_result.problems.into_iter().map(|p| {
            let sub_problems = p.sub_problems.unwrap_or_default()
                .into_iter()
                .map(|s| ParsedSubProblem {
                    letter: s.number,
                    content: s.content,
                })
                .collect();
            
            ParsedProblem {
                number: p.number,
                content: p.content,
                sub_problems,
                continues_from_prev: false,
                continues_to_next: false,
            }
        }).collect();
        
        let result = AIParseResult { problems };
        
        // Cache regex results too
        self.cache.set(text, result.clone()).await;
        
        Ok(result)
    }
    
    /// AI-powered parsing with retry logic
    async fn ai_parse_with_retry(&self, text: &str) -> anyhow::Result<AIParseResult> {
        let config = RetryConfig::default();
        
        retry_with_backoff(&config, "AI parse", || async {
            self.ai_parse_internal(text).await
        }).await
    }

    /// AI-powered parsing via Mistral (internal implementation)
    async fn ai_parse_internal(&self, text: &str) -> anyhow::Result<AIParseResult> {
        let api_key = self.api_key.as_ref()
            .ok_or_else(|| anyhow::anyhow!("No API key available"))?;
        
        let python_script = format!(r#"
import json
import os
import re
from mistralai import Mistral

api_key = os.getenv("MISTRAL_API_KEY", "{}")
client = Mistral(api_key=api_key)

ocr_text = '''{}'''

# Clean OCR text - remove obvious OCR artifacts
ocr_text = re.sub(r'([а-яa-z])\n\s*\1', r'\1', ocr_text)  # Remove duplicate letters at line breaks
ocr_text = re.sub(r'\n\s*\n', '\n', ocr_text)  # Remove excessive blank lines

prompt = '''Ты - эксперт по анализу математических учебников с 99% точностью.

ЗАДАЧА: Разбери OCR текст и выдели ВСЕ задачи с подзадачами.

КРИТИЧЕСКИ ВАЖНЫЕ ПРАВИЛА:
1. Номера задач: 223, 224, 225 (целые числа, могут быть точки для подномеров: 1.1, 1.2)
2. Подзадачи ВСЕГДА начинаются с буквы и скобки: а), б), в), г), д), е), ж), з), и), к), л), м), н), о), п), р), с), т)
3. Подзадача = буква + ) + пробел/перенос + текст
4. Если текст содержит "а)" или "б)" - это подзадачи
5. Задача заканчивается перед следующей задачей или концом текста
6. Игнорируй: теоремы, определения, примеры, упражнения без номеров
7. Верни ТОЛЬКО JSON

ОСОБЫЕ СЛУЧАИ:
- "289. Текст... а)... б)... в)..." - это задача 289 с подзадачами
- "Докажите, что..." без номера - НЕ задача
- "Пример 1" - НЕ задача (это пример)

ФОРМАТ ОТВЕТА (строго JSON):
{{
  "problems": [
    {{
      "number": "289",
      "content": "Полный текст задачи со всеми подзадачами (а), б), в)...)",
      "sub_problems": [
        {{"letter": "а", "content": "Текст подзадачи без 'а)'"}},
        {{"letter": "б", "content": "Текст подзадачи без 'б)'"}},
        {{"letter": "в", "content": "Текст подзадачи без 'в)'"}}
      ],
      "continues_from_prev": false,
      "continues_to_next": false
    }}
  ]
}}

Если задача начинается на этой странице (есть номер в начале) - continues_from_prev = false
Если задача очевидно продолжается с предыдущей страницы (начинается с текста без номера, который логически продолжает предыдущую) - continues_from_prev = true

OCR текст:
''' + ocr_text + '''

Верни ТОЛЬКО JSON, без markdown (без ```).'''

try:
    response = client.chat.complete(
        model="mistral-large-latest",
        messages=[{{"role": "user", "content": prompt}}],
        temperature=0.05,
        max_tokens=8000
    )
    
    result_text = response.choices[0].message.content.strip()
    
    # Clean markdown
    result_text = re.sub(r'^```json\s*', '', result_text)
    result_text = re.sub(r'^```\s*', '', result_text)
    result_text = re.sub(r'```\s*$', '', result_text)
    result_text = result_text.strip()
    
    data = json.loads(result_text)
    
    if "problems" not in data:
        data = {{"problems": []}}
    
    print(json.dumps(data, ensure_ascii=False))
    
except Exception as e:
    print(json.dumps({{"error": str(e), "problems": []}}, ensure_ascii=False))
    raise
"#, api_key, text.replace("'''", "'''"));

        let output = Command::new("python3")
            .arg("-c")
            .arg(&python_script)
            .env("MISTRAL_API_KEY", api_key)
            .output()
            .map_err(|e| anyhow::anyhow!("Failed to run Python: {}", e))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        if !stderr.is_empty() {
            log::warn!("AI parser stderr: {}", stderr);
        }

        let result: AIParseResult = serde_json::from_str(&stdout)
            .map_err(|e| anyhow::anyhow!("Failed to parse AI response: {}. Output: {}", e, stdout))?;

        Ok(result)
    }

    /// Analyze if problems continue across pages
    pub fn analyze_cross_page(&self, current_page_problems: &[ParsedProblem], 
                              next_page_text: Option<&str>) -> CrossPageAnalysis {
        let last_problem = current_page_problems.last();
        
        let last_problem_complete = if let Some(last) = last_problem {
            let content = &last.content;
            // Check if ends with sentence terminator
            content.trim_end().ends_with(&['.', ';', ')', '!', '?'][..])
        } else {
            true
        };

        let incomplete_problem = if last_problem_complete {
            None
        } else {
            last_problem.map(|p| p.number.clone())
        };

        let continued_problem = next_page_text.and_then(|text| {
            // If next page starts with text (not a number), it might be a continuation
            let trimmed = text.trim_start();
            if !trimmed.is_empty() {
                let first_char = trimmed.chars().next().unwrap();
                // If starts with letter (not digit), likely a continuation
                if first_char.is_alphabetic() && !first_char.is_numeric() {
                    last_problem.map(|p| p.number.clone())
                } else {
                    None
                }
            } else {
                None
            }
        });

        CrossPageAnalysis {
            continued_problem,
            incomplete_problem,
            last_problem_complete,
        }
    }

    /// Merge problems that continue across pages
    pub fn merge_cross_page_problems(&self, 
                                     prev_problems: Option<&ParsedProblem>,
                                     current_problems: &mut [ParsedProblem],
                                     next_page_analysis: Option<&CrossPageAnalysis>) {
        // Mark problems that continue from previous page
        if let Some(prev) = prev_problems {
            if let Some(first) = current_problems.first_mut() {
                // If first problem has same number as last on prev page
                if first.number == prev.number {
                    first.continues_from_prev = true;
                }
            }
        }

        // Mark problems that continue to next page
        if let Some(analysis) = next_page_analysis {
            if let Some(ref incomplete_num) = analysis.incomplete_problem {
                for problem in current_problems.iter_mut() {
                    if &problem.number == incomplete_num {
                        problem.continues_to_next = true;
                    }
                }
            }
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MultiPageParseResult {
    pub pages: Vec<PageParseResult>,
    pub merged_problems: Vec<MergedProblem>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PageParseResult {
    pub page_number: u32,
    pub problems: Vec<ParsedProblem>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MergedProblem {
    pub number: String,
    pub content: String,
    pub pages: Vec<u32>,
    pub sub_problems: Vec<ParsedSubProblem>,
}

impl MultiPageParseResult {
    /// Merge problems that span multiple pages
    pub fn merge_problems(&mut self) {
        let mut merged: std::collections::HashMap<String, MergedProblem> = std::collections::HashMap::new();
        
        for page in &self.pages {
            for problem in &page.problems {
                let entry = merged.entry(problem.number.clone()).or_insert(MergedProblem {
                    number: problem.number.clone(),
                    content: String::new(),
                    pages: Vec::new(),
                    sub_problems: Vec::new(),
                });
                
                // Append content if continues
                if problem.continues_from_prev && !entry.content.is_empty() {
                    entry.content.push('\n');
                }
                entry.content.push_str(&problem.content);
                entry.pages.push(page.page_number);
                
                // Merge sub-problems (only if not already present)
                for sub in &problem.sub_problems {
                    if !entry.sub_problems.iter().any(|s| s.letter == sub.letter) {
                        entry.sub_problems.push(sub.clone());
                    }
                }
            }
        }
        
        self.merged_problems = merged.into_values().collect();
        // Sort by problem number
        self.merged_problems.sort_by(|a, b| {
            let a_num: f64 = a.number.parse().unwrap_or(0.0);
            let b_num: f64 = b.number.parse().unwrap_or(0.0);
            a_num.partial_cmp(&b_num).unwrap_or(std::cmp::Ordering::Equal)
        });
    }
}
