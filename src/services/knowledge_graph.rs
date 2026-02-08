use std::collections::{HashMap, HashSet};
use serde::{Deserialize, Serialize};
use regex::Regex;
use crate::models::Problem;

/// Knowledge Graph - graph of interconnected math concepts
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeGraph {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub clusters: Vec<Cluster>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    pub label: String,
    pub node_type: NodeType,
    pub difficulty: Option<u8>,
    pub problem_count: u32,
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub size: f64,
    pub color: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeType {
    Chapter,
    Topic,
    Concept,
    Formula,
    Problem,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub id: String,
    pub source: String,
    pub target: String,
    pub edge_type: EdgeType,
    pub weight: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeType {
    Contains,
    Requires,
    Related,
    Similar,
    LeadsTo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cluster {
    pub id: String,
    pub label: String,
    pub node_ids: Vec<String>,
    pub color: String,
}

/// Graph builder
pub struct KnowledgeGraphBuilder {
    nodes: HashMap<String, Node>,
    edges: Vec<Edge>,
    concept_extractor: ConceptExtractor,
}

impl KnowledgeGraphBuilder {
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            edges: Vec::new(),
            concept_extractor: ConceptExtractor::new(),
        }
    }

    /// Add chapter as a node
    pub fn add_chapter(&mut self, chapter_id: &str, title: &str, problem_count: u32) {
        let node = Node {
            id: chapter_id.to_string(),
            label: title.to_string(),
            node_type: NodeType::Chapter,
            difficulty: None,
            problem_count,
            x: None,
            y: None,
            size: 30.0 + problem_count as f64 * 0.5,
            color: "#4a90d9".to_string(),
        };
        self.nodes.insert(chapter_id.to_string(), node);
    }

    /// Add problem and extract concepts
    pub fn add_problem(&mut self, problem: &Problem) {
        let problem_node_id = format!("problem:{}", problem.id);
        
        // Create problem node
        let problem_node = Node {
            id: problem_node_id.clone(),
            label: format!("Задача {}", problem.number),
            node_type: NodeType::Problem,
            difficulty: problem.difficulty,
            problem_count: 1,
            x: None,
            y: None,
            size: 15.0 + problem.difficulty.unwrap_or(5) as f64,
            color: "#58a6ff".to_string(),
        };
        self.nodes.insert(problem_node_id.clone(), problem_node);

        // Link to chapter
        if let Some(chapter_node_id) = self.find_chapter_for_problem(problem) {
            self.edges.push(Edge {
                id: format!("{}->{}", chapter_node_id, problem_node_id),
                source: chapter_node_id,
                target: problem_node_id.clone(),
                edge_type: EdgeType::Contains,
                weight: 1.0,
            });
        }

        // Extract and add concepts
        let concepts = self.concept_extractor.extract_concepts(&problem.content);
        
        for concept in concepts {
            let concept_id = format!("concept:{}", concept.to_lowercase().replace(" ", "_"));
            
            // Add concept node if not exists
            if !self.nodes.contains_key(&concept_id) {
                let concept_node = Node {
                    id: concept_id.clone(),
                    label: concept,
                    node_type: NodeType::Concept,
                    difficulty: None,
                    problem_count: 0,
                    x: None,
                    y: None,
                    size: 20.0,
                    color: "#238636".to_string(),
                };
                self.nodes.insert(concept_id.clone(), concept_node);
            }

            // Link problem to concept
            self.edges.push(Edge {
                id: format!("{}->{}", problem_node_id, concept_id),
                source: problem_node_id.clone(),
                target: concept_id,
                edge_type: EdgeType::Related,
                weight: 0.7,
            });
        }

        // Extract formulas
        for formula in &problem.latex_formulas {
            let formula_id = format!("formula:{}", Self::hash_formula(formula));
            
            if !self.nodes.contains_key(&formula_id) {
                let formula_node = Node {
                    id: formula_id.clone(),
                    label: format!("${}$", &formula[..formula.len().min(20)]),
                    node_type: NodeType::Formula,
                    difficulty: None,
                    problem_count: 0,
                    x: None,
                    y: None,
                    size: 10.0,
                    color: "#d29922".to_string(),
                };
                self.nodes.insert(formula_id.clone(), formula_node);
            }

            self.edges.push(Edge {
                id: format!("{}->{}", problem_node_id, formula_id),
                source: problem_node_id.clone(),
                target: formula_id,
                edge_type: EdgeType::Contains,
                weight: 0.9,
            });
        }
    }

    /// Build similarity edges between problems
    pub fn build_similarity_edges(&mut self, threshold: f64) {
        let problem_nodes: Vec<String> = self.nodes.values()
            .filter(|n| matches!(n.node_type, NodeType::Problem))
            .map(|n| n.id.clone())
            .collect();

        for i in 0..problem_nodes.len() {
            for j in (i + 1)..problem_nodes.len() {
                let id1 = &problem_nodes[i];
                let id2 = &problem_nodes[j];

                let similarity = self.calculate_similarity(id1, id2);
                
                if similarity >= threshold {
                    self.edges.push(Edge {
                        id: format!("sim:{}:{}", id1, id2),
                        source: id1.clone(),
                        target: id2.clone(),
                        edge_type: EdgeType::Similar,
                        weight: similarity,
                    });
                }
            }
        }
    }

    /// Calculate similarity between two problems
    fn calculate_similarity(&self, id1: &str, id2: &str) -> f64 {
        let concepts1 = self.get_concepts_for_problem(id1);
        let concepts2 = self.get_concepts_for_problem(id2);

        if concepts1.is_empty() || concepts2.is_empty() {
            return 0.0;
        }

        let intersection: HashSet<_> = concepts1.intersection(&concepts2).collect();
        let union: HashSet<_> = concepts1.union(&concepts2).collect();

        intersection.len() as f64 / union.len() as f64
    }

    fn get_concepts_for_problem(&self, problem_id: &str) -> HashSet<String> {
        self.edges.iter()
            .filter(|e| e.source == problem_id && matches!(e.edge_type, EdgeType::Related))
            .map(|e| e.target.clone())
            .collect()
    }

    /// Apply force-directed layout
    pub fn apply_layout(&mut self, iterations: usize) {
        let width = 1000.0;
        let height = 800.0;
        let mut rng = rand::thread_rng();
        // Initialize random positions
        for node in self.nodes.values_mut() {
            if node.x.is_none() {
                node.x = Some(rand::random::<f64>() * width);
                node.y = Some(rand::random::<f64>() * height);
            }
        }

        // Force-directed iterations
        for _ in 0..iterations {
            let mut forces: HashMap<String, (f64, f64)> = HashMap::new();

            // Repulsive forces between all nodes
            let node_ids: Vec<String> = self.nodes.keys().cloned().collect();
            for i in 0..node_ids.len() {
                for j in (i + 1)..node_ids.len() {
                    let id1 = &node_ids[i];
                    let id2 = &node_ids[j];

                    let n1 = self.nodes.get(id1).unwrap();
                    let n2 = self.nodes.get(id2).unwrap();

                    let dx = n2.x.unwrap() - n1.x.unwrap();
                    let dy = n2.y.unwrap() - n1.y.unwrap();
                    let dist_sq = dx * dx + dy * dy + 0.1;
                    let dist = dist_sq.sqrt();

                    let force = 1000.0 / dist_sq;
                    let fx = (dx / dist) * force;
                    let fy = (dy / dist) * force;

                    forces.entry(id1.clone())
                        .and_modify(|(x, y)| { *x -= fx; *y -= fy; })
                        .or_insert((-fx, -fy));

                    forces.entry(id2.clone())
                        .and_modify(|(x, y)| { *x += fx; *y += fy; })
                        .or_insert((fx, fy));
                }
            }

            // Attractive forces along edges
            for edge in &self.edges {
                let n1 = self.nodes.get(&edge.source).unwrap();
                let n2 = self.nodes.get(&edge.target).unwrap();

                let dx = n2.x.unwrap() - n1.x.unwrap();
                let dy = n2.y.unwrap() - n1.y.unwrap();
                let dist = (dx * dx + dy * dy).sqrt() + 0.1;

                let target_dist = 100.0 / edge.weight;
                let force = (dist - target_dist) * 0.01;

                let fx = (dx / dist) * force;
                let fy = (dy / dist) * force;

                forces.entry(edge.source.clone())
                    .and_modify(|(x, y)| { *x += fx; *y += fy; })
                    .or_insert((fx, fy));

                forces.entry(edge.target.clone())
                    .and_modify(|(x, y)| { *x -= fx; *y -= fy; })
                    .or_insert((-fx, -fy));
            }

            // Apply forces
            for (id, (fx, fy)) in forces {
                if let Some(node) = self.nodes.get_mut(&id) {
                    let x = (node.x.unwrap() + fx * 0.1).clamp(50.0, width - 50.0);
                    let y = (node.y.unwrap() + fy * 0.1).clamp(50.0, height - 50.0);
                    node.x = Some(x);
                    node.y = Some(y);
                }
            }
        }
    }

    /// Detect clusters using simple connected components
    pub fn detect_clusters(&self) -> Vec<Cluster> {
        let mut visited: HashSet<String> = HashSet::new();
        let mut clusters = Vec::new();
        let mut cluster_id = 0;

        for node_id in self.nodes.keys() {
            if visited.contains(node_id) {
                continue;
            }

            let mut component = Vec::new();
            let mut stack = vec![node_id.clone()];

            while let Some(current) = stack.pop() {
                if visited.contains(&current) {
                    continue;
                }
                visited.insert(current.clone());
                component.push(current.clone());

                // Find neighbors
                for edge in &self.edges {
                    if edge.source == current && !visited.contains(&edge.target) {
                        stack.push(edge.target.clone());
                    } else if edge.target == current && !visited.contains(&edge.source) {
                        stack.push(edge.source.clone());
                    }
                }
            }

            if !component.is_empty() {
                let colors = vec!["#ff6b6b", "#4ecdc4", "#45b7d1", "#96ceb4", "#ffeaa7", "#dfe6e9"];
                clusters.push(Cluster {
                    id: format!("cluster_{}", cluster_id),
                    label: format!("Topic {}", cluster_id + 1),
                    node_ids: component,
                    color: colors[cluster_id % colors.len()].to_string(),
                });
                cluster_id += 1;
            }
        }

        clusters
    }

    /// Build the final graph
    pub fn build(mut self) -> KnowledgeGraph {
        self.apply_layout(100);
        let clusters = self.detect_clusters();

        KnowledgeGraph {
            nodes: self.nodes.into_values().collect(),
            edges: self.edges,
            clusters,
        }
    }

    fn find_chapter_for_problem(&self, problem: &Problem) -> Option<String> {
        // Extract chapter ID from problem ID (format: book:chapter:number)
        let parts: Vec<_> = problem.id.split(':').collect();
        if parts.len() >= 2 {
            Some(format!("{}:{}", parts[0], parts[1]))
        } else {
            None
        }
    }

    fn hash_formula(formula: &str) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        
        let mut hasher = DefaultHasher::new();
        formula.hash(&mut hasher);
        format!("{:x}", hasher.finish())[..8].to_string()
    }
}

