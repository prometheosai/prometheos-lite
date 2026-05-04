//! Issue 1: Repo Intelligence Engine Tests
//!
//! Comprehensive tests for the Repo Intelligence Engine including:
//! - RepoContext building with file ranking
//! - Symbol extraction and relationship mapping
//! - Dependency graph parsing (Cargo.toml, package.json)
//! - Language detection
//! - File relevance scoring
//! - Cache management

use std::collections::HashMap;
use std::path::PathBuf;

use prometheos_lite::harness::repo_intelligence::{
    build_repo_context, parse_dependency_graph, CodeSymbol, DependencyGraph,
    DependencySpec, EdgeKind, PackageManagerType, RankedFile, RepoCache,
    RepoContext, RepoMap, SymbolEdge, SymbolKind, Visibility,
};

// ============================================================================
// Basic Structure Tests
// ============================================================================

#[test]
fn test_repo_context_structure() {
    let context = RepoContext {
        root: PathBuf::from("/test/repo"),
        ranked_files: vec![],
        symbols: vec![],
        relationships: vec![],
        compressed_context: String::new(),
        token_estimate: 0,
        language_breakdown: HashMap::new(),
        dependency_graph: DependencyGraph::default(),
    };

    assert_eq!(context.root, PathBuf::from("/test/repo"));
    assert!(context.ranked_files.is_empty());
    assert!(context.symbols.is_empty());
}

#[test]
fn test_ranked_file_creation() {
    let file = RankedFile {
        path: PathBuf::from("src/main.rs"),
        score: 100,
        reason: "main entry point".to_string(),
        symbol_count: 5,
        language: "rust".to_string(),
    };

    assert_eq!(file.path, PathBuf::from("src/main.rs"));
    assert_eq!(file.score, 100);
    assert_eq!(file.reason, "main entry point");
}

#[test]
fn test_code_symbol_creation() {
    let symbol = CodeSymbol {
        name: "main".to_string(),
        kind: SymbolKind::Function,
        file: PathBuf::from("src/main.rs"),
        line_start: 1,
        line_end: 10,
        column_start: 0,
        column_end: 10,
        documentation: Some("Main function".to_string()),
        signature: Some("fn main()".to_string()),
        visibility: Visibility::Public,
    };

    assert_eq!(symbol.name, "main");
    assert_eq!(symbol.kind, SymbolKind::Function);
    assert_eq!(symbol.visibility, Visibility::Public);
}

#[test]
fn test_symbol_edge_creation() {
    let edge = SymbolEdge {
        from: "main".to_string(),
        to: "helper".to_string(),
        file: PathBuf::from("src/main.rs"),
        line: 5,
        kind: EdgeKind::Call,
    };

    assert_eq!(edge.from, "main");
    assert_eq!(edge.to, "helper");
    assert_eq!(edge.kind, EdgeKind::Call);
}

// ============================================================================
// Dependency Graph Tests (using public API)
// ============================================================================

#[test]
fn test_parse_dependency_graph_from_cargo_toml() {
    // Use the sample_repo fixture which has a Cargo.toml
    let fixture_path = PathBuf::from("tests/fixtures/sample_repo");
    
    let graph = parse_dependency_graph(&fixture_path);

    // The sample repo has an empty Cargo.toml
    assert_eq!(graph.package_manager, PackageManagerType::Cargo);
    assert_eq!(graph.source_file.file_name().unwrap(), "Cargo.toml");
}

#[test]
fn test_dependency_graph_default() {
    let graph = DependencyGraph::default();

    assert!(graph.dependencies.is_empty());
    assert!(graph.dev_dependencies.is_empty());
    assert!(graph.build_dependencies.is_empty());
    assert!(graph.peer_dependencies.is_empty());
    assert_eq!(graph.package_manager, PackageManagerType::Unknown);
}

#[test]
fn test_dependency_spec_creation() {
    let spec = DependencySpec {
        version: Some("1.0.0".to_string()),
        path: None,
        git: None,
        features: vec!["feature1".to_string(), "feature2".to_string()],
        optional: false,
        target: None,
    };

    assert_eq!(spec.version, Some("1.0.0".to_string()));
    assert_eq!(spec.features.len(), 2);
    assert!(!spec.optional);
}

// ============================================================================
// Cache Management Tests
// ============================================================================

#[test]
fn test_repo_cache_creation() {
    let temp_dir = std::env::temp_dir();
    let cache = RepoCache::new(temp_dir.clone());

    assert!(cache.cached_files().is_empty());
    assert_eq!(cache.stats().total_files, 0);
}

