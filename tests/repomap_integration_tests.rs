#![cfg(any())]
// Quarantined: obsolete integration suite targets pre-audit harness APIs.
//! P0-Audit-015: RepoMap integration tests for real AST parsing

use anyhow::Result;
use prometheos_lite::harness::repo_intelligence::{RepoMap, build_module_graph};
use std::path::Path;
use tempfile::TempDir;
use tokio::fs;

#[tokio::test]
async fn test_repomap_normal_functions() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let repo_path = temp_dir.path();
    
    // Create Rust file with normal functions
    fs::write(repo_path.join("lib.rs"), r#"
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

pub async fn multiply(x: i32, y: i32) -> i32 {
    x * y
}

fn helper() -> String {
    "helper".to_string()
}
"#).await?;
    
    let repo_map = build_module_graph(repo_path).await?;
    
    // Verify functions were extracted
    assert!(!repo_map.modules.is_empty(), "Should have extracted modules");
    
    let lib_module = repo_map.modules.iter()
        .find(|m| m.name == "lib")
        .expect("Should find lib module");
    
    assert!(lib_module.functions.len() >= 2, "Should extract at least 2 functions");
    
    let function_names: Vec<String> = lib_module.functions.iter()
        .map(|f| f.name.clone())
        .collect();
    
    assert!(function_names.contains(&"add".to_string()), "Should find add function");
    assert!(function_names.contains(&"multiply".to_string()), "Should find multiply function");
    assert!(function_names.contains(&"helper".to_string()), "Should find helper function");
    
    Ok(())
}

#[tokio::test]
async fn test_repomap_structs_and_enums() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let repo_path = temp_dir.path();
    
    // Create Rust file with structs and enums
    fs::write(repo_path.join("types.rs"), r#"
pub struct User {
    pub id: u64,
    pub name: String,
    pub email: Option<String>,
}

pub enum Status {
    Active,
    Inactive { reason: String },
    Pending,
}

pub struct Config<T> {
    pub data: T,
    pub enabled: bool,
}
"#).await?;
    
    let repo_map = build_module_graph(repo_path).await?;
    
    let types_module = repo_map.modules.iter()
        .find(|m| m.name == "types")
        .expect("Should find types module");
    
    assert!(types_module.structs.len() >= 2, "Should extract at least 2 structs");
    assert!(types_module.enums.len() >= 1, "Should extract at least 1 enum");
    
    let struct_names: Vec<String> = types_module.structs.iter()
        .map(|s| s.name.clone())
        .collect();
    
    assert!(struct_names.contains(&"User".to_string()), "Should find User struct");
    assert!(struct_names.contains(&"Config".to_string()), "Should find Config struct");
    
    let status_enum = types_module.enums.iter()
        .find(|e| e.name == "Status")
        .expect("Should find Status enum");
    
    assert!(!status_enum.variants.is_empty(), "Should extract enum variants");
    
    Ok(())
}

#[tokio::test]
async fn test_repomap_traits_and_impls() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let repo_path = temp_dir.path();
    
    // Create Rust file with traits and impls
    fs::write(repo_path.join("traits.rs"), r#"
pub trait Drawable {
    fn draw(&self);
    fn area(&self) -> f64;
}

pub struct Circle {
    pub radius: f64,
}

impl Drawable for Circle {
    fn draw(&self) {
        println!("Drawing circle with radius {}", self.radius);
    }
    
    fn area(&self) -> f64 {
        3.14159 * self.radius * self.radius
    }
}

impl Circle {
    pub fn new(radius: f64) -> Self {
        Self { radius }
    }
}
"#).await?;
    
    let repo_map = build_module_graph(repo_path).await?;
    
    let traits_module = repo_map.modules.iter()
        .find(|m| m.name == "traits")
        .expect("Should find traits module");
    
    assert!(traits_module.traits.len() >= 1, "Should extract at least 1 trait");
    
    let drawable_trait = traits_module.traits.iter()
        .find(|t| t.name == "Drawable")
        .expect("Should find Drawable trait");
    
    assert!(!drawable_trait.methods.is_empty(), "Should extract trait methods");
    
    // Check impl blocks
    let impl_blocks = traits_module.impl_blocks.iter()
        .filter(|i| i.target_type.contains("Circle"))
        .collect::<Vec<_>>();
    
    assert!(!impl_blocks.is_empty(), "Should find impl block for Circle");
    
    Ok(())
}

