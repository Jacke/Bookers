use crate::models::Problem;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Similar problems detector
pub struct SimilarityDetector {
    formula_weight: f64,
    text_weight: f64,
    concept_weight: f64,
    min_similarity: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimilarityResult {
    pub problem_id: String,
    pub similar_problems: Vec<SimilarProblem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimilarProblem {
    pub problem_id: String,
    pub similarity: f64, // 0.0 - 1.0
    pub match_type: MatchType,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchType {
    ExactFormula,      // Одинаковые формулы
    SimilarFormula,    // Похожие формулы
    SharedConcepts,    // Общие концепции
    SameTopic,         // Одна тема
    TextSimilarity,    // Похожий текст
}

/// Feature vector for problem comparison
#[derive(Clone)]
struct ProblemFeatures {
    problem_id: String,
    formulas: HashSet<String>,
    concepts: HashSet<String>,
    text_tokens: HashSet<String>,
    difficulty: Option<u8>,
}

impl SimilarityDetector {
    pub fn new() -> Self {
        Self {
            formula_weight: 0.5,
            text_weight: 0.2,
            concept_weight: 0.3,
            min_similarity: 0.3,
        }
    }

    /// Configure weights
    pub fn with_weights(
        mut self,
        formula: f64,
        text: f64,
        concept: f64,
    ) -> Self {
        self.formula_weight = formula;
        self.text_weight = text;
        self.concept_weight = concept;
        self
    }

    /// Find similar problems for a given problem
    pub fn find_similar(
        &self,
        problem: &Problem,
        candidates: &[Problem],
        top_k: usize,
    ) -> SimilarityResult {
        let target_features = self.extract_features(problem);
        
        let mut similarities: Vec<SimilarProblem> = candidates
            .iter()
            .filter(|p| p.id != problem.id) // Exclude self
            .filter_map(|candidate| {
                let candidate_features = self.extract_features(candidate);
                let similarity = self.calculate_similarity(&target_features, &candidate_features);
                
                if similarity >= self.min_similarity {
                    let match_type = self.determine_match_type(&target_features, &candidate_features);
                    let reason = self.generate_reason(&match_type, &target_features, &candidate_features);
                    
                    Some(SimilarProblem {
                        problem_id: candidate.id.clone(),
                        similarity,
                        match_type,
                        reason,
                    })
                } else {
                    None
                }
            })
            .collect();

        // Sort by similarity descending
        similarities.sort_by(|a, b| b.similarity.partial_cmp(&a.similarity).unwrap());
        similarities.truncate(top_k);

        SimilarityResult {
            problem_id: problem.id.clone(),
            similar_problems: similarities,
        }
    }

    /// Batch find similar problems for all problems
    pub fn find_all_similar(
        &self,
        problems: &[Problem],
        top_k: usize,
    ) -> Vec<SimilarityResult> {
        problems
            .iter()
            .map(|problem| self.find_similar(problem, problems, top_k))
            .collect()
    }

    /// Build similarity matrix
    pub fn build_similarity_matrix(&self, problems: &[Problem]) -> SimilarityMatrix {
        let n = problems.len();
        let mut matrix = vec![vec![0.0; n]; n];
        let mut feature_cache: HashMap<String, ProblemFeatures> = HashMap::new();

        // Pre-compute features
        for (i, problem) in problems.iter().enumerate() {
            let features = self.extract_features(problem);
            feature_cache.insert(problem.id.clone(), features);
        }

        // Compute similarities
        for i in 0..n {
            for j in (i + 1)..n {
                let f1 = feature_cache.get(&problems[i].id).unwrap();
                let f2 = feature_cache.get(&problems[j].id).unwrap();
                
                let sim = self.calculate_similarity(f1, f2);
                matrix[i][j] = sim;
                matrix[j][i] = sim;
            }
        }

        let ids: Vec<String> = problems.iter().map(|p| p.id.clone()).collect();

        SimilarityMatrix {
            problem_ids: ids,
            matrix,
        }
    }

    /// Extract features from problem
    fn extract_features(&self, problem: &Problem) -> ProblemFeatures {
        let formulas: HashSet<String> = problem.latex_formulas.iter().cloned().collect();
        
        let concepts = self.extract_concepts(&problem.content);
        
        let text_tokens: HashSet<String> = problem.content
            .to_lowercase()
            .split_whitespace()
            .map(|s| s.trim_matches(|c: char| !c.is_alphanumeric()).to_string())
            .filter(|s| s.len() > 3)
            .filter(|s| !self.is_stop_word(s))
            .collect();

        ProblemFeatures {
            problem_id: problem.id.clone(),
            formulas,
            concepts,
            text_tokens,
            difficulty: problem.difficulty,
        }
    }

    /// Extract concepts from text
    fn extract_concepts(&self, text: &str) -> HashSet<String> {
        let concepts = vec![
            "уравнение", "неравенство", "функция", "производная", "интеграл",
            "предел", "логарифм", "экспонента", "тригонометрия", "вектор",
            "матрица", "определитель", "ряд", "многочлен", "корень",
            "дискриминант", "парабола", "окружность", "треугольник", "угол",
            "площадь", "объём", "периметр", "гипербола", "эллипс",
        ];

        let text_lower = text.to_lowercase();
        concepts
            .into_iter()
            .filter(|c| text_lower.contains(c))
            .map(|s| s.to_string())
            .collect()
    }

    /// Check if word is a stop word
    fn is_stop_word(&self, word: &str) -> bool {
        let stop_words: HashSet<&str> = [
            "что", "как", "где", "когда", "кто", "почему", "зачем",
            "the", "and", "for", "are", "but", "not", "you",
            "решите", "найдите", "вычислите", "докажите", "покажите",
            "this", "that", "with", "from", "have", "has",
        ].iter().cloned().collect();
        
        stop_words.contains(word)
    }

    /// Calculate similarity between two feature vectors
    fn calculate_similarity(
        &self,
        a: &ProblemFeatures,
        b: &ProblemFeatures,
    ) -> f64 {
        let formula_sim = self.jaccard_similarity(&a.formulas, &b.formulas);
        let concept_sim = self.jaccard_similarity(&a.concepts, &b.concepts);
        let text_sim = self.jaccard_similarity(&a.text_tokens, &b.text_tokens);

        // Weighted combination
        let similarity = formula_sim * self.formula_weight
            + concept_sim * self.concept_weight
            + text_sim * self.text_weight;

        // Boost for similar difficulty
        let difficulty_boost = match (a.difficulty, b.difficulty) {
            (Some(d1), Some(d2)) => {
                let diff = (d1 as i16 - d2 as i16).abs();
                if diff <= 1 { 0.1 } else { 0.0 }
            }
            _ => 0.0,
        };

        (similarity + difficulty_boost).min(1.0)
    }

    /// Jaccard similarity: |A ∩ B| / |A ∪ B|
    fn jaccard_similarity<T: Eq + std::hash::Hash>(
        &self,
        a: &HashSet<T>,
        b: &HashSet<T>,
    ) -> f64 {
        if a.is_empty() && b.is_empty() {
            return 1.0;
        }
        if a.is_empty() || b.is_empty() {
            return 0.0;
        }

        let intersection: HashSet<_> = a.intersection(b).collect();
        let union: HashSet<_> = a.union(b).collect();

        intersection.len() as f64 / union.len() as f64
    }

    /// Determine match type based on features
    fn determine_match_type(
        &self,
        a: &ProblemFeatures,
        b: &ProblemFeatures,
    ) -> MatchType {
        let formula_sim = self.jaccard_similarity(&a.formulas, &b.formulas);
        let concept_sim = self.jaccard_similarity(&a.concepts, &b.concepts);

        if formula_sim > 0.8 {
            MatchType::ExactFormula
        } else if formula_sim > 0.3 {
            MatchType::SimilarFormula
        } else if concept_sim > 0.7 {
            MatchType::SameTopic
        } else if concept_sim > 0.3 {
            MatchType::SharedConcepts
        } else {
            MatchType::TextSimilarity
        }
    }

    /// Generate human-readable reason
    fn generate_reason(
        &self,
        match_type: &MatchType,
        a: &ProblemFeatures,
        b: &ProblemFeatures,
    ) -> String {
        match match_type {
            MatchType::ExactFormula => "Одинаковые математические формулы".to_string(),
            MatchType::SimilarFormula => "Похожие математические выражения".to_string(),
            MatchType::SharedConcepts => {
                let shared: Vec<_> = a.concepts.intersection(&b.concepts).collect();
                if shared.len() <= 3 {
                    format!("Общие понятия: {}", shared.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "))
                } else {
                    format!("{} общих математических понятий", shared.len())
                }
            }
            MatchType::SameTopic => "Одна тема/раздел".to_string(),
            MatchType::TextSimilarity => "Схожая формулировка".to_string(),
        }
    }

    /// Find duplicate problems (very high similarity)
    pub fn find_duplicates(&self, problems: &[Problem], threshold: f64) -> Vec<DuplicateGroup> {
        let matrix = self.build_similarity_matrix(problems);
        let mut visited: HashSet<usize> = HashSet::new();
        let mut groups = Vec::new();

        for i in 0..problems.len() {
            if visited.contains(&i) {
                continue;
            }

            let mut group = vec![matrix.problem_ids[i].clone()];
            visited.insert(i);

            for j in (i + 1)..problems.len() {
                if !visited.contains(&j) && matrix.matrix[i][j] >= threshold {
                    group.push(matrix.problem_ids[j].clone());
                    visited.insert(j);
                }
            }

            if group.len() > 1 {
                groups.push(DuplicateGroup {
                    problem_ids: group,
                    similarity: matrix.matrix[i][i + 1], // Representative similarity
                });
            }
        }

        groups
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimilarityMatrix {
    pub problem_ids: Vec<String>,
    pub matrix: Vec<Vec<f64>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DuplicateGroup {
    pub problem_ids: Vec<String>,
    pub similarity: f64,
}

impl Default for SimilarityDetector {
    fn default() -> Self {
        Self::new()
    }
}

/// Recommendation engine based on similarity
pub struct ProblemRecommender {
    similarity_detector: SimilarityDetector,
}

impl ProblemRecommender {
    pub fn new() -> Self {
        Self {
            similarity_detector: SimilarityDetector::new(),
        }
    }

    /// Recommend similar problems to practice
    pub fn recommend_for_practice(
        &self,
        solved_problems: &[Problem],
        all_problems: &[Problem],
        count: usize,
    ) -> Vec<RecommendedProblem> {
        // Get all similar problems to solved ones
        let mut recommendations: Vec<RecommendedProblem> = Vec::new();
        let mut seen_ids: HashSet<String> = solved_problems.iter().map(|p| p.id.clone()).collect();

        for solved in solved_problems {
            let similar = self.similarity_detector.find_similar(solved, all_problems, 3);
            
            for sim in similar.similar_problems {
                if seen_ids.contains(&sim.problem_id) {
                    continue;
                }
                seen_ids.insert(sim.problem_id.clone());

                recommendations.push(RecommendedProblem {
                    problem_id: sim.problem_id,
                    based_on: solved.id.clone(),
                    reason: format!("Похожа на решённую задачу {}", solved.number),
                    similarity: sim.similarity,
                });
            }
        }

        // Sort by similarity and return top N
        recommendations.sort_by(|a, b| b.similarity.partial_cmp(&a.similarity).unwrap());
        recommendations.truncate(count);
        recommendations
    }

    /// Recommend next problems based on difficulty progression
    pub fn recommend_progression(
        &self,
        last_solved: &Problem,
        all_problems: &[Problem],
        count: usize,
    ) -> Vec<RecommendedProblem> {
        let current_difficulty = last_solved.difficulty.unwrap_or(5);
        
        // Find problems with slightly higher difficulty
        let target_difficulty = (current_difficulty + 1).min(10);
        
        let candidates: Vec<&Problem> = all_problems
            .iter()
            .filter(|p| {
                p.difficulty.map(|d| d == target_difficulty).unwrap_or(false)
                    && p.id != last_solved.id
            })
            .collect();

        candidates
            .into_iter()
            .take(count)
            .map(|p| RecommendedProblem {
                problem_id: p.id.clone(),
                based_on: last_solved.id.clone(),
                reason: format!("Следующий уровень сложности ({}/10)", target_difficulty),
                similarity: 0.5,
            })
            .collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecommendedProblem {
    pub problem_id: String,
    pub based_on: String,
    pub reason: String,
    pub similarity: f64,
}

impl Default for ProblemRecommender {
    fn default() -> Self {
        Self::new()
    }
}
