use crate::models::problem::{Problem, TheoryBlock, TheoryType};
use chrono::Utc;
use lazy_regex::regex;
use regex::Regex;

/// Parser for extracting problems and theory from OCR text
pub struct TextbookParser {
    /// Patterns for detecting problem starts
    problem_patterns: Vec<Regex>,
    /// Patterns for detecting theory blocks
    theory_patterns: Vec<Regex>,
}

/// Result of parsing a chapter
#[derive(Debug)]
pub struct ParseResult {
    pub problems: Vec<Problem>,
    pub theory_blocks: Vec<TheoryBlock>,
    /// Content that couldn't be classified
    pub unclassified: Vec<String>,
}

impl Default for TextbookParser {
    fn default() -> Self {
        Self::new()
    }
}

impl TextbookParser {
    pub fn new() -> Self {
        let problem_patterns = vec![
            // Задача 1.1: ... или Задача №1: ...
            Regex::new(r"(?im)^\s*#*\s*Задача\s*[№#]?\s*(\d+[\.\d\w]*)[:.\s)]+").unwrap(),
            // Упражнение 5: ...
            Regex::new(r"(?im)^\s*#*\s*Упражнение\s*[№#]?\s*(\d+)[:.\s)]+").unwrap(),
            // Example 1: ... Problem 1: ...
            Regex::new(r"(?im)^\s*#*\s*(?:Example|Problem|Exercise)\s*[№#]?\s*(\d+)[:.\s)]+").unwrap(),
            // 1) ... 1. ... 1) $formula$ ...
            Regex::new(r"(?m)^\s*(\d+[\.\d]*)\s*[\.)\]]\s*(?:\$|[А-ЯA-Z])").unwrap(),
            // №125 или #125
            Regex::new(r"(?m)^\s*[№#]\s*(\d+)[:.\s)]+").unwrap(),
        ];

        // Theory blocks are much more consistent across OCR outputs than problems.
        // Keep capture groups stable:
        // 1) type keyword, 2) optional number, 3) optional inline title/rest of the header line.
        let theory_patterns = vec![Regex::new(
            r"(?im)^\s*#*\s*(теорема|определение|свойство|формула|доказательство|theorem|definition|property|formula|proof)\s*(\d*)\s*[:.\s]*(.*)$",
        )
        .unwrap()];

        Self {
            problem_patterns,
            theory_patterns,
        }
    }

    /// Detect sub-problem (а), б), в), г), д), е), ж), з), и), к) ...)
    pub fn detect_sub_problem(&self, line: &str) -> Option<String> {
        // Try multiple patterns to catch different OCR formats
        // Pattern 1: Russian letters with ) or . or ]
        let patterns = [
            r"(?i)^\s*([а-яё])\s*[\.\)\]]",
            r"(?i)^\s*([a-z])\s*[\.\)\]]",
            r"(?i)^\s*\(([а-яёa-z])\)",  // (а) or (a)
        ];
        
        for pattern in &patterns {
            if let Ok(re) = Regex::new(pattern) {
                if let Some(caps) = re.captures(line) {
                    if let Some(m) = caps.get(1) {
                        let letter = m.as_str().to_lowercase();
                        // Validate it's a single letter
                        if letter.chars().count() == 1 {
                            return Some(letter);
                        }
                    }
                }
            }
        }
        None
    }

    /// Parse OCR text and extract problems and theory blocks
    pub fn parse(&self, text: &str, book_id: &str, chapter_num: u32) -> ParseResult {
        let mut problems = Vec::new();
        let mut theory_blocks = Vec::new();
        let mut unclassified = Vec::new();

        let mut current_problem: Option<ProblemBuilder> = None;
        let mut current_theory: Option<TheoryBuilder> = None;
        let mut current_unclassified = String::new();

        let mut _problem_counter = 0u32;
        let mut theory_counter = 0u32;
        let mut current_page: Option<u32> = None;
        
        // Page number patterns
        let page_pattern = regex::Regex::new(r"(?i)(?:страница|стр\.?|page)\s*(\d+)").unwrap();

        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            
            // Check for page number indicator
            if let Some(caps) = page_pattern.captures(trimmed) {
                if let Some(page_num) = caps.get(1) {
                    current_page = page_num.as_str().parse().ok();
                }
            }

            // Check if this is a problem start
            if let Some(problem_num) = self.detect_problem_start(trimmed) {
                // Save previous content
                if let Some(pb) = current_problem.take() {
                    problems.push(pb.build(book_id, chapter_num));
                }
                if let Some(tb) = current_theory.take() {
                    theory_blocks.push(tb.build(book_id, chapter_num));
                }
                if !current_unclassified.is_empty() {
                    unclassified.push(current_unclassified.clone());
                    current_unclassified.clear();
                }

                _problem_counter += 1;
                let mut pb = ProblemBuilder::new(problem_num, trimmed);
                pb.page_number = current_page;
                current_problem = Some(pb);
                continue;
            }

            // Check if this is a theory block start
            if let Some((theory_type, title)) = self.detect_theory_start(trimmed) {
                // Save previous problem if exists
                if let Some(pb) = current_problem.take() {
                    problems.push(pb.build(book_id, chapter_num));
                }
                if let Some(tb) = current_theory.take() {
                    theory_blocks.push(tb.build(book_id, chapter_num));
                }

                theory_counter += 1;
                let mut tb = TheoryBuilder::new(theory_counter, theory_type, title);
                tb.page_number = current_page;
                current_theory = Some(tb);
                continue;
            }

            // Check if this is a sub-problem (а), б), в)...)
            if let Some(ref mut pb) = current_problem {
                if let Some(letter) = self.detect_sub_problem(trimmed) {
                    pb.start_sub_problem(letter, trimmed);
                    continue;
                }
                // Check if we're in a sub-problem
                if pb.current_sub.is_some() {
                    pb.add_line_to_sub(trimmed);
                } else {
                    pb.add_line(trimmed);
                }
            } else if let Some(ref mut tb) = current_theory {
                tb.add_line(trimmed);
            } else {
                current_unclassified.push_str(trimmed);
                current_unclassified.push('\n');
            }
        }