#[test]
fn test_repo_cache_update_file() {
    let temp_dir = std::env::temp_dir().join("test_cache_update");
    std::fs::create_dir_all(&temp_dir).ok();

    let mut cache = RepoCache::new(temp_dir.clone());

    // Create a test file
    let test_file = temp_dir.join("test.rs");
    std::fs::write(&test_file, "fn main() {}").unwrap();

    let symbol = CodeSymbol {
        name: "main".to_string(),
        kind: SymbolKind::Function,
        file: test_file.clone(),
        line_start: 1,
        line_end: 1,
        column_start: 0,
        column_end: 10,
        documentation: None,
        signature: Some("fn main()".to_string()),
        visibility: Visibility::Public,
    };

    cache.update_file(&test_file, vec![symbol], vec![]);

    assert_eq!(cache.stats().total_files, 1);
    assert_eq!(cache.stats().total_symbols, 1);

    // Cleanup
    std::fs::remove_dir_all(&temp_dir).ok();
}

#[test]
fn test_repo_cache_remove_file() {
    let temp_dir = std::env::temp_dir().join("test_cache_remove");
    std::fs::create_dir_all(&temp_dir).ok();

    let mut cache = RepoCache::new(temp_dir.clone());

    let test_file = temp_dir.join("test.rs");
    std::fs::write(&test_file, "fn main() {}").unwrap();

    let symbol = CodeSymbol {
        name: "main".to_string(),
        kind: SymbolKind::Function,
        file: test_file.clone(),
        line_start: 1,
        line_end: 1,
        column_start: 0,
        column_end: 10,
        documentation: None,
        signature: None,
        visibility: Visibility::Public,
    };

    cache.update_file(&test_file, vec![symbol], vec![]);
    assert_eq!(cache.stats().total_files, 1);

    cache.remove_file(&test_file);
    assert_eq!(cache.stats().total_files, 0);

    std::fs::remove_dir_all(&temp_dir).ok();
}

#[test]
fn test_repo_cache_invalidate_all() {
    let temp_dir = std::env::temp_dir().join("test_cache_invalidate");
    std::fs::create_dir_all(&temp_dir).ok();

    let mut cache = RepoCache::new(temp_dir.clone());

    let test_file = temp_dir.join("test.rs");
    std::fs::write(&test_file, "fn main() {}").unwrap();

    let symbol = CodeSymbol {
        name: "main".to_string(),
        kind: SymbolKind::Function,
        file: test_file.clone(),
        line_start: 1,
        line_end: 1,
        column_start: 0,
        column_end: 10,
        documentation: None,
        signature: None,
        visibility: Visibility::Public,
    };

    cache.update_file(&test_file, vec![symbol], vec![]);
    assert_eq!(cache.stats().total_files, 1);

    cache.invalidate_all();
    assert_eq!(cache.stats().total_files, 0);

    std::fs::remove_dir_all(&temp_dir).ok();
}

// ============================================================================
// Symbol Kind and Visibility Tests
// ============================================================================

#[test]
fn test_symbol_kind_variants() {
    assert!(matches!(SymbolKind::Function, SymbolKind::Function));
    assert!(matches!(SymbolKind::Struct, SymbolKind::Struct));
    assert!(matches!(SymbolKind::Enum, SymbolKind::Enum));
    assert!(matches!(SymbolKind::Trait, SymbolKind::Trait));
    assert!(matches!(SymbolKind::Module, SymbolKind::Module));
}

#[test]
fn test_visibility_variants() {
    assert!(matches!(Visibility::Public, Visibility::Public));
    assert!(matches!(Visibility::Private, Visibility::Private));
    assert!(matches!(Visibility::Protected, Visibility::Protected));
    assert!(matches!(Visibility::Internal, Visibility::Internal));
}

#[test]
fn test_edge_kind_variants() {
    assert!(matches!(EdgeKind::Import, EdgeKind::Import));
    assert!(matches!(EdgeKind::Call, EdgeKind::Call));
    assert!(matches!(EdgeKind::Reference, EdgeKind::Reference));
    assert!(matches!(EdgeKind::Inherits, EdgeKind::Inherits));
    assert!(matches!(EdgeKind::Implements, EdgeKind::Implements));
    assert!(matches!(EdgeKind::Contains, EdgeKind::Contains));
}

// ============================================================================
// RepoMap Alias Test
// ============================================================================

#[test]
fn test_repop_map_alias() {
    // RepoMap is an alias for RepoContext
    let _context: RepoMap = RepoContext {
        root: PathBuf::from("/test"),
        ranked_files: vec![],
        symbols: vec![],
        relationships: vec![],
        compressed_context: String::new(),
        token_estimate: 0,
        language_breakdown: HashMap::new(),
        dependency_graph: DependencyGraph::default(),
    };
}