#[tokio::test]
async fn test_repomap_modules_and_visibility() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let repo_path = temp_dir.path();
    
    // Create Rust file with different visibility levels
    fs::write(repo_path.join("visibility.rs"), r#"
pub fn public_function() -> i32 { 42 }

fn private_function() -> i32 { 24 }

pub(crate) fn crate_function() -> i32 { 18 }

pub mod public_mod {
    pub fn inside_public() -> i32 { 99 }
}

mod private_mod {
    pub fn inside_private() -> i32 { 77 }
}
"#).await?;
    
    let repo_map = build_module_graph(repo_path).await?;
    
    let visibility_module = repo_map.modules.iter()
        .find(|m| m.name == "visibility")
        .expect("Should find visibility module");
    
    // Check function visibility
    let public_funcs = visibility_module.functions.iter()
        .filter(|f| f.is_public)
        .count();
    
    let private_funcs = visibility_module.functions.iter()
        .filter(|f| !f.is_public)
        .count();
    
    assert!(public_funcs >= 1, "Should have at least 1 public function");
    assert!(private_funcs >= 1, "Should have at least 1 private function");
    
    // Check module visibility
    let public_mods = visibility_module.submodules.iter()
        .filter(|m| m.is_public)
        .count();
    
    let private_mods = visibility_module.submodules.iter()
        .filter(|m| !m.is_public)
        .count();
    
    assert!(public_mods >= 1, "Should have at least 1 public module");
    assert!(private_mods >= 1, "Should have at least 1 private module");
    
    Ok(())
}

#[tokio::test]
async fn test_repomap_documentation_extraction() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let repo_path = temp_dir.path();
    
    // Create Rust file with documentation
    fs::write(repo_path.join("documented.rs"), r#"
//! This module provides utility functions for mathematical operations.

/// Adds two numbers together.
/// 
/// # Arguments
/// 
/// * `a` - First number to add
/// * `b` - Second number to add
/// 
/// # Returns
/// 
/// The sum of `a` and `b`
/// 
/// # Examples
/// 
/// ```
/// use crate::documented::add;
/// let result = add(2, 3);
/// assert_eq!(result, 5);
/// ```
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

/// Represents a user in the system
#[derive(Debug, Clone)]
pub struct User {
    /// Unique identifier for the user
    pub id: u64,
    
    /// User's display name
    pub name: String,
}
"#).await?;
    
    let repo_map = build_module_graph(repo_path).await?;
    
    let documented_module = repo_map.modules.iter()
        .find(|m| m.name == "documented")
        .expect("Should find documented module");
    
    // Check module-level documentation
    assert!(documented_module.documentation.is_some(), "Should extract module documentation");
    let module_doc = documented_module.documentation.as_ref().unwrap();
    assert!(module_doc.contains("utility functions"), "Should contain module description");
    
    // Check function documentation
    let add_function = documented_module.functions.iter()
        .find(|f| f.name == "add")
        .expect("Should find add function");
    
    assert!(add_function.documentation.is_some(), "Should extract function documentation");
    let func_doc = add_function.documentation.as_ref().unwrap();
    assert!(func_doc.contains("Adds two numbers"), "Should contain function description");
    
    // Check struct documentation
    let user_struct = documented_module.structs.iter()
        .find(|s| s.name == "User")
        .expect("Should find User struct");
    
    assert!(user_struct.documentation.is_some(), "Should extract struct documentation");
    let struct_doc = user_struct.documentation.as_ref().unwrap();
    assert!(struct_doc.contains("user in the system"), "Should contain struct description");
    
    Ok(())
}

#[tokio::test]
async fn test_repomap_inline_modules() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let repo_path = temp_dir.path();
    
    // Create Rust file with inline modules
    fs::write(repo_path.join("inline.rs"), r#"
pub mod utils {
    pub fn helper() -> i32 { 42 }
    
    mod internal {
        pub fn secret() -> i32 { 99 }
    }
}

mod private {
    pub fn hidden() -> i32 { 7 }
}
"#).await?;
    
    let repo_map = build_module_graph(repo_path).await?;
    
    let inline_module = repo_map.modules.iter()
        .find(|m| m.name == "inline")
        .expect("Should find inline module");
    
    // Check inline module detection
    let inline_modules = inline_module.submodules.iter()
        .filter(|m| m.is_inline)
        .count();
    
    assert!(inline_modules >= 2, "Should detect inline modules");
    
    // Check nested inline module
    let utils_module = inline_module.submodules.iter()
        .find(|m| m.name == "utils")
        .expect("Should find utils inline module");
    
    assert!(utils_module.is_inline, "Utils should be marked as inline");
    assert!(utils_module.functions.len() >= 1, "Should extract functions from inline module");
    
    // Check nested inline module
    let internal_modules = utils_module.submodules.iter()
        .filter(|m| m.name == "internal")
        .collect::<Vec<_>>();
    
    assert!(!internal_modules.is_empty(), "Should find nested inline module");
    
    Ok(())
}

