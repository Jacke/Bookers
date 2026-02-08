use crate::models::Problem;
use serde::{Deserialize, Serialize};
use std::process::Command;

/// AI-powered auto-tagger for problems
pub struct AutoTagger {
    api_key: Option<String>,
    local_classifier: LocalClassifier,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProblemTags {
    pub problem_id: String,
    pub tags: Vec<Tag>,
    pub difficulty: Option<u8>,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tag {
    pub name: String,
    pub category: TagCategory,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TagCategory {
    Subject,      // алгебра, геометрия, тригонометрия
    Topic,        // уравнения, функции, производные
    Method,       // метод неопределенных коэффициентов, замена переменной
    Difficulty,   // easy, medium, hard, olympiad
    Concept,      // дискриминант, логарифм, вектор
}

/// Local rule-based classifier (fallback)
pub struct LocalClassifier {
    rules: Vec<(Tag, Vec<String>)>,
}

impl AutoTagger {
    pub fn new(api_key: Option<String>) -> Self {
        Self {
            api_key,
            local_classifier: LocalClassifier::new(),
        }
    }

    /// Tag a single problem
    pub async fn tag_problem(&self, problem: &Problem) -> anyhow::Result<ProblemTags> {
        // Try AI tagging if API key available
        if let Some(ref key) = self.api_key {
            match self.ai_tag_problem(problem, key).await {
                Ok(tags) => return Ok(tags),
                Err(e) => {
                    log::warn!("AI tagging failed, using local classifier: {}", e);
                }
            }
        }

        // Fallback to local classifier
        Ok(self.local_classifier.tag_problem(problem))
    }

    /// Tag multiple problems (batch)
    pub async fn tag_problems(&self, problems: &[Problem]) -> Vec<ProblemTags> {
        let mut results = Vec::new();

        for problem in problems {
            match self.tag_problem(problem).await {
                Ok(tags) => results.push(tags),
                Err(e) => {
                    log::error!("Failed to tag problem {}: {}", problem.id, e);
                    // Add empty tags on error
                    results.push(ProblemTags {
                        problem_id: problem.id.clone(),
                        tags: vec![],
                        difficulty: None,
                        confidence: 0.0,
                    });
                }
            }
        }

        results
    }

    /// AI-powered tagging via Mistral
    async fn ai_tag_problem(&self, problem: &Problem, api_key: &str) -> anyhow::Result<ProblemTags> {
        let prompt = format!(r#"
Проанализируй математическую задачу и определи теги.

ЗАДАЧА #{}:
{}

Ответь в формате JSON:
{{
  "subject": "алгебра|геометрия|тригонометрия|калькулус|статистика",
  "topics": ["тема1", "тема2"],
  "methods": ["метод1", "метод2"],
  "concepts": ["концепт1", "концепт2"],
  "difficulty": 1-10,
  "difficulty_label": "easy|medium|hard|olympiad"
}}

Требования:
- subject: только одно значение
- topics: 1-3 темы
- methods: конкретные методы решения
- concepts: математические понятия в задаче
- difficulty: оценка сложности от 1 до 10
"#, problem.number, problem.content);

        let python_script = format!(r#"
import json
import os
from mistralai import Mistral

api_key = os.getenv("MISTRAL_API_KEY", "{}")
client = Mistral(api_key=api_key)

prompt = '''{}'''

try:
    response = client.chat.complete(
        model="mistral-large-latest",
        messages=[{{"role": "user", "content": prompt}}],
        temperature=0.1,
        max_tokens=1000
    )
    
    result_text = response.choices[0].message.content.strip()
    
    # Clean markdown
    result_text = result_text.replace("```json", "").replace("```", "").strip()
    
    data = json.loads(result_text)
    print(json.dumps(data, ensure_ascii=False))
    
except Exception as e:
    print(json.dumps({{"error": str(e)}}, ensure_ascii=False))
    raise
"#, api_key, prompt.replace("'''", "'''"));

        let output = Command::new("python3")
            .arg("-c")
            .arg(&python_script)
            .env("MISTRAL_API_KEY", api_key)
            .output()
            .map_err(|e| anyhow::anyhow!("Failed to run Python: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("AI tagging failed: {}", stderr));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let ai_result: AiTagResult = serde_json::from_str(&stdout)
            .map_err(|e| anyhow::anyhow!("Failed to parse AI response: {}. Output: {}", e, stdout))?;

        // Convert AI result to tags
        let mut tags = Vec::new();

        // Subject
        tags.push(Tag {
            name: ai_result.subject.clone(),
            category: TagCategory::Subject,
            confidence: 0.9,
        });

        // Topics
        for topic in &ai_result.topics {
            tags.push(Tag {
                name: topic.clone(),
                category: TagCategory::Topic,
                confidence: 0.85,
            });
        }

        // Methods
        for method in &ai_result.methods {
            tags.push(Tag {
                name: method.clone(),
                category: TagCategory::Method,
                confidence: 0.8,
            });
        }

        // Concepts
        for concept in &ai_result.concepts {
            tags.push(Tag {
                name: concept.clone(),
                category: TagCategory::Concept,
                confidence: 0.9,
            });
        }

        // Difficulty
        tags.push(Tag {
            name: ai_result.difficulty_label.clone(),
            category: TagCategory::Difficulty,
            confidence: 0.75,
        });

        Ok(ProblemTags {
            problem_id: problem.id.clone(),
            tags,
            difficulty: Some(ai_result.difficulty.clamp(1, 10) as u8),
            confidence: 0.85,
        })
    }
}

#[derive(Debug, Deserialize)]
struct AiTagResult {
    subject: String,
    topics: Vec<String>,
    methods: Vec<String>,
    concepts: Vec<String>,
    difficulty: u32,
    difficulty_label: String,
}

impl LocalClassifier {
    pub fn new() -> Self {
        let rules = vec![
            // Subjects
            (Tag { name: "алгебра".to_string(), category: TagCategory::Subject, confidence: 0.9 },
             vec!["уравнени".to_string(), "многочлен".to_string(), "корень".to_string(), "формула".to_string()]),
            
            (Tag { name: "геометрия".to_string(), category: TagCategory::Subject, confidence: 0.9 },
             vec!["треугольник".to_string(), "окружность".to_string(), "прямая".to_string(), "угол".to_string(), "сторона".to_string()]),
            
            (Tag { name: "тригонометрия".to_string(), category: TagCategory::Subject, confidence: 0.9 },
             vec!["sin".to_string(), "cos".to_string(), "tg".to_string(), "sinus".to_string(), "cosinus".to_string()]),
            
            (Tag { name: "калькулус".to_string(), category: TagCategory::Subject, confidence: 0.9 },
             vec!["производная".to_string(), "интеграл".to_string(), "предел".to_string(), "функция".to_string()]),
            
            // Topics
            (Tag { name: "квадратные уравнения".to_string(), category: TagCategory::Topic, confidence: 0.85 },
             vec!["квадратное уравнение".to_string(), "дискриминант".to_string(), "ax²".to_string()]),
            
            (Tag { name: "системы уравнений".to_string(), category: TagCategory::Topic, confidence: 0.85 },
             vec!["система".to_string(), "решите систему".to_string()]),
            
            (Tag { name: "неравенства".to_string(), category: TagCategory::Topic, confidence: 0.85 },
             vec!["неравенство".to_string(), ">".to_string(), "<".to_string(), "≥".to_string(), "≤".to_string()]),
            
            (Tag { name: "производные".to_string(), category: TagCategory::Topic, confidence: 0.85 },
             vec!["найдите производную".to_string(), "дифференцирование".to_string()]),
            
            (Tag { name: "интегралы".to_string(), category: TagCategory::Topic, confidence: 0.85 },
             vec!["вычислите интеграл".to_string(), "интегрирование".to_string()]),
            
            // Methods
            (Tag { name: "метод дискриминанта".to_string(), category: TagCategory::Method, confidence: 0.8 },
             vec!["дискриминант".to_string(), "D = b²".to_string()]),
            
            (Tag { name: "метод замены переменной".to_string(), category: TagCategory::Method, confidence: 0.8 },
             vec!["замена".to_string(), "подстановка".to_string()]),
            
            (Tag { name: "метод математической индукции".to_string(), category: TagCategory::Method, confidence: 0.8 },
             vec!["индукц".to_string(), "докажите для всех n".to_string()]),
            
            // Concepts
            (Tag { name: "дискриминант".to_string(), category: TagCategory::Concept, confidence: 0.9 },
             vec!["дискриминант".to_string(), "D = ".to_string()]),
            
            (Tag { name: "логарифм".to_string(), category: TagCategory::Concept, confidence: 0.9 },
             vec!["log".to_string(), "ln ".to_string(), "логарифм".to_string()]),
            
            (Tag { name: "вектор".to_string(), category: TagCategory::Concept, confidence: 0.9 },
             vec!["вектор".to_string(), "→".to_string()]),
        ];

        Self { rules }
    }

    pub fn tag_problem(&self, problem: &Problem) -> ProblemTags {
        let mut tags = Vec::new();
        let mut difficulty = None;

        // Apply rules
        for (tag, keywords) in &self.rules {
            for keyword in keywords {
                if problem.content.to_lowercase().contains(&keyword.to_lowercase()) {
                    // Avoid duplicates
                    if !tags.iter().any(|t: &Tag| t.name == tag.name && t.category == tag.category) {
                        tags.push(tag.clone());
                    }
                    break;
                }
            }
        }

        // Estimate difficulty based on content
        difficulty = Some(self.estimate_difficulty(problem));

        // Add difficulty tag
        let diff_label = match difficulty {
            Some(d) if d <= 3 => "easy",
            Some(d) if d <= 6 => "medium",
            Some(d) if d <= 8 => "hard",
            _ => "olympiad",
        };
        
        tags.push(Tag {
            name: diff_label.to_string(),
            category: TagCategory::Difficulty,
            confidence: 0.6,
        });

        let confidence = if tags.is_empty() { 0.3 } else { 0.6 };

        ProblemTags {
            problem_id: problem.id.clone(),
            tags,
            difficulty,
            confidence,
        }
    }

    fn estimate_difficulty(&self, problem: &Problem) -> u8 {
        let content = problem.content.to_lowercase();
        let mut score = 5u8;

        // Increase difficulty for advanced terms
        let hard_terms = ["докажите", "доказать", "покажите", "интеграл", "производная", "предел", "ряд"];
        for term in &hard_terms {
            if content.contains(term) {
                score += 1;
            }
        }

        // Decrease for simple terms
        let easy_terms = ["вычислите", "решите", "найдите значение"];
        for term in &easy_terms {
            if content.contains(term) {
                score = score.saturating_sub(1);
            }
        }

        // Length factor
        let words = content.split_whitespace().count();
        if words > 100 {
            score += 1;
        }
        if words > 200 {
            score += 1;
        }

        score.clamp(1, 10)
    }
}

impl Default for LocalClassifier {
    fn default() -> Self {
        Self::new()
    }
}

/// Batch auto-tagging for all problems in a chapter
pub async fn auto_tag_chapter(
    tagger: &AutoTagger,
    problems: &[Problem],
) -> Vec<ProblemTags> {
    tagger.tag_problems(problems).await
}
