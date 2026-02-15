use serde::{Deserialize, Serialize};
use crate::models::{Problem, TheoryBlock, TheoryType};

/// Complete page content parser - extracts ALL elements from page
pub struct PageContentParser {
    api_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedPageContent {
    /// Page metadata
    pub metadata: PageMetadata,
    
    /// All content elements in order
    pub elements: Vec<PageElement>,
    
    /// Statistics
    pub stats: PageStats,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageMetadata {
    pub page_number: Option<u32>,
    pub chapter_title: Option<String>,
    pub section_title: Option<String>,
    pub header: Option<String>,
    pub footer: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum PageElement {
    #[serde(rename = "problem")]
    Problem(ParsedProblem),
    
    #[serde(rename = "theory")]
    Theory(ParsedTheory),
    
    #[serde(rename = "example")]
    Example(ParsedExample),
    
    #[serde(rename = "figure")]
    Figure(ParsedFigure),
    
    #[serde(rename = "table")]
    Table(ParsedTable),
    
    #[serde(rename = "remark")]
    Remark(ParsedRemark),
    
    #[serde(rename = "exercise")]
    Exercise(ParsedExercise),
    
    #[serde(rename = "text")]
    Text(ParsedText),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedProblem {
    pub number: String,
    pub content: String,
    pub sub_problems: Vec<ParsedSubProblem>,
    pub difficulty: Option<u8>,
    pub category: Option<String>,
    pub formulas: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedSubProblem {
    pub letter: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedTheory {
    pub theory_type: TheoryElementType,
    pub title: Option<String>,
    pub number: Option<String>,
    pub content: String,
    pub formulas: Vec<String>,
    pub importance: ImportanceLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TheoryElementType {
    Definition,      // Определение
    Theorem,         // Теорема
    Lemma,           // Лемма
    Corollary,       // Следствие
    Property,        // Свойство
    Axiom,           // Аксиома
    Postulate,       // Постулат
    Formula,         // Формула (как элемент)
    Rule,            // Правило
    Method,          // Метод
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportanceLevel {
    Critical,    // Основной материал, обязательно к изучению
    Important,   // Важный материал
    Standard,    // Обычный материал
    Optional,    // Дополнительный материал
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedExample {
    pub number: Option<String>,
    pub title: Option<String>,
    pub problem: String,
    pub solution: String,
    pub formulas: Vec<String>,
    pub is_solved: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedFigure {
    pub number: Option<String>,
    pub caption: Option<String>,
    pub description: String,  // Текстовое описание изображения
    pub image_reference: Option<String>,
    pub figure_type: FigureType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FigureType {
    Graph,        // График функции
    Diagram,      // Диаграмма
    Geometric,    // Геометрическая фигура
    Chart,        // Диаграмма/график
    Illustration, // Иллюстрация
    Table,        // Таблица как изображение
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedTable {
    pub number: Option<String>,
    pub caption: Option<String>,
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedRemark {
    pub content: String,
    pub remark_type: RemarkType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemarkType {
    Note,        // Примечание
    Warning,     // Внимание
    Tip,         // Совет
    Important,   // Важно
    Remember,    // Запомните
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedExercise {
    pub number: String,
    pub content: String,
    pub difficulty: Option<u8>,
    pub is_practice: bool,  // true = для отработки навыков
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedText {
    pub content: String,
    pub is_intro: bool,     // Вводный текст
    pub is_conclusion: bool,// Заключение
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageStats {
    pub problem_count: usize,
    pub theory_count: usize,
    pub example_count: usize,
    pub figure_count: usize,
    pub exercise_count: usize,
    pub total_formulas: usize,
}

impl PageContentParser {
    pub fn new(api_key: Option<String>) -> Self {
        Self { api_key }
    }
    
    /// Parse complete page content
    pub async fn parse_page(&self, ocr_text: &str, page_num: Option<u32>) -> anyhow::Result<ParsedPageContent> {
        // Try AI parser first
        if let Some(ref key) = self.api_key {
            match self.ai_parse_page(ocr_text, page_num, key).await {
                Ok(result) => return Ok(result),
                Err(e) => {
                    log::warn!("AI page parser failed, using regex fallback: {}", e);
                }
            }
        }
        
        // Fallback to regex parser
        Ok(self.regex_parse_page(ocr_text, page_num))
    }
    
    /// AI-powered page parsing
    async fn ai_parse_page(&self, text: &str, page_num: Option<u32>, api_key: &str) -> anyhow::Result<ParsedPageContent> {
        let python_script = format!(r#"
import json
import re
from mistralai import Mistral

api_key = os.getenv("MISTRAL_API_KEY", "{}")
client = Mistral(api_key=api_key)

ocr_text = '''{}'''

# Clean OCR
ocr_text = re.sub(r'([а-яa-z])\n\s*\1', r'\1', ocr_text)
ocr_text = re.sub(r'\n\s*\n+', '\n\n', ocr_text)

prompt = '''Ты - эксперт по анализу учебников. Разбери страницу и извлеки ВСЕ элементы.

ЭЛЕМЕНТЫ ДЛЯ ИЗВЛЕЧЕНИЯ:

1. МЕТАДАННЫЕ СТРАНИЦЫ:
   - Номер страницы (обычно вверху/внизу)
   - Название главы/раздела
   - Заголовок страницы

2. ТЕОРИЯ (важно!):
   - Определения ("Определение 1. ...")
   - Теоремы ("Теорема 1. ..." + доказательство)
   - Леммы, следствия
   - Свойства, аксиомы
   - Формулы (выделенные отдельно)
   - Методы решения

3. ПРИМЕРЫ:
   - Примеры с решениями ("Пример 1.")
   - Разбор задач

4. ЗАДАЧИ:
   - Номер + условие + подзадачи (а, б, в)

5. РИСУНКИ/ГРАФИКИ:
   - Описание изображений
   - Подписи к рисункам ("Рис. 1. ...")
   - Графики функций

6. ТАБЛИЦЫ:
   - Таблицы с данными

7. ЗАМЕЧАНИЯ:
   - Примечания, советы, предупреждения

8. УПРАЖНЕНИЯ:
   - Для самостоятельной работы

ФОРМАТ ОТВЕТА (строго JSON):
{{
  "metadata": {{
    "page_number": 15,
    "chapter_title": "Квадратные уравнения",
    "section_title": "Формула дискриминанта",
    "header": "...",
    "footer": "..."
  }},
  "elements": [
    {{
      "type": "theory",
      "theory_type": "definition",
      "title": "Квадратное уравнение",
      "number": "1",
      "content": "Квадратным уравнением называется...",
      "formulas": ["ax^2 + bx + c = 0"],
      "importance": "critical"
    }},
    {{
      "type": "theorem", 
      "theory_type": "theorem",
      "title": "Теорема Виета",
      "number": "2",
      "content": "Если x1, x2 - корни...",
      "formulas": ["x1 + x2 = -b/a", "x1 * x2 = c/a"],
      "importance": "critical"
    }},
    {{
      "type": "example",
      "number": "1",
      "problem": "Решить x^2 - 5x + 6 = 0",
      "solution": "D = 25 - 24 = 1...",
      "formulas": ["D = b^2 - 4ac"],
      "is_solved": true
    }},
    {{
      "type": "problem",
      "number": "125",
      "content": "Решите уравнение...",
      "sub_problems": [
        {{"letter": "а", "content": "x^2 = 4"}},
        {{"letter": "б", "content": "x^2 = 9"}}
      ],
      "difficulty": 5,
      "category": "квадратные уравнения"
    }},
    {{
      "type": "figure",
      "number": "1",
      "caption": "График параболы",
      "description": "Парабола y = x^2 с ветвями вверх...",
      "figure_type": "graph"
    }},
    {{
      "type": "remark",
      "remark_type": "note", 
      "content": "Обратите внимание..."
    }},
    {{
      "type": "text",
      "content": "Текстовый абзац...",
      "is_intro": false,
      "is_conclusion": false
    }}
  ],
  "stats": {{
    "problem_count": 5,
    "theory_count": 3,
    "example_count": 2,
    "figure_count": 1,
    "exercise_count": 0,
    "total_formulas": 8
  }}
}}

ВАЖНО:
- Извлекай ВСЕ элементы в порядке их появления
- Теория приоритетнее задач
- Сохраняй LaTeX формулы в content и formulas
- Если нет элемента, не включай его

OCR текст:
''' + ocr_text + '''

Верни ТОЛЬКО JSON, без markdown.'''

try:
    import os
    response = client.chat.complete(
        model="mistral-large-latest",
        messages=[{{"role": "user", "content": prompt}}],
        temperature=0.1,
        max_tokens=8000
    )
    
    result_text = response.choices[0].message.content.strip()
    result_text = re.sub(r'^```json\s*', '', result_text)
    result_text = re.sub(r'^```\s*', '', result_text)
    result_text = re.sub(r'```\s*$', '', result_text)
    result_text = result_text.strip()
    
    data = json.loads(result_text)
    print(json.dumps(data, ensure_ascii=False))
    
except Exception as e:
    print(json.dumps({{"error": str(e)}}, ensure_ascii=False))
    raise
"#, api_key, text.replace("'''", "'''"));

        let output = std::process::Command::new("python3")
            .arg("-c")
            .arg(&python_script)
            .env("MISTRAL_API_KEY", api_key)
            .output()
            .map_err(|e| anyhow::anyhow!("Failed to run Python: {}", e))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("AI parsing failed: {}", stderr));
        }

        let result: ParsedPageContent = serde_json::from_str(&stdout)
            .map_err(|e| anyhow::anyhow!("Failed to parse AI response: {}. Output: {}", e, stdout))?;

        Ok(result)
    }
    
    /// Regex-based fallback parser
    fn regex_parse_page(&self, text: &str, page_num: Option<u32>) -> ParsedPageContent {
        let mut elements = Vec::new();
        
        // Extract metadata
        let metadata = self.extract_metadata(text, page_num);
        
        // Parse line by line
        let lines: Vec<&str> = text.lines().collect();
        let mut i = 0;
        
        while i < lines.len() {
            let line = lines[i].trim();
            
            // Skip empty lines
            if line.is_empty() {
                i += 1;
                continue;
            }
            
            // Try to identify element type
            if let Some(theory) = self.try_parse_theory(&lines, i) {
                i = theory.1;
                elements.push(PageElement::Theory(theory.0));
            } else if let Some(example) = self.try_parse_example(&lines, i) {
                i = example.1;
                elements.push(PageElement::Example(example.0));
            } else if let Some(problem) = self.try_parse_problem(&lines, i) {
                i = problem.1;
                elements.push(PageElement::Problem(problem.0));
            } else if let Some(figure) = self.try_parse_figure(&lines, i) {
                i = figure.1;
                elements.push(PageElement::Figure(figure.0));
            } else if let Some(remark) = self.try_parse_remark(&lines, i) {
                i = remark.1;
                elements.push(PageElement::Remark(remark.0));
            } else {
                // Regular text
                let (text_elem, next_i) = self.parse_text_block(&lines, i);
                i = next_i;
                if !text_elem.content.is_empty() {
                    elements.push(PageElement::Text(text_elem));
                }
            }
        }
        
        // Calculate stats
        let stats = self.calculate_stats(&elements);
        
        ParsedPageContent {
            metadata,
            elements,
            stats,
        }
    }
    
    fn extract_metadata(&self, text: &str, page_num: Option<u32>) -> PageMetadata {
        use regex::Regex;
        
        let mut metadata = PageMetadata {
            page_number: page_num,
            chapter_title: None,
            section_title: None,
            header: None,
            footer: None,
        };
        
        // Try to find page number in text
        let page_re = Regex::new(r"(?m)^\s*(\d+)\s*$").unwrap();
        if let Some(caps) = page_re.captures(text) {
            if let Ok(num) = caps[1].parse::<u32>() {
                if metadata.page_number.is_none() {
                    metadata.page_number = Some(num);
                }
            }
        }
        
        // Try to find chapter/section headers
        let chapter_re = Regex::new(r"(?i)глава\s+(\d+)[.:\s]+(.+)").unwrap();
        if let Some(caps) = chapter_re.captures(text) {
            metadata.chapter_title = Some(caps[2].trim().to_string());
        }
        
        metadata
    }
    
    fn try_parse_theory(&self, lines: &[&str], start: usize) -> Option<(ParsedTheory, usize)> {
        use regex::Regex;
        
        let line = lines[start].trim();
        
        // Patterns for theory elements
        let patterns = vec![
            (r"(?i)^\s*определение\s*(\d*)[.:\s]*(.+)", TheoryElementType::Definition),
            (r"(?i)^\s*теорема\s*(\d*)[.:\s]*(.+)", TheoryElementType::Theorem),
            (r"(?i)^\s*лемма\s*(\d*)[.:\s]*(.+)", TheoryElementType::Lemma),
            (r"(?i)^\s*следствие\s*(\d*)[.:\s]*(.+)", TheoryElementType::Corollary),
            (r"(?i)^\s*свойство\s*(\d*)[.:\s]*(.+)", TheoryElementType::Property),
            (r"(?i)^\s*аксиома\s*(\d*)[.:\s]*(.+)", TheoryElementType::Axiom),
            (r"(?i)^\s*формула\s*(\d*)[.:\s]*(.+)", TheoryElementType::Formula),
        ];
        
        for (pattern, theory_type) in patterns {
            let re = Regex::new(pattern).unwrap();
            if let Some(caps) = re.captures(line) {
                let number = caps.get(1).map(|m| m.as_str().trim().to_string())
                    .filter(|s| !s.is_empty());
                let title = caps.get(2).map(|m| m.as_str().trim().to_string())
                    .filter(|s| !s.is_empty());
                
                // Collect content until next theory element or problem
                let mut content_lines = vec![];
                let mut i = start + 1;
                
                while i < lines.len() {
                    let next = lines[i].trim();
                    if next.is_empty() {
                        i += 1;
                        continue;
                    }
                    // Stop at next theory or problem
                    if Regex::new(r"(?i)^(определение|теорема|лемма|следствие|задача|пример|\d+[.)]\s)").unwrap().is_match(next) {
                        break;
                    }
                    content_lines.push(next);
                    i += 1;
                }
                
                let content = content_lines.join("\n");
                let formulas = self.extract_formulas(&content);
                
                return Some((ParsedTheory {
                    theory_type,
                    title,
                    number,
                    content,
                    formulas,
                    importance: ImportanceLevel::Important,
                }, i));
            }
        }
        
        None
    }
    
    fn try_parse_example(&self, lines: &[&str], start: usize) -> Option<(ParsedExample, usize)> {
        use regex::Regex;
        
        let line = lines[start].trim();
        let re = Regex::new(r"(?i)^\s*пример\s*(\d*)[.:\s]*(.+)?").unwrap();
        
        if let Some(caps) = re.captures(line) {
            let number = caps.get(1).map(|m| m.as_str().trim().to_string())
                .filter(|s| !s.is_empty());
            let title = caps.get(2).map(|m| m.as_str().trim().to_string())
                .filter(|s| !s.is_empty());
            
            let mut problem_lines = vec![];
            let mut solution_lines = vec![];
            let mut in_solution = false;
            let mut i = start + 1;
            
            while i < lines.len() {
                let next = lines[i].trim();
                if next.is_empty() {
                    i += 1;
                    continue;
                }
                
                // Check for solution marker
                if Regex::new(r"(?i)^(решение|доказательство|ответ)[:\s]").unwrap().is_match(next) {
                    in_solution = true;
                    i += 1;
                    continue;
                }
                
                // Stop at next element
                if Regex::new(r"(?i)^(пример|задача|теорема|определение|\d+[.)]\s)").unwrap().is_match(next) {
                    break;
                }
                
                if in_solution {
                    solution_lines.push(next);
                } else {
                    problem_lines.push(next);
                }
                i += 1;
            }
            
            let problem = problem_lines.join("\n");
            let solution = solution_lines.join("\n");
            let is_solved = !solution.is_empty();
            let formulas = self.extract_formulas(&format!("{} {}", problem, solution));
            
            return Some((ParsedExample {
                number,
                title,
                problem,
                solution,
                formulas,
                is_solved,
            }, i));
        }
        
        None
    }
    
    fn try_parse_problem(&self, lines: &[&str], start: usize) -> Option<(ParsedProblem, usize)> {
        use regex::Regex;
        
        let line = lines[start].trim();
        
        // Problem patterns
        let patterns = vec![
            r"^\s*(\d+)\s*[.\)]\s*(.+)",  // 123. text or 123) text
            r"(?i)^\s*задача\s*(\d+)[.:\s]+(.+)",  // Задача 123. text
        ];
        
        for pattern in patterns {
            let re = Regex::new(pattern).unwrap();
            if let Some(caps) = re.captures(line) {
                let number = caps[1].to_string();
                let content = caps[2].to_string();
                
                // Collect content and sub-problems
                let mut content_lines = vec![content];
                let mut sub_problems = vec![];
                let mut i = start + 1;
                
                while i < lines.len() {
                    let next = lines[i].trim();
                    if next.is_empty() {
                        i += 1;
                        continue;
                    }
                    
                    // Check for sub-problem
                    if let Some(sub_caps) = Regex::new(r"^\s*([а-яa-z])\s*[\)]\s*(.+)").unwrap().captures(next) {
                        let letter = sub_caps[1].to_string();
                        let sub_content = sub_caps[2].to_string();
                        sub_problems.push(ParsedSubProblem {
                            letter,
                            content: sub_content,
                        });
                        i += 1;
                        continue;
                    }
                    
                    // Stop at next problem or element
                    if Regex::new(r"(?i)^(задача|пример|теорема|определение|\d+[.\)]\s)").unwrap().is_match(next) {
                        break;
                    }
                    
                    content_lines.push(next.to_string());
                    i += 1;
                }
                
                let full_content = content_lines.join("\n");
                let formulas = self.extract_formulas(&full_content);
                
                return Some((ParsedProblem {
                    number,
                    content: full_content,
                    sub_problems,
                    difficulty: None,
                    category: None,
                    formulas,
                }, i));
            }
        }
        
        None
    }
    
    fn try_parse_figure(&self, lines: &[&str], start: usize) -> Option<(ParsedFigure, usize)> {
        use regex::Regex;
        
        let line = lines[start].trim();
        
        // Figure patterns
        let patterns = vec![
            r"(?i)^\s*рис[.унок]*\s*(\d+)[.:\s]*(.+)",
            r"(?i)^\s*график\s*(\d*)[.:\s]*(.+)?",
            r"(?i)^\s*диаграмма\s*(\d*)[.:\s]*(.+)?",
            r"(?i)^\s*таблица\s*(\d+)[.:\s]*(.+)",
        ];
        
        for pattern in patterns {
            let re = Regex::new(pattern).unwrap();
            if let Some(caps) = re.captures(line) {
                let number = caps.get(1).map(|m| m.as_str().trim().to_string())
                    .filter(|s| !s.is_empty());
                let caption = caps.get(2).map(|m| m.as_str().trim().to_string())
                    .filter(|s| !s.is_empty());
                
                let figure_type = if line.to_lowercase().contains("график") {
                    FigureType::Graph
                } else if line.to_lowercase().contains("диаграмма") {
                    FigureType::Chart
                } else if line.to_lowercase().contains("таблица") {
                    FigureType::Table
                } else {
                    FigureType::Illustration
                };
                
                return Some((ParsedFigure {
                    number,
                    caption,
                    description: "Изображение из OCR".to_string(),
                    image_reference: None,
                    figure_type,
                }, start + 1));
            }
        }
        
        None
    }
    
    fn try_parse_remark(&self, lines: &[&str], start: usize) -> Option<(ParsedRemark, usize)> {
        use regex::Regex;
        
        let line = lines[start].trim();
        
        let patterns = vec![
            (r"(?i)^\s*замечани[ея][.:\s]*(.+)", RemarkType::Note),
            (r"(?i)^\s*примечани[ея][.:\s]*(.+)", RemarkType::Note),
            (r"(?i)^\s*совет[.:\s]*(.+)", RemarkType::Tip),
            (r"(?i)^\s*важно[.:\s]*(.+)", RemarkType::Important),
            (r"(?i)^\s*внимание[.:\s]*(.+)", RemarkType::Warning),
            (r"(?i)^\s*запомните[.:\s]*(.+)", RemarkType::Remember),
        ];
        
        for (pattern, remark_type) in patterns {
            let re = Regex::new(pattern).unwrap();
            if let Some(caps) = re.captures(line) {
                let content = caps[1].to_string();
                return Some((ParsedRemark {
                    content,
                    remark_type,
                }, start + 1));
            }
        }
        
        None
    }
    
    fn parse_text_block(&self, lines: &[&str], start: usize) -> (ParsedText, usize) {
        let mut content_lines = vec![];
        let mut i = start;
        
        while i < lines.len() {
            let line = lines[i].trim();
            if line.is_empty() {
                i += 1;
                continue;
            }
            
            // Stop at any recognizable element
            if self.is_element_start(line) {
                break;
            }
            
            content_lines.push(line);
            i += 1;
            
            // Limit text block size
            if content_lines.len() >= 5 {
                break;
            }
        }
        
        (ParsedText {
            content: content_lines.join(" "),
            is_intro: start == 0,
            is_conclusion: false,
        }, i)
    }
    
    fn is_element_start(&self, line: &str) -> bool {
        use regex::Regex;
        let patterns = [
            r"(?i)^(определение|теорема|лемма|следствие|свойство|аксиома|формула)",
            r"(?i)^(пример|задача|упражнение)",
            r"(?i)^(рис[.унок]*|график|диаграмма|таблица)",
            r"(?i)^(замечани|примечани|совет|важно|внимание|запомните)",
            r"^\s*\d+\s*[.\)]\s+",
            r"^\s*[а-яa-z]\s*[\)]\s+",
        ];
        
        for pattern in patterns {
            if Regex::new(pattern).unwrap().is_match(line) {
                return true;
            }
        }
        false
    }
    
    fn extract_formulas(&self, text: &str) -> Vec<String> {
        let mut formulas = Vec::new();
        let re = regex::Regex::new(r"\$([^$]+)\$").unwrap();
        for cap in re.captures_iter(text) {
            formulas.push(cap[1].to_string());
        }
        formulas
    }
    
    fn calculate_stats(&self, elements: &[PageElement]) -> PageStats {
        let mut stats = PageStats {
            problem_count: 0,
            theory_count: 0,
            example_count: 0,
            figure_count: 0,
            exercise_count: 0,
            total_formulas: 0,
        };
        
        for elem in elements {
            match elem {
                PageElement::Problem(p) => {
                    stats.problem_count += 1;
                    stats.total_formulas += p.formulas.len();
                }
                PageElement::Theory(t) => {
                    stats.theory_count += 1;
                    stats.total_formulas += t.formulas.len();
                }
                PageElement::Example(e) => {
                    stats.example_count += 1;
                    stats.total_formulas += e.formulas.len();
                }
                PageElement::Figure(_) => stats.figure_count += 1,
                PageElement::Exercise(_) => stats.exercise_count += 1,
                _ => {}
            }
        }
        
        stats
    }
}

impl Default for PageContentParser {
    fn default() -> Self {
        Self::new(None)
    }
}

/// Convert parsed content to database models
pub fn convert_to_models(
    parsed: ParsedPageContent,
    book_id: &str,
    chapter_num: u32,
) -> (Vec<Problem>, Vec<TheoryBlock>) {
    let mut problems = Vec::new();
    let mut theories = Vec::new();
    let mut theory_counter = 0;
    
    for elem in parsed.elements {
        match elem {
            PageElement::Problem(p) => {
                let problem_id = format!("{}:{}:{}", book_id, chapter_num, p.number);
                problems.push(Problem {
                    id: problem_id,
                    chapter_id: format!("{}:{}", book_id, chapter_num),
                    page_id: None,
                    parent_id: None,
                    number: p.number.clone(),
                    display_name: format!("Задача {}", p.number),
                    content: p.content,
                    latex_formulas: p.formulas,
                    page_number: None,
                    difficulty: p.difficulty,
                    has_solution: false,
                    created_at: chrono::Utc::now(),
                    solution: None,
                    sub_problems: None,
                    continues_from_page: None,
                    continues_to_page: None,
                    is_cross_page: false,
                    is_bookmarked: false,
                });
            }
            PageElement::Theory(t) => {
                theory_counter += 1;
                let theory_id = format!("{}:{}:T:{}", book_id, chapter_num, theory_counter);
                
                let theory_type = match t.theory_type {
                    TheoryElementType::Definition => TheoryType::Definition,
                    TheoryElementType::Theorem => TheoryType::Theorem,
                    TheoryElementType::Lemma => TheoryType::Property,
                    TheoryElementType::Corollary => TheoryType::Property,
                    TheoryElementType::Property => TheoryType::Property,
                    TheoryElementType::Axiom => TheoryType::Definition,
                    TheoryElementType::Formula => TheoryType::Formula,
                    TheoryElementType::Postulate => TheoryType::Definition,
                    TheoryElementType::Rule => TheoryType::Property,
                    TheoryElementType::Method => TheoryType::Explanation,
                };
                
                theories.push(TheoryBlock {
                    id: theory_id,
                    chapter_id: format!("{}:{}", book_id, chapter_num),
                    block_num: theory_counter,
                    title: t.title.or_else(|| Some(format!("{:?}", t.theory_type))),
                    block_type: theory_type,
                    content: t.content,
                    latex_formulas: t.formulas,
                    page_number: None,
                    created_at: chrono::Utc::now(),
                });
            }
            _ => {} // Other elements not stored in DB yet
        }
    }
    
    (problems, theories)
}