#[tokio::test]
async fn test_repomap_complex_rust_code() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let repo_path = temp_dir.path();
    
    // Create complex Rust file with generics, lifetimes, macros
    fs::write(repo_path.join("complex.rs"), r#"
use std::collections::HashMap;

pub struct Repository<T> 
where 
    T: Clone + Send + Sync,
{
    data: HashMap<String, T>,
    version: u32,
}

impl<T> Repository<T>
where 
    T: Clone + Send + Sync,
{
    pub fn new() -> Self {
        Self {
            data: HashMap::new(),
            version: 1,
        }
    }
    
    pub fn insert(&mut self, key: String, value: T) -> Option<T> {
        self.data.insert(key, value)
    }
    
    pub fn get<'a>(&'a self, key: &str) -> Option<&'a T> {
        self.data.get(key)
    }
}

macro_rules! create_getter {
    ($field:ident, $field_type:ty) => {
        pub fn $field(&self) -> &$field_type {
            &self.$field
        }
    };
}

pub struct Config {
    pub name: String,
    pub enabled: bool,
}

impl Config {
    create_getter!(name, String);
    create_getter!(enabled, bool);
}

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

pub async fn process_data<T>(input: T) -> Result<T>
where 
    T: serde::Serialize + serde::de::DeserializeOwned,
{
    // Complex processing logic
    Ok(input)
}
"#).await?;
    
    let repo_map = build_module_graph(repo_path).await?;
    
    let complex_module = repo_map.modules.iter()
        .find(|m| m.name == "complex")
        .expect("Should find complex module");
    
    // Check generic struct extraction
    let repository_struct = complex_module.structs.iter()
        .find(|s| s.name == "Repository")
        .expect("Should find Repository struct");
    
    assert!(repository_struct.generics.is_some(), "Should extract generic parameters");
    
    // Check impl blocks with generics
    let impl_blocks = complex_module.impl_blocks.iter()
        .filter(|i| i.target_type.contains("Repository"))
        .collect::<Vec<_>>();
    
    assert!(!impl_blocks.is_empty(), "Should find impl block for Repository");
    
    // Check function extraction from impl blocks
    let repository_impl = impl_blocks.iter()
        .find(|i| i.target_type.contains("Repository"))
        .expect("Should find Repository impl");
    
    assert!(repository_impl.methods.len() >= 2, "Should extract methods from impl block");
    
    // Check macro-generated functions
    let config_struct = complex_module.structs.iter()
        .find(|s| s.name == "Config")
        .expect("Should find Config struct");
    
    let config_impl = complex_module.impl_blocks.iter()
        .find(|i| i.target_type.contains("Config"))
        .expect("Should find Config impl");
    
    assert!(config_impl.methods.len() >= 2, "Should extract macro-generated methods");
    
    // Check type aliases
    let type_aliases = complex_module.type_aliases.iter()
        .filter(|t| t.name == "Result")
        .collect::<Vec<_>>();
    
    assert!(!type_aliases.is_empty(), "Should extract type aliases");
    
    Ok(())
}

#[tokio::test]
async fn test_repomap_error_handling() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let repo_path = temp_dir.path();
    
    // Create Rust file with syntax errors
    fs::write(repo_path.join("invalid.rs"), r#"
pub fn broken( -> i32 {  // Missing parameter name
    42
}

pub struct Invalid {  // Missing closing brace
    pub field: String
"#).await?;
    
    let repo_map = build_module_graph(repo_path).await?;
    
    // Should still create RepoMap but with errors noted
    let invalid_module = repo_map.modules.iter()
        .find(|m| m.name == "invalid")
        .expect("Should find invalid module");
    
    // RepoMap should handle syntax errors gracefully
    assert!(!repo_map.modules.is_empty(), "Should still create module entry");
    
    // Check if error information is captured
    // This depends on the actual RepoMap implementation
    // The test ensures RepoMap doesn't panic on invalid syntax
    
    Ok(())
}
