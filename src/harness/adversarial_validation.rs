use crate::harness::repo_intelligence::SymbolInfo;
use anyhow::Result;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdversarialTestSuite {
    pub edge_case_tests: Vec<EdgeCaseTest>,
    pub property_tests: Vec<PropertyTest>,
    pub fuzz_tests: Vec<FuzzTest>,
    pub stress_tests: Vec<StressTest>,
    pub generated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EdgeCaseTest {
    pub id: String,
    pub name: String,
    pub target_function: String,
    pub edge_case_type: EdgeCaseType,
    pub inputs: Vec<String>,
    pub expected_behavior: ExpectedBehavior,
    pub priority: TestPriority,
    pub language: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum EdgeCaseType {
    EmptyInput,
    NullInput,
    MaximumValue,
    MinimumValue,
    BoundaryValue,
    InvalidFormat,
    UnicodeEdge,
    IntegerOverflow,
    DivisionByZero,
    DeepRecursion,
    RaceCondition,
    ResourceExhaustion,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum TestPriority {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ExpectedBehavior {
    ReturnError(String),
    ReturnValue(String),
    Panic(String),
    Timeout,
    NoCrash,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PropertyTest {
    pub id: String,
    pub name: String,
    pub target_function: String,
    pub property: Property,
    pub generator: ValueGenerator,
    pub num_iterations: usize,
    pub language: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Property {
    Idempotent,
    Commutative,
    Associative,
    Inverse,
    Reflexive,
    Symmetric,
    Transitive,
    Monotonic,
    Injective,
    Surjective,
    PureFunction,
    TotalOrder,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ValueGenerator {
    pub gen_type: GeneratorType,
    pub constraints: Vec<Constraint>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum GeneratorType {
    IntegerRange { min: i64, max: i64 },
    StringPattern { pattern: String, min_len: usize, max_len: usize },
    FloatRange { min: f64, max: f64 },
    ArrayOf { element_gen: Box<GeneratorType>, min_len: usize, max_len: usize },
    OptionOf { inner: Box<GeneratorType> },
    OneOf { variants: Vec<String> },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Constraint {
    NonEmpty,
    Unique,
    Sorted,
    AsciiOnly,
    Positive,
    Negative,
    NonZero,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FuzzTest {
    pub id: String,
    pub target: String,
    pub duration_seconds: u64,
    pub seed: u64,
    pub corpus_inputs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StressTest {
    pub id: String,
    pub target: String,
    pub load_pattern: LoadPattern,
    pub duration_seconds: u64,
    pub concurrency: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum LoadPattern {
    Constant { rps: u32 },
    RampUp { start_rps: u32, end_rps: u32 },
    Spike { base_rps: u32, spike_rps: u32, spike_duration_secs: u64 },
    Random { min_rps: u32, max_rps: u32 },
}

#[derive(Debug, Clone)]
pub struct AdversarialTestGenerator {
    edge_case_patterns: HashMap<EdgeCaseType, Vec<String>>,
}

impl Default for AdversarialTestGenerator {
    fn default() -> Self {
        Self::new()
    }
}

impl AdversarialTestGenerator {
    pub fn new() -> Self {
        let mut edge_case_patterns = HashMap::new();

        edge_case_patterns.insert(EdgeCaseType::EmptyInput, vec![
            r"vec!\[\]".to_string(),
            r#""""#.to_string(),
            r"None".to_string(),
            r"0".to_string(),
        ]);

        edge_case_patterns.insert(EdgeCaseType::MaximumValue, vec![
            r"i64::MAX".to_string(),
            r"usize::MAX".to_string(),
            r"f64::MAX".to_string(),
        ]);

        edge_case_patterns.insert(EdgeCaseType::MinimumValue, vec![
            r"i64::MIN".to_string(),
            r"isize::MIN".to_string(),
            r"f64::MIN".to_string(),
        ]);

        edge_case_patterns.insert(EdgeCaseType::UnicodeEdge, vec![
            r#""\u{0000}""#.to_string(),
            r#""\u{FFFF}""#.to_string(),
            r#""\u{1F600}""#.to_string(),
            r#""日本語""#.to_string(),
            r#""𐍈""#.to_string(),
        ]);

        edge_case_patterns.insert(EdgeCaseType::InvalidFormat, vec![
            r#""not-a-number""#.to_string(),
            r#""2023-99-99""#.to_string(),
            r#""invalid@email""#.to_string(),
        ]);

        Self { edge_case_patterns }
    }

    pub fn generate_edge_case_tests(
        &self,
        function_name: &str,
        signature: &str,
        language: &str,
    ) -> Vec<EdgeCaseTest> {
        let mut tests = vec![];
        let mut id_counter = 0;

        // Parse signature to determine parameter types
        let param_types = self.parse_signature(signature);

        for (edge_case_type, patterns) in &self.edge_case_patterns {
            // Generate test for each pattern
            for pattern in patterns {
                id_counter += 1;
                let test_id = format!("edge_{}_{}", function_name, id_counter);

                let inputs = self.generate_inputs_for_type(&param_types, pattern, *edge_case_type);

                let expected = self.determine_expected_behavior(*edge_case_type, &param_types);

                tests.push(EdgeCaseTest {
                    id: test_id,
                    name: format!("{} - {:?}", function_name, edge_case_type),
                    target_function: function_name.to_string(),
                    edge_case_type: *edge_case_type,
                    inputs,
                    expected_behavior: expected,
                    priority: self.determine_priority(*edge_case_type),
                    language: language.to_string(),
                });
            }
        }

        tests
    }

    pub fn generate_property_tests(
        &self,
        function_name: &str,
        signature: &str,
        language: &str,
    ) -> Vec<PropertyTest> {
        let mut tests = vec![];
        let properties = self.infer_properties(function_name, signature);

        for (i, property) in properties.iter().enumerate() {
            let generator = self.create_property_generator(signature);

            tests.push(PropertyTest {
                id: format!("prop_{}_{}", function_name, i),
                name: format!("{} should be {:?}", function_name, property),
                target_function: function_name.to_string(),
                property: *property,
                generator,
                num_iterations: 100,
                language: language.to_string(),
            });
        }

        tests
    }

    pub fn generate_fuzz_tests(&self, target: &str, duration_secs: u64) -> Vec<FuzzTest> {
        vec![FuzzTest {
            id: format!("fuzz_{}", target),
            target: target.to_string(),
            duration_seconds: duration_secs,
            seed: 42,
            corpus_inputs: vec![
                "valid_input_1".to_string(),
                "valid_input_2".to_string(),
                "".to_string(),
            ],
        }]
    }

    pub fn generate_stress_tests(&self, target: &str, duration_secs: u64) -> Vec<StressTest> {
        vec![
            StressTest {
                id: format!("stress_{}_constant", target),
                target: target.to_string(),
                load_pattern: LoadPattern::Constant { rps: 100 },
                duration_seconds: duration_secs,
                concurrency: 10,
            },
            StressTest {
                id: format!("stress_{}_spike", target),
                target: target.to_string(),
                load_pattern: LoadPattern::Spike {
                    base_rps: 10,
                    spike_rps: 1000,
                    spike_duration_secs: 30,
                },
                duration_seconds: duration_secs,
                concurrency: 50,
            },
        ]
    }

    pub fn generate_complete_suite(
        &self,
        symbols: &[SymbolInfo],
        language: &str,
    ) -> AdversarialTestSuite {
        let mut all_edge_cases = vec![];
        let mut all_properties = vec![];
        let mut all_fuzz = vec![];
        let mut all_stress = vec![];

        for symbol in symbols {
            if symbol.kind == "function" {
                let func_name = &symbol.name;
                let signature = &symbol.signature;

                all_edge_cases.extend(self.generate_edge_case_tests(func_name, signature, language));
                all_properties.extend(self.generate_property_tests(func_name, signature, language));
                all_fuzz.extend(self.generate_fuzz_tests(func_name, 60));
                all_stress.extend(self.generate_stress_tests(func_name, 120));
            }
        }

        AdversarialTestSuite {
            edge_case_tests: all_edge_cases,
            property_tests: all_properties,
            fuzz_tests: all_fuzz,
            stress_tests: all_stress,
            generated_at: chrono::Utc::now(),
        }
    }

    fn parse_signature(&self, signature: &str) -> Vec<String> {
        // Extract parameter types from function signature
        // e.g., "fn foo(x: i32, y: String) -> bool" -> ["i32", "String"]
        let re = Regex::new(r"\((.*)\)").unwrap();
        if let Some(cap) = re.captures(signature) {
            let params = cap.get(1).map(|m| m.as_str()).unwrap_or("");
            params
                .split(',')
                .filter_map(|p| {
                    let parts: Vec<_> = p.split(':').collect();
                    parts.get(1).map(|t| t.trim().to_string())
                })
                .collect()
        } else {
            vec![]
        }
    }

    fn generate_inputs_for_type(
        &self,
        types: &[String],
        pattern: &str,
        edge_case: EdgeCaseType,
    ) -> Vec<String> {
        types
            .iter()
            .map(|t| self.adapt_pattern_to_type(pattern, t, edge_case))
            .collect()
    }

    fn adapt_pattern_to_type(&self, pattern: &str, type_name: &str, edge_case: EdgeCaseType) -> String {
        match type_name {
            "String" | "&str" => pattern.to_string(),
            "i32" | "i64" | "isize" | "u32" | "u64" | "usize" => {
                if edge_case == EdgeCaseType::MaximumValue {
                    format!("{}::MAX", type_name.split("::").last().unwrap_or(type_name))
                } else if edge_case == EdgeCaseType::MinimumValue {
                    format!("{}::MIN", type_name.split("::").last().unwrap_or(type_name))
                } else {
                    "0".to_string()
                }
            }
            "Vec<_>" | "Vec<T>" => r"vec![]".to_string(),
            "Option<_>" | "Option<T>" => r"None".to_string(),
            _ => pattern.to_string(),
        }
    }

    fn determine_expected_behavior(&self, edge_case: EdgeCaseType, _types: &[String]) -> ExpectedBehavior {
        match edge_case {
            EdgeCaseType::DivisionByZero => ExpectedBehavior::Panic("Division by zero".to_string()),
            EdgeCaseType::NullInput => ExpectedBehavior::ReturnError("Null pointer".to_string()),
            EdgeCaseType::InvalidFormat => ExpectedBehavior::ReturnError("Invalid format".to_string()),
            EdgeCaseType::IntegerOverflow => ExpectedBehavior::ReturnError("Integer overflow".to_string()),
            EdgeCaseType::ResourceExhaustion => ExpectedBehavior::ReturnError("Out of memory".to_string()),
            EdgeCaseType::DeepRecursion => ExpectedBehavior::Timeout,
            _ => ExpectedBehavior::NoCrash,
        }
    }

    fn determine_priority(&self, edge_case: EdgeCaseType) -> TestPriority {
        match edge_case {
            EdgeCaseType::DivisionByZero |
            EdgeCaseType::IntegerOverflow |
            EdgeCaseType::NullInput => TestPriority::Critical,
            EdgeCaseType::EmptyInput |
            EdgeCaseType::MaximumValue |
            EdgeCaseType::MinimumValue => TestPriority::High,
            _ => TestPriority::Medium,
        }
    }

    fn infer_properties(&self, function_name: &str, _signature: &str) -> Vec<Property> {
        let mut properties = vec![];
        let lower_name = function_name.to_lowercase();

        // Infer properties based on function name patterns
        if lower_name.contains("sort") {
            properties.push(Property::Idempotent);
            properties.push(Property::TotalOrder);
        }
        if lower_name.contains("eq") || lower_name.contains("equal") {
            properties.push(Property::Reflexive);
            properties.push(Property::Symmetric);
            properties.push(Property::Transitive);
        }
        if lower_name.contains("hash") || lower_name.contains("id") {
            properties.push(Property::Injective);
        }
        if lower_name.contains("map") || lower_name.contains("filter") {
            properties.push(Property::PureFunction);
        }
        if lower_name.contains("add") || lower_name.contains("sum") {
            properties.push(Property::Commutative);
            properties.push(Property::Associative);
        }
        if lower_name.contains("compare") || lower_name.contains("cmp") {
            properties.push(Property::TotalOrder);
        }

        // Default: pure function
        if properties.is_empty() {
            properties.push(Property::PureFunction);
        }

        properties
    }

    fn create_property_generator(&self, _signature: &str) -> ValueGenerator {
        ValueGenerator {
            gen_type: GeneratorType::IntegerRange { min: 0, max: 1000 },
            constraints: vec![Constraint::NonZero],
        }
    }
}

pub fn generate_adversarial_tests(
    symbols: &[SymbolInfo],
    language: &str,
) -> AdversarialTestSuite {
    let generator = AdversarialTestGenerator::new();
    generator.generate_complete_suite(symbols, language)
}

pub fn format_test_suite(suite: &AdversarialTestSuite) -> String {
    let mut output = String::new();

    output.push_str("Adversarial Test Suite\n");
    output.push_str("======================\n\n");

    output.push_str(&format!("Generated: {}\n", suite.generated_at));
    output.push_str(&format!("Edge case tests: {}\n", suite.edge_case_tests.len()));
    output.push_str(&format!("Property tests: {}\n", suite.property_tests.len()));
    output.push_str(&format!("Fuzz tests: {}\n", suite.fuzz_tests.len()));
    output.push_str(&format!("Stress tests: {}\n\n", suite.stress_tests.len()));

    if !suite.edge_case_tests.is_empty() {
        output.push_str("Edge Case Tests:\n");
        for test in &suite.edge_case_tests {
            output.push_str(&format!(
                "  - {}: {:?} ({:?})\n",
                test.name, test.edge_case_type, test.priority
            ));
        }
    }

    output
}

pub fn filter_tests_by_priority(suite: &AdversarialTestSuite, min_priority: TestPriority) -> AdversarialTestSuite {
    let priority_value = |p: &TestPriority| match p {
        TestPriority::Low => 1,
        TestPriority::Medium => 2,
        TestPriority::High => 3,
        TestPriority::Critical => 4,
    };

    let min_value = priority_value(&min_priority);

    AdversarialTestSuite {
        edge_case_tests: suite
            .edge_case_tests
            .iter()
            .filter(|t| priority_value(&t.priority) >= min_value)
            .cloned()
            .collect(),
        property_tests: suite.property_tests.clone(),
        fuzz_tests: suite.fuzz_tests.clone(),
        stress_tests: suite.stress_tests.clone(),
        generated_at: suite.generated_at,
    }
}