/// Extract mathematical concepts from text
pub struct ConceptExtractor {
    concept_patterns: Vec<(String, Regex)>,
}

impl ConceptExtractor {
    pub fn new() -> Self {
        let patterns = vec![
            ("уравнение", r"уравнение"),
            ("неравенство", r"неравенство"),
            ("функция", r"функци[ия]"),
            ("производная", r"производная"),
            ("интеграл", r"интеграл"),
            ("предел", r"предел"),
            ("ряд", r"ряд"),
            ("многочлен", r"многочлен"),
            ("корень", r"корень"),
            ("степень", r"степень"),
            ("логарифм", r"логарифм"),
            ("тригонометрия", r"(?:sin|cos|tg|ctg|sinus|cosinus)"),
            ("вектор", r"вектор"),
            ("матрица", r"матриц[аы]"),
            ("определитель", r"определитель"),
            ("прямая", r"прямая"),
            ("окружность", r"окружность"),
            ("парабола", r"парабола"),
            ("эллипс", r"эллипс"),
            ("гипербола", r"гипербола"),
            ("треугольник", r"треугольник"),
            ("четырёхугольник", r"четырёхугольник"),
            ("многоугольник", r"многоугольник"),
            ("площадь", r"площадь"),
            ("периметр", r"периметр"),
            ("объём", r"объём"),
            ("доказательство", r"докажите|доказать"),
            ("квадратное уравнение", r"квадратное\s+уравнение"),
            ("система уравнений", r"система\s+уравнений"),
            ("арифметическая прогрессия", r"арифметическая\s+прогрессия"),
            ("геометрическая прогрессия", r"геометрическая\s+прогрессия"),
        ];

        let concept_patterns: Vec<_> = patterns.into_iter()
            .filter_map(|(name, pattern)| {
                Regex::new(&format!(r"(?i)\b{}", pattern)).ok()
                    .map(|r| (name.to_string(), r))
            })
            .collect();

        Self { concept_patterns }
    }

    pub fn extract_concepts(&self, text: &str) -> Vec<String> {
        let mut concepts = Vec::new();

        for (name, pattern) in &self.concept_patterns {
            if pattern.is_match(text) && !concepts.contains(name) {
                concepts.push(name.clone());
            }
        }

        concepts
    }
}

impl Default for KnowledgeGraphBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl Default for ConceptExtractor {
    fn default() -> Self {
        Self::new()
    }
}
