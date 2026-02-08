# Textbook Math Parser - Feature Guide

## Что реализовано

### 1. Структура данных для учебников

```
Book → Chapters → [Problems + Theory Blocks]
```

- **Book**: Метаданные учебника
- **Chapter**: Глава с задачами и теорией
- **Problem**: Задача с ID (например, `algebra-7:3:15`)
- **TheoryBlock**: Блок теории (определение, теорема, формула)
- **Solution**: AI-сгенерированное решение

### 2. Парсинг задач из OCR

Файл: `src/services/parser.rs`

```rust
let parser = TextbookParser::new();
let result = parser.parse(ocr_text, "algebra-7", 3);

// result.problems - список задач
// result.theory_blocks - список теории
```

**Распознаваемые паттерны:**
- `Задача 1: ...`
- `Задача №1. ...`
- `1) ...` или `1. ...`
- `№125. ...`
- `Упражнение 5: ...`
- `Теорема 1: ...`
- `Определение: ...`

### 3. AI Решение задач

Файл: `src/services/ai_solver.rs`

```rust
let solver = AISolver::new(&config)?;
let solution = solver.solve(&problem, Some("claude"), theory_context).await?;
```

**Поддерживаемые провайдеры:**
- Claude (рекомендуется)
- OpenAI GPT-4o
- Mistral

### 4. База данных (SQLite)

Таблицы:
- `books` - Учебники
- `chapters` - Главы
- `problems` - Задачи с LaTeX формулами
- `theory_blocks` - Теория
- `solutions` - Решения от AI

### 5. API Endpoints

**Получить задачи главы:**
```
GET /api/chapters/{chapter_id}/problems
```

**Получить решение (AI):**
```
POST /api/problems/{problem_id}/solve
Body: { "provider": "claude", "force_regenerate": false }
```

**Импорт OCR текста:**
```
POST /api/import
Body: {
  "book_id": "algebra-7",
  "chapter_num": 3,
  "chapter_title": "Квадратные уравнения",
  "text": "OCR текст..."
}
```

**Поиск по формуле:**
```
GET /api/search?formula=x^2+y^2
```

### 6. HTML Просмотр с KaTeX

**Список задач главы:**
```
GET /textbook/chapter/{chapter_id}
```

**Одна задача с решением:**
```
GET /textbook/problem/{problem_id}
```

В шаблонах используется KaTeX для рендеринга LaTeX формул.

## Использование

### 1. Добавить учебник

```bash
# 1. Положить PDF в resources/
# 2. Запустить OCR
# 3. Импортировать результат

curl -X POST http://localhost:8081/api/import \
  -H "Content-Type: application/json" \
  -d '{
    "book_id": "algebra-7",
    "chapter_num": 1,
    "chapter_title": "Действительные числа",
    "text": "OCR текст с задачами..."
  }'
```

### 2. Просмотр в браузере

1. Открыть главу: `http://localhost:8081/textbook/chapter/algebra-7:1`
2. Кликнуть на задачу
3. Нажать "Solve with AI"
4. Решение сохраняется в БД

### 3. API ключи для AI

```env
ANTHROPIC_API_KEY=your_key  # Рекомендуется для математики
OPENAI_API_KEY=your_key
MISTRAL_API_KEY=your_key
```

## Архитектура файлов

```
src/
├── models/
│   ├── mod.rs
│   └── problem.rs          # Problem, TheoryBlock, Solution
├── services/
│   ├── parser.rs           # Парсинг OCR → задачи
│   ├── ai_solver.rs        # AI решение задач
│   └── database.rs         # SQLite операции
├── handlers/
│   ├── problems.rs         # API endpoints
│   └── textbook.rs         # HTML views
templates/
├── textbook/
│   ├── chapter_problems.html  # Список задач
│   └── problem_view.html      # Одна задача с решением
```

## Расширение

### Добавить новый AI провайдер

```rust
#[async_trait]
impl SolutionProvider for MyProvider {
    async fn solve(&self, problem: &Problem, context: &str) -> Result<String> {
        // Вызвать API
    }
}
```

### Добавить новый паттерн задачи

В `src/services/parser.rs`:
```rust
let problem_patterns = vec![
    Regex::new(r"(?im)^\s*Вопрос\s*(\d+)[:.\s]+")?,
];
```

## Преимущества

1. **Уникальные ID**: Каждая задача имеет ID вида `book:chapter:number`
2. **LaTeX рендеринг**: Автоматический рендер формул в браузере
3. **AI решения**: Интеграция с Claude/OpenAI для решения
4. **Поиск по формулам**: Можно искать задачи по LaTeX формулам
5. **Сохранение**: Все решения сохраняются в БД
6. **Рейтинг**: Можно оценивать качество решений

## Дальнейшие улучшения

- [ ] Drag-and-drop импорт PDF
- [ ] Автоматическое разделение на главы
- [ ] Сравнение решений от разных AI
- [ ] Экспорт в Anki/Quizlet
- [ ] Генерация тестов по задачам
- [ ] Режим обучения (показывать подсказки)
