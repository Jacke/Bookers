use crate::models::Problem;

/// Validation result
#[derive(Debug, Clone)]
pub struct ValidationResult {
    pub is_valid: bool,
    pub errors: Vec<ValidationError>,
    pub warnings: Vec<ValidationWarning>,
}

#[derive(Debug, Clone)]
pub struct ValidationError {
    pub code: String,
    pub message: String,
    pub problem_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ValidationWarning {
    pub code: String,
    pub message: String,
    pub problem_id: Option<String>,
}

impl ValidationResult {
    pub fn new() -> Self {
        Self {
            is_valid: true,
            errors: Vec::new(),
            warnings: Vec::new(),
        }
    }
    
    pub fn add_error(&mut self, code: &str, message: &str, problem_id: Option<String>) {
        self.is_valid = false;
        self.errors.push(ValidationError {
            code: code.to_string(),
            message: message.to_string(),
            problem_id,
        });
    }
    
    pub fn add_warning(&mut self, code: &str, message: &str, problem_id: Option<String>) {
        self.warnings.push(ValidationWarning {
            code: code.to_string(),
            message: message.to_string(),
            problem_id,
        });
    }
}

impl Default for ValidationResult {
    fn default() -> Self {
        Self::new()
    }
}

/// Validate LaTeX syntax
pub fn validate_latex(content: &str) -> Vec<String> {
    let mut errors = Vec::new();
    
    // Check for unclosed math delimiters
    let inline_count = content.matches('$').count();
    if inline_count % 2 != 0 {
        errors.push("Unclosed inline math delimiter ($)".to_string());
    }
    
    // Check for unclosed display math
    let display_opens = content.matches("$$").count();
    if display_opens % 2 != 0 {
        errors.push("Unclosed display math delimiter ($$)".to_string());
    }
    
    // Check for common LaTeX errors
    let unclosed_braces = content.matches('{').count().saturating_sub(content.matches('}').count());
    if unclosed_braces > 0 {
        errors.push(format!("{} unclosed braces '{{'", unclosed_braces));
    }
    
    let unclosed_brackets = content.matches('[').count().saturating_sub(content.matches(']').count());
    if unclosed_brackets > 0 {
        errors.push(format!("{} unclosed brackets '['", unclosed_brackets));
    }
    
    // Check for common misspelled commands
    let common_misspellings = vec![
        ("\\frac", vec!["\\f rac", "\\frc", "\\fract"]),
        ("\\sqrt", vec!["\\sqr", "\\squrt", "\\sqt"]),
        ("\\cdot", vec!["\\cdt", r"\cdot", "\\ct"]),
        ("\\alpha", vec![r"\alpha", "\\alfa"]),
        ("\\beta", vec![r"\beta", "\\bta"]),
        ("\\gamma", vec![r"\gamma", "\\gama"]),
    ];
    
    for (correct, misspellings) in common_misspellings {
        for misspelling in misspellings {
            if content.contains(misspelling) {
                errors.push(format!("Possible misspelling: '{}' should be '{}'", misspelling, correct));
            }
        }
    }
    
    errors
}

/// Validate problem sequence (check for gaps)
pub fn validate_problem_sequence(problems: &[Problem]) -> ValidationResult {
    let mut result = ValidationResult::new();
    
    if problems.is_empty() {
        return result;
    }
    
    // Extract numeric problem numbers
    let mut numbers: Vec<(String, u32)> = problems
        .iter()
        .filter(|p| p.parent_id.is_none()) // Only main problems, not sub-problems
        .filter_map(|p| {
            // Try to parse number as integer
            p.number.parse::<u32>().ok().map(|n| (p.id.clone(), n))
        })
        .collect();
    
    if numbers.is_empty() {
        return result;
    }
    
    // Sort by number
    numbers.sort_by_key(|(_, n)| *n);
    
    // Check for duplicates
    let mut seen = std::collections::HashSet::new();
    for (id, num) in &numbers {
        if !seen.insert(*num) {
            result.add_error(
                "DUPLICATE_NUMBER",
                &format!("Duplicate problem number: {}", num),
                Some(id.clone()),
            );
        }
    }
    
    // Check for gaps
    for window in numbers.windows(2) {
        let (_id1, num1) = &window[0];
        let (id2, num2) = &window[1];
        
        if num2 - num1 > 1 {
            let gap_start = num1 + 1;
            let gap_end = num2 - 1;
            
            if gap_start == gap_end {
                result.add_warning(
                    "MISSING_NUMBER",
                    &format!("Missing problem number: {}", gap_start),
                    Some(id2.clone()),
                );
            } else {
                result.add_warning(
                    "MISSING_RANGE",
                    &format!("Missing problem numbers: {}-{}", gap_start, gap_end),
                    Some(id2.clone()),
                );
            }
        }
    }
    
    // Check if first problem is reasonable (usually starts at 1)
    if let Some((_, first)) = numbers.first() {
        if *first > 1 && *first < 1000 {
            result.add_warning(
                "SEQUENCE_START",
                &format!("Problem sequence starts at {}, expected 1", first),
                None,
            );
        }
    }
    
    result
}

/// Validate single problem
pub fn validate_problem(problem: &Problem) -> ValidationResult {
    let mut result = ValidationResult::new();
    
    // Check empty fields
    if problem.number.trim().is_empty() {
        result.add_error("EMPTY_NUMBER", "Problem number is empty", Some(problem.id.clone()));
    }
    
    if problem.content.trim().is_empty() {
        result.add_error("EMPTY_CONTENT", "Problem content is empty", Some(problem.id.clone()));
    }
    
    if problem.display_name.trim().is_empty() {
        result.add_warning("EMPTY_DISPLAY_NAME", "Display name is empty", Some(problem.id.clone()));
    }
    
    // Validate LaTeX
    let latex_errors = validate_latex(&problem.content);
    for error in latex_errors {
        result.add_warning("LATEX_SYNTAX", &error, Some(problem.id.clone()));
    }
    
    // Check content length
    let content_len = problem.content.len();
    if content_len < 10 {
        result.add_warning(
            "SHORT_CONTENT",
            &format!("Content is very short ({} chars)", content_len),
            Some(problem.id.clone()),
        );
    }
    
    if content_len > 10000 {
        result.add_warning(
            "LONG_CONTENT",
            &format!("Content is very long ({} chars)", content_len),
            Some(problem.id.clone()),
        );
    }
    
    // Check for common OCR artifacts
    let ocr_artifacts = vec![
        ("double spaces", "  "),
        ("broken unicode", "\u{FFFD}"),
    ];
    
    for (name, pattern) in ocr_artifacts {
        if problem.content.contains(pattern) {
            result.add_warning(
                "OCR_ARTIFACT",
                &format!("Possible OCR artifact: {}", name),
                Some(problem.id.clone()),
            );
        }
    }
    
    // Validate sub-problems if present
    if let Some(subs) = &problem.sub_problems {
        let expected_letters = vec!["а", "б", "в", "г", "д", "е", "ж", "з", "и", "к"];
        
        for (i, sub) in subs.iter().enumerate() {
            if i < expected_letters.len() {
                let expected = expected_letters[i];
                if sub.number != expected {
                    result.add_warning(
                        "SUB_PROBLEM_ORDER",
                        &format!("Expected sub-problem '{}', found '{}'", expected, sub.number),
                        Some(sub.id.clone()),
                    );
                }
            }
        }
    }
    
    result
}

/// Validate batch of problems before import
pub fn validate_batch_import(problems: &[Problem], chapter_id: &str) -> ValidationResult {
    let mut result = ValidationResult::new();
    
    // Validate each problem
    for problem in problems {
        let problem_result = validate_problem(problem);
        result.errors.extend(problem_result.errors);
        result.warnings.extend(problem_result.warnings);
        if !problem_result.is_valid {
            result.is_valid = false;
        }
    }
    
    // Check for duplicates within batch
    let mut seen_numbers = std::collections::HashSet::new();
    for problem in problems {
        let key = format!("{}:{}", chapter_id, problem.number);
        if !seen_numbers.insert(key.clone()) {
            result.add_error(
                "BATCH_DUPLICATE",
                &format!("Duplicate problem number in batch: {}", problem.number),
                Some(problem.id.clone()),
            );
        }
    }
    
    // Validate sequence
    let seq_result = validate_problem_sequence(problems);
    result.errors.extend(seq_result.errors);
    result.warnings.extend(seq_result.warnings);
    
    result
}

/// Quick validation for API responses
pub fn quick_validate(content: &str) -> Option<String> {
    // Check for unclosed delimiters
    if content.matches('$').count() % 2 != 0 {
        return Some("Unclosed math delimiter".to_string());
    }
    
    // Check for empty
    if content.trim().is_empty() {
        return Some("Empty content".to_string());
    }
    
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_validate_latex() {
        let valid = "Solve $x^2 + y^2 = 1$";
        assert!(validate_latex(valid).is_empty());
        
        let unclosed = "Solve $x^2 + y^2 = 1";
        assert!(!validate_latex(unclosed).is_empty());
    }
    
    #[test]
    fn test_validate_sequence() {
        let problems = vec![
            create_test_problem("1"),
            create_test_problem("2"),
            create_test_problem("4"), // Gap: missing 3
            create_test_problem("5"),
        ];
        
        let result = validate_problem_sequence(&problems);
        assert!(!result.warnings.is_empty());
        assert!(result.warnings.iter().any(|w| w.code == "MISSING_NUMBER"));
    }
    
    fn create_test_problem(number: &str) -> Problem {
        Problem {
            id: format!("test:{}", number),
            chapter_id: "test:1".to_string(),
            page_id: None,
            parent_id: None,
            number: number.to_string(),
            display_name: format!("Problem {}", number),
            content: format!("Content of problem {}", number),
            latex_formulas: vec![],
            page_number: None,
            difficulty: None,
            has_solution: false,
            created_at: chrono::Utc::now(),
            solution: None,
            sub_problems: None,
            continues_from_page: None,
            continues_to_page: None,
            is_cross_page: false,
        }
    }
}
