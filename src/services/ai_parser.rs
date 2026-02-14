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
    pub async fn parse_text(&self, book_id: &str, text: &str, page_num: Option<u32>) -> anyhow::Result<AIParseResult> {
        let cache_key = format!("{}\n{}", book_id, text);

        // Check cache first
        if let Some(cached) = self.cache.get(&cache_key).await {
            log::info!("✅ Cache hit for page {:?}", page_num);
            return Ok(cached);
        }

        // Book-specific parser (deterministic) for known textbooks.
        if algebra7_parser::matches(book_id) {
            log::info!("Using book-specific parser for {}", book_id);
            let result = algebra7_parser::parse(text);
            self.cache.set(&cache_key, result.clone()).await;
            return Ok(result);
        }
        
        // Try AI parser first if API key available
        if let Some(ref _key) = self.api_key {
            match self.ai_parse_with_retry(text).await {
                Ok(result) => {
                    log::info!("✅ AI parser successfully found {} problems", result.problems.len());
                    // Cache the result
                    self.cache.set(&cache_key, result.clone()).await;
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
        self.cache.set(&cache_key, result.clone()).await;
        
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

    /// Extract the "tail" of a problem that continues to next page.
    /// This returns the portion of content that should be prepended to the next page's continuation.
    pub fn extract_continuation_tail(&self, problem: &ParsedProblem) -> Option<String> {
        if !problem.continues_to_next {
            return None;
        }

        let content = problem.content.trim();
        if content.is_empty() {
            return None;
        }

        let lines: Vec<&str> = content.lines().collect();
        if lines.len() < 2 {
            return None;
        }

        let last_line = lines.last().unwrap().trim();
        if last_line.ends_with('.') || last_line.ends_with(';') || last_line.ends_with('!') || last_line.ends_with('?') {
            return None;
        }

        let tail = lines[1..].join("\n").trim().to_string();
        if tail.is_empty() {
            None
        } else {
            Some(tail)
        }
    }

    /// Merge content from previous page with current problem content.
    /// Returns new content with previous tail prepended.
    pub fn merge_with_prev_content(&self, current_content: &str, prev_tail: Option<&str>) -> String {
        match prev_tail {
            Some(tail) if !tail.is_empty() => {
                format!("{}\n\n{}", tail, current_content)
            }
            _ => current_content.to_string(),
        }
    }

    /// Process a list of problems with cross-page context from previous and next pages.
    /// This handles both marking and content merging.
    pub fn process_cross_page(&self,
                              prev_problem: Option<&ParsedProblem>,
                              prev_continuation_tail: Option<&str>,
                              current_problems: &mut Vec<ParsedProblem>,
                              next_problems: Option<&[ParsedProblem]>) {
        // Merge with previous page content
        if let Some(first) = current_problems.first_mut() {
            if let Some(prev) = prev_problem {
                if first.number == prev.number {
                    first.continues_from_prev = true;
                    if let Some(tail) = prev_continuation_tail {
                        first.content = self.merge_with_prev_content(&first.content, Some(tail));
                    }
                }
            }
        }

        // Mark and extract tail for next page
        if let Some(next) = next_problems {
            if let Some(current_last) = current_problems.last() {
                if let Some(next_first) = next.first() {
                    if current_last.number == next_first.number {
                        // Mark current as continuing
                        if let Some(last) = current_problems.last_mut() {
                            last.continues_to_next = true;
                        }
                    }
                }
            }
        }
    }
}

/// Deterministic parser for `algebra-7` OCR output.
///
/// Goal: reliably extract "exercise"-style problems like `71. ...` and `566. ...` (and sub-problems
/// like `а)` / `a)`) while avoiding false positives from step lists like `1) ...` inside examples.
mod algebra7_parser {
    use super::{AIParseResult, ParsedProblem, ParsedSubProblem};

    pub fn matches(book_id: &str) -> bool {
        let id = book_id.trim().trim_end_matches(".pdf");
        id.eq_ignore_ascii_case("algebra-7")
    }

    pub fn parse(text: &str) -> AIParseResult {
        let mut out = Vec::<ParsedProblem>::new();

        let mut current: Option<ProblemBuilder> = None;

        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() {
                continue;
            }

            // Skip markdown headings (section titles, etc.)
            if line.starts_with('#') {
                continue;
            }

            // Chapter header often appears at page boundary and should not be appended
            // to the previous problem content.
            if is_chapter_heading_line(line) {
                if let Some(pb) = current.take() {
                    out.push(pb.finish());
                }
                continue;
            }

            // Main problem start:
            // - `71. ...`
            // - `Задача 1. ...` / `Задача 1: ...`
            if let Some((num, rest)) = parse_main_problem_start(line) {
                if let Some(pb) = current.take() {
                    out.push(pb.finish());
                }
                current = Some(ProblemBuilder::new(num, rest));
                continue;
            }

            let Some(pb) = current.as_mut() else {
                continue;
            };

            // Sub-problem start: `а) ...`, `a) ...`, `(а) ...`
            if let Some((letter, rest)) = parse_sub_problem_start(line) {
                pb.start_sub(letter, rest);
                continue;
            }

            pb.push_line(line);
        }

        if let Some(pb) = current.take() {
            out.push(pb.finish());
        }

        AIParseResult { problems: out }
    }

    fn is_chapter_heading_line(line: &str) -> bool {
        let lower = line.trim().to_lowercase();
        let Some(rest) = lower.strip_prefix("глава") else {
            return false;
        };

        let rest = rest.trim_start();
        if rest.is_empty() {
            return false;
        }

        let chapter_token: String = rest
            .chars()
            .take_while(|c| !c.is_whitespace() && *c != '.' && *c != ':')
            .collect();

        if chapter_token.is_empty() {
            return false;
        }

        let is_digits = chapter_token.chars().all(|c| c.is_ascii_digit());
        let is_roman = chapter_token
            .chars()
            .all(|c| matches!(c, 'i' | 'v' | 'x' | 'l' | 'c' | 'd' | 'm'));

        is_digits || is_roman
    }

    fn parse_main_problem_start(line: &str) -> Option<(String, String)> {
        if let Some((num, rest)) = parse_zadacha_start(line) {
            return Some((num, rest));
        }
        if let Some((num, rest)) = parse_numeric_dot_start(line) {
            return Some((num, rest));
        }
        None
    }

    fn parse_zadacha_start(line: &str) -> Option<(String, String)> {
        let rest = line.strip_prefix("Задача")?.trim_start();
        let bytes = rest.as_bytes();
        let mut i = 0usize;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i == 0 {
            return None;
        }

        let num = rest[..i].to_string();
        let mut j = i;
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }

        if j < bytes.len() {
            // Accept `.`, `:`, `)` as delimiter after the number.
            match bytes[j] {
                b'.' | b':' | b')' => {
                    j += 1;
                    while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                        j += 1;
                    }
                }
                _ => {}
            }
        }

        let content = rest[j..].trim().to_string();
        Some((num, content))
    }

    fn parse_numeric_dot_start(line: &str) -> Option<(String, String)> {
        let bytes = line.as_bytes();
        let mut i = 0usize;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i == 0 || i >= bytes.len() || bytes[i] != b'.' {
            return None;
        }

        let num = line[..i].to_string();
        i += 1; // skip '.'
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let content = line[i..].trim().to_string();
        Some((num, content))
    }

    fn parse_sub_problem_start(line: &str) -> Option<(String, String)> {
        let mut it = line.chars();
        let first = it.next()?;

        if first == '(' {
            let letter = it.next()?;
            let close = it.next()?;
            if close != ')' {
                return None;
            }
            let rest: String = it.collect();
            return Some((letter.to_lowercase().to_string(), rest.trim().to_string()));
        }

        if !first.is_alphabetic() {
            return None;
        }

        let second = it.next()?;
        if second != ')' && second != '.' && second != ']' {
            return None;
        }

        let rest: String = it.collect();
        Some((first.to_lowercase().to_string(), rest.trim().to_string()))
    }

    #[derive(Debug)]
    struct ProblemBuilder {
        number: String,
        content: Vec<String>,
        sub_problems: Vec<SubBuilder>,
        current_sub: Option<SubBuilder>,
    }

    impl ProblemBuilder {
        fn new(number: String, first_content_line: String) -> Self {
            let mut content = Vec::new();
            if !first_content_line.is_empty() {
                content.push(first_content_line);
            }
            Self {
                number,
                content,
                sub_problems: Vec::new(),
                current_sub: None,
            }
        }

        fn start_sub(&mut self, letter: String, first_line: String) {
            if let Some(sub) = self.current_sub.take() {
                self.sub_problems.push(sub);
            }
            self.current_sub = Some(SubBuilder::new(letter, first_line));
        }

        fn push_line(&mut self, line: &str) {
            if let Some(ref mut sub) = self.current_sub {
                sub.push_line(line);
            } else {
                self.content.push(line.to_string());
            }
        }

        fn finish(mut self) -> ParsedProblem {
            if let Some(sub) = self.current_sub.take() {
                self.sub_problems.push(sub);
            }

            let content = self.content.join("\n").trim().to_string();
            let sub_problems = self
                .sub_problems
                .into_iter()
                .map(|s| s.finish())
                .collect();

            ParsedProblem {
                number: self.number,
                content,
                sub_problems,
                continues_from_prev: false,
                continues_to_next: false,
            }
        }
    }

    #[derive(Debug)]
    struct SubBuilder {
        letter: String,
        content: Vec<String>,
    }

    impl SubBuilder {
        fn new(letter: String, first_line: String) -> Self {
            let mut content = Vec::new();
            if !first_line.is_empty() {
                content.push(first_line);
            }
            Self { letter, content }
        }

        fn push_line(&mut self, line: &str) {
            self.content.push(line.to_string());
        }

        fn finish(self) -> ParsedSubProblem {
            ParsedSubProblem {
                letter: self.letter,
                content: self.content.join("\n").trim().to_string(),
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn parses_numbered_exercises_and_subproblems() {
            let text = r#"
71. Результаты проверки скорости чтения...

# Упражнения для повторения

72. Найдите значение выражения:
a) 2+2
б) 3+3
"#;

            let res = parse(text);
            assert_eq!(res.problems.len(), 2);
            assert_eq!(res.problems[0].number, "71");
            assert_eq!(res.problems[1].number, "72");
            assert_eq!(res.problems[1].sub_problems.len(), 2);
            assert_eq!(res.problems[1].sub_problems[0].letter, "a");
            assert_eq!(res.problems[1].sub_problems[0].content, "2+2");
        }

        #[test]
        fn does_not_treat_step_lists_as_problems() {
            let text = r#"
Пример 2. Найдём значение выражения
1) $6^{2}=36$
2) $(-2)^{5}=-32$

73. Настоящая задача.
"#;

            let res = parse(text);
            assert_eq!(res.problems.len(), 1);
            assert_eq!(res.problems[0].number, "73");
        }

        #[test]
        fn parses_zadacha_prefix() {
            let text = r#"
## 5. Выражения с переменными
Задача 1. Завод ежедневно перерабатывает 5 т молока.
Задача 2: Ширина прямоугольника равна 5 см.
"#;

            let res = parse(text);
            assert_eq!(res.problems.len(), 2);
            assert_eq!(res.problems[0].number, "1");
            assert!(res.problems[0].content.starts_with("Завод"));
            assert_eq!(res.problems[1].number, "2");
            assert!(res.problems[1].content.starts_with("Ширина"));
        }

        #[test]
        fn strips_chapter_header_from_previous_problem_tail() {
            let text = r#"
702. Трёхзначное число оканчивается цифрой 6.
Если её зачеркнуть, то полученное число будет меньше данного на 366.
Найдите данное трёхзначное число.
Глава 5. Разложение многочленов на множители
"#;

            let res = parse(text);
            assert_eq!(res.problems.len(), 1);
            let content = &res.problems[0].content;
            assert!(!content.contains("Глава 5"));
            assert!(content.contains("Найдите данное трёхзначное число."));
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
