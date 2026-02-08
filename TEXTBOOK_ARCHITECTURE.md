# Textbook Math Parser Architecture

## Goals
1. Parse textbooks (especially math) into structured format
2. Extract problems with unique IDs
3. Extract theory sections with IDs
4. Render LaTeX math in browser (MathJax/KaTeX)
5. Click problem → Get AI solution → Save solution

## Database Schema (SQLite/PostgreSQL)

```sql
-- Books
books (id, title, author, file_path, created_at)

-- Chapters/Sections
chapters (id, book_id, number, title, content, created_at)

-- Theory blocks
theory_blocks (
  id, 
  chapter_id, 
  block_number, -- Уникальный номер в главе (T-1, T-2, ...)
  title,
  content_markdown,
  latex_formulas[], -- массив формул для быстрого поиска
  created_at
)

-- Problems
problems (
  id,
  chapter_id,
  problem_number, -- Номер задачи (1.1, 1.2, 125, ...)
  display_id, -- Для отображения (задача 1, задача 2...)
  content_markdown,
  latex_formulas[],
  difficulty_score, -- AI оценка сложности (опционально)
  has_solution,
  created_at
)

-- Solutions (AI generated)
solutions (
  id,
  problem_id,
  provider, -- openai, claude, mathpix...
  content_markdown,
  latex_formulas[],
  is_verified, -- Пользователь подтвердил что решение верное
  rating, -- Оценка пользователя
  created_at,
  updated_at
)

-- Problem attempts (история решений пользователя)
problem_attempts (
  id,
  problem_id,
  user_answer,
  is_correct,
  attempt_date
)
```

## Data Models (Rust)

```rust
// Problem ID format: {book_id}:{chapter_num}:{problem_num}
// Example: "algebra-7:3:15" (книга algebra-7, глава 3, задача 15)

pub struct Problem {
    pub id: String,
    pub chapter_id: String,
    pub number: String, // "1.1", "125"
    pub content: String, // Markdown with LaTeX
    pub latex_formulas: Vec<String>,
    pub solution: Option<Solution>,
}

pub struct TheoryBlock {
    pub id: String,
    pub chapter_id: String,
    pub block_num: u32,
    pub title: Option<String>,
    pub content: String, // Markdown with LaTeX
}

pub struct Solution {
    pub id: String,
    pub problem_id: String,
    pub content: String,
    pub provider: String,
    pub created_at: DateTime<Utc>,
}
```

## LaTeX Problem Detection Patterns

```rust
// Паттерны для определения задач в тексте
const PROBLEM_PATTERNS: &[&str] = &[
    r"(?i)^\s*#*\s*Задача\s*[№#]?\s*(\d+[\.\d]*)[.:\s]", // Задача 1.1: ...
    r"(?i)^\s*#*\s*Упражнение\s*[№#]?\s*(\d+)[.:\s]", // Упражнение 5: ...
    r"(?i)^\s*(\d+)\s*[.\)]\s*\$[^$]+\$", // 1) $formula$ или 1. $formula$
    r"(?i)^\s*#*\s*Example\s*[№#]?\s*(\d+)[.:\s]", // Example 1: ...
    r"(?i)^\s*#*\s*Problem\s*[№#]?\s*(\d+)[.:\s]", // Problem 1: ...
];

// Паттерны для теории
const THEORY_PATTERNS: &[&str] = &[
    r"(?i)^\s*#*\s*Теорема\s*[№#]?\s*(\d*)[.:\s]", // Теорема 1:
    r"(?i)^\s*#*\s*Определение\s*[№#]?\s*(\d*)[.:\s]", // Определение:
    r"(?i)^\s*#*\s*Свойство\s*[№#]?\s*(\d*)[.:\s]", // Свойство 1:
    r"(?i)^\s*#*\s*Формула\s*[№#]?\s*(\d*)[.:\s]", // Формула:
];
```

## API Endpoints

```rust
// Получить все задачи главы
GET /api/chapters/{chapter_id}/problems

// Получить конкретную задачу с решением
GET /api/problems/{problem_id}

// Получить решение задачи (сгенерировать или из кэша)
POST /api/problems/{problem_id}/solve
Body: { provider: "claude" | "openai" | "mistral", force_regenerate: bool }

// Сохранить/обновить решение
PUT /api/problems/{problem_id}/solution
Body: { content: "..." }

// Получить теорию главы
GET /api/chapters/{chapter_id}/theory

// Получить конкретный блок теории
GET /api/theory/{theory_id}

// Поиск по формулам
GET /api/search?formula="x^2+y^2"
```