        // Save final blocks
        if let Some(pb) = current_problem {
            problems.push(pb.build(book_id, chapter_num));
        }
        if let Some(tb) = current_theory {
            theory_blocks.push(tb.build(book_id, chapter_num));
        }
        if !current_unclassified.is_empty() {
            unclassified.push(current_unclassified);
        }

        // Post-process: extract LaTeX formulas
        let problems: Vec<_> = problems
            .into_iter()
            .map(|mut p| {
                p.latex_formulas = p.extract_formulas();
                p
            })
            .collect();

        let theory_blocks: Vec<_> = theory_blocks
            .into_iter()
            .map(|mut t| {
                t.latex_formulas = extract_formulas(&t.content);
                t
            })
            .collect();

        ParseResult {
            problems,
            theory_blocks,
            unclassified,
        }
    }

    /// Detect if line starts a problem and extract problem number
    fn detect_problem_start(&self, line: &str) -> Option<String> {
        for pattern in &self.problem_patterns {
            if let Some(caps) = pattern.captures(line) {
                if let Some(num) = caps.get(1) {
                    return Some(num.as_str().trim().to_string());
                }
                // For patterns without capture group, use line number or counter
                return Some("?".to_string());
            }
        }
        None
    }

    /// Detect if line starts a theory block
    fn detect_theory_start(&self, line: &str) -> Option<(TheoryType, Option<String>)> {
        for pattern in &self.theory_patterns {
            if let Some(caps) = pattern.captures(line) {
                let type_str = caps.get(1).map(|m| m.as_str().to_lowercase());
                let num = caps.get(2).and_then(|m| {
                    let s = m.as_str().trim();
                    if s.is_empty() {
                        None
                    } else {
                        Some(s.to_string())
                    }
                });
                let inline_title = caps
                    .get(3)
                    .map(|m| m.as_str().trim())
                    .filter(|s| !s.is_empty());

                let theory_type = match type_str.as_deref() {
                    Some("теорема") | Some("theorem") => TheoryType::Theorem,
                    Some("определение") | Some("definition") => TheoryType::Definition,
                    Some("свойство") | Some("property") => TheoryType::Property,
                    Some("формула") | Some("formula") => TheoryType::Formula,
                    Some("доказательство") => TheoryType::Proof,
                    _ => TheoryType::Other,
                };

                let title = match (num, inline_title) {
                    (Some(n), Some(t)) => Some(format!("{} {}", n, t)),
                    (Some(n), None) => Some(n),
                    (None, Some(t)) => Some(t.to_string()),
                    (None, None) => None,
                };

                return Some((theory_type, title));
            }
        }
        None
    }
}

/// Sub-problem builder
#[derive(Debug)]
struct SubProblemBuilder {
    letter: String,
    content: String,
}

impl SubProblemBuilder {
    fn new(letter: String, first_line: &str) -> Self {
        Self {
            letter,
            content: first_line.to_string(),
        }
    }
    
    fn add_line(&mut self, line: &str) {
        self.content.push('\n');
        self.content.push_str(line);
    }
    
    fn build(self, parent_id: &str) -> Problem {
        let id = format!("{}:{}", parent_id, self.letter);
        let formulas = extract_formulas(&self.content);
        Problem {
            id,
            chapter_id: parent_id.split(':').take(2).collect::<Vec<_>>().join(":"),
            page_id: None,
            parent_id: Some(parent_id.to_string()),
            number: self.letter.clone(),
            display_name: format!("{})", self.letter),
            content: self.content,
            latex_formulas: formulas,
            page_number: None,
            difficulty: None,
            has_solution: false,
            created_at: Utc::now(),
            solution: None,
            sub_problems: None,
            continues_from_page: None,
            continues_to_page: None,
            is_cross_page: false,
        }
    }
}

/// Builder for constructing Problem
struct ProblemBuilder {
    number: String,
    content: String,
    page_number: Option<u32>,
    sub_problems: Vec<SubProblemBuilder>,
    current_sub: Option<SubProblemBuilder>,
}

impl ProblemBuilder {
    fn new(number: String, first_line: &str) -> Self {
        Self {
            number,
            content: first_line.to_string(),
            page_number: None,
            sub_problems: Vec::new(),
            current_sub: None,
        }
    }

    fn add_line(&mut self, line: &str) {
        self.content.push('\n');
        self.content.push_str(line);
    }
    
    fn start_sub_problem(&mut self, letter: String, line: &str) {
        // Save current sub-problem if exists
        if let Some(sub) = self.current_sub.take() {
            self.sub_problems.push(sub);
        }
        self.current_sub = Some(SubProblemBuilder::new(letter, line));
    }
    
    fn add_line_to_sub(&mut self, line: &str) {
        if let Some(ref mut sub) = self.current_sub {
            sub.add_line(line);
        }
    }
    
    fn finish_sub_problems(&mut self) {
        if let Some(sub) = self.current_sub.take() {
            self.sub_problems.push(sub);
        }
    }

    fn build(mut self, book_id: &str, chapter_num: u32) -> Problem {
        self.finish_sub_problems();
        let id = Problem::generate_id(book_id, chapter_num, &self.number);
        
        let sub_problems = if self.sub_problems.is_empty() {
            None
        } else {
            Some(self.sub_problems.into_iter().map(|s| s.build(&id)).collect())
        };
        
        Problem {
            id: id.clone(),
            chapter_id: format!("{}:{}", book_id, chapter_num),
            page_id: None,
            parent_id: None,
            number: self.number.clone(),
            display_name: format!("Problem {}", self.number),
            content: self.content,
            latex_formulas: vec![],
            page_number: self.page_number,
            difficulty: None,
            has_solution: false,
            created_at: Utc::now(),
            solution: None,
            sub_problems,
            continues_from_page: None,
            continues_to_page: None,
            is_cross_page: false,
        }
    }
}

/// Builder for constructing TheoryBlock
struct TheoryBuilder {
    block_num: u32,
    block_type: TheoryType,
    title: Option<String>,
    content: String,
    page_number: Option<u32>,
}

impl TheoryBuilder {
    fn new(block_num: u32, block_type: TheoryType, title: Option<String>) -> Self {
        Self {
            block_num,
            block_type,
            title,
            content: String::new(),
            page_number: None,
        }
    }

    fn add_line(&mut self, line: &str) {
        if !self.content.is_empty() {
            self.content.push('\n');
        }
        self.content.push_str(line);
    }

    fn build(self, book_id: &str, chapter_num: u32) -> TheoryBlock {
        let id = TheoryBlock::generate_id(book_id, chapter_num, self.block_num);
        TheoryBlock {
            id,
            chapter_id: format!("{}:{}", book_id, chapter_num),
            block_num: self.block_num,
            title: self.title,
            block_type: self.block_type,
            content: self.content,
            latex_formulas: vec![],
            page_number: self.page_number,
            created_at: Utc::now(),
        }
    }
}

/// Extract LaTeX formulas from text
fn extract_formulas(text: &str) -> Vec<String> {
    let mut formulas = Vec::new();

    // Pattern for inline math: $...$
    let inline_re = regex!(r"\$([^$]+)\$");
    // Pattern for display math: $$...$$
    let display_re = regex!(r"\$\$([^$]+)\$\$");
    // Pattern for \[...\]
    let bracket_re = regex!(r"\\\[([^\]]+)\\\]");

    for cap in inline_re.captures_iter(text) {
        formulas.push(cap[1].to_string());
    }
    for cap in display_re.captures_iter(text) {
        formulas.push(cap[1].to_string());
    }
    for cap in bracket_re.captures_iter(text) {
        formulas.push(cap[1].to_string());
    }

    formulas
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_problem() {
        let parser = TextbookParser::new();

        assert!(parser.detect_problem_start("Задача 1: Решить уравнение").is_some());
        assert!(parser.detect_problem_start("Задача №5. Найти").is_some());
        assert!(parser.detect_problem_start("1. Вычислить интеграл").is_some());
        assert!(parser.detect_problem_start("1) $x^2 + 3$").is_some());
        assert!(parser.detect_problem_start("№125. Решить").is_some());
    }

    #[test]
    fn test_detect_theory() {
        let parser = TextbookParser::new();

        assert!(parser.detect_theory_start("Теорема 1: О сумме углов").is_some());
        assert!(parser.detect_theory_start("Определение: Производная").is_some());
    }

    #[test]
    fn test_parse_simple() {
        let parser = TextbookParser::new();
        let text = r#"
Теорема Пифагора: $c^2 = a^2 + b^2$

Задача 1: Найти гипотенузу если $a=3$, $b=4$
Решение: $c = \sqrt{9+16} = 5$

Задача 2: Найти катет если $c=5$, $a=3$
Ответ: $b=4$
"#;

        let result = parser.parse(text, "algebra-7", 1);

        assert_eq!(result.problems.len(), 2);
        assert_eq!(result.theory_blocks.len(), 1);
        assert_eq!(result.problems[0].number, "1");
        assert_eq!(result.problems[1].number, "2");
    }
}