## Frontend Structure

```
templates/
├── textbook/
│   ├── viewer.html      -- Основной просмотрщик учебника
│   ├── problem_list.html -- Список задач главы
│   ├── problem_view.html -- Одна задача с решением
│   └── theory_view.html  -- Теория
├── components/
│   ├── math_renderer.html -- MathJax/KaTeX инициализация
│   ├── problem_card.html  -- Карточка задачи
│   └── solution_view.html -- Отображение решения
```

## Math Rendering (Frontend)

```html
<!-- MathJax Configuration -->
<script>
window.MathJax = {
  tex: {
    inlineMath: [['$', '$'], ['\\(', '\\)']],
    displayMath: [['$$', '$$'], ['\\[', '\\]']],
    processEscapes: true,
    processEnvironments: true
  },
  options: {
    skipHtmlTags: ['script', 'noscript', 'style', 'textarea', 'pre']
  }
};
</script>
<script src="https://cdn.jsdelivr.net/npm/mathjax@3/es5/tex-chtml.js"></script>

<!-- Или KaTeX (быстрее) -->
<link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/katex@0.16.9/dist/katex.min.css">
<script src="https://cdn.jsdelivr.net/npm/katex@0.16.9/dist/katex.min.js"></script>
<script src="https://cdn.jsdelivr.net/npm/katex@0.16.9/dist/contrib/auto-render.min.js"></script>
```

## AI Solution Generation

```rust
pub struct AISolver {
    providers: HashMap<String, Box<dyn SolutionProvider>>,
}

#[async_trait]
pub trait SolutionProvider: Send + Sync {
    async fn solve(&self, problem: &Problem, context: &str) -> Result<String, Error>;
}

// Промпт для решения задачи
const SOLUTION_PROMPT: &str = r#"
Ты - опытный преподаватель математики. Реши следующую задачу подробно,
объясняя каждый шаг. Используй LaTeX для форматирования формул.

Задача: {problem_content}

Контекст из учебника (теория):
{theory_context}

Требования:
1. Дай подробное решение
2. Объясни каждый шаг
3. Используй LaTeX для формул ($...$ для inline, $$...$$ для display)
4. Если есть несколько способов решения, покажи основной
5. В конце дай краткий ответ

Решение:
"#;
```

## Problem Detection Algorithm

```rust
pub struct TextbookParser;

impl TextbookParser {
    pub fn parse_ocr_text(text: &str, chapter_id: &str) -> ParseResult {
        let mut problems = Vec::new();
        let mut theory_blocks = Vec::new();
        let mut current_block = String::new();
        let mut block_num = 0u32;
        
        for line in text.lines() {
            // Проверяем является ли строка началом задачи
            if let Some(problem_num) = Self::detect_problem_start(line) {
                // Сохраняем предыдущий блок как теорию
                if !current_block.is_empty() {
                    block_num += 1;
                    theory_blocks.push(TheoryBlock {
                        id: format!("{}:T:{}", chapter_id, block_num),
                        chapter_id: chapter_id.to_string(),
                        block_num,
                        content: current_block.clone(),
                        ..Default::default()
                    });
                    current_block.clear();
                }
                
                // Начинаем новую задачу
                problems.push(Problem {
                    id: format!("{}:P:{}", chapter_id, problem_num),
                    chapter_id: chapter_id.to_string(),
                    number: problem_num,
                    content: line.to_string(),
                    ..Default::default()
                });
            } else {
                current_block.push_str(line);
                current_block.push('\n');
            }
        }
        
        ParseResult { problems, theory_blocks }
    }
}
```

## File Structure Changes

```
src/
├── models/
│   ├── mod.rs
│   ├── problem.rs      -- Problem, Solution, TheoryBlock
│   ├── chapter.rs      -- Chapter, Book
│   └── user.rs         -- User progress
├── services/
│   ├── mod.rs
│   ├── ocr.rs          -- Существующий OCR
│   ├── parser.rs       -- Парсер задач из OCR текста
│   ├── ai_solver.rs    -- AI решение задач
│   └── database.rs     -- Работа с БД
├── handlers/
│   ├── mod.rs
│   ├── problems.rs     -- API для задач
│   ├── solutions.rs    -- API для решений
│   └── theory.rs       -- API для теории
├── templates/
│   └── ...
```
