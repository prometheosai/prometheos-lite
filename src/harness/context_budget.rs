//! P1-Issue3: Context budget enforcement by section
//!
//! This module provides comprehensive context budget management to ensure
//! that the context provided to LLMs stays within reasonable limits while
//! maintaining the most relevant information across different sections.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// P1-Issue3: Context budget enforcement by section
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextBudget {
    /// Total budget in tokens
    pub total_budget: usize,
    /// Section-specific budgets
    pub sections: HashMap<ContextSection, SectionBudget>,
    /// Current usage tracking
    pub current_usage: HashMap<ContextSection, SectionUsage>,
    /// Budget enforcement policy
    pub policy: BudgetPolicy,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum ContextSection {
    /// Source code files
    Files,
    /// Symbol definitions and signatures
    Symbols,
    /// Dependency information
    Dependencies,
    /// Test files and test coverage
    Tests,
    /// Documentation and comments
    Documentation,
    /// Build configuration
    BuildConfig,
    /// Error messages and logs
    Errors,
    /// Import/export relationships
    Imports,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SectionBudget {
    /// Maximum tokens for this section
    pub max_tokens: usize,
    /// Minimum tokens required for this section
    pub min_tokens: usize,
    /// Priority for budget allocation (higher = more important)
    pub priority: u8,
    /// Whether this section can be truncated
    pub can_truncate: bool,
    /// Whether this section can be completely omitted
    pub can_omit: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SectionUsage {
    /// Current tokens used
    pub tokens_used: usize,
    /// Items included in this section
    pub items_included: Vec<String>,
    /// Items excluded due to budget
    pub items_excluded: Vec<String>,
    /// Truncation applied
    pub truncated: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum BudgetPolicy {
    /// Strict enforcement - never exceed budget
    Strict,
    /// Soft enforcement - can exceed with warning
    Soft,
    /// Adaptive - adjust based on content importance
    Adaptive,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BudgetEnforcementResult {
    /// Final context after budget enforcement
    pub final_context: String,
    /// Total tokens used
    pub total_tokens_used: usize,
    /// Section usage details
    pub section_usage: HashMap<ContextSection, SectionUsage>,
    /// Budget violations (if any)
    pub violations: Vec<BudgetViolation>,
    /// Recommendations for budget optimization
    pub recommendations: Vec<BudgetRecommendation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BudgetViolation {
    /// Section that violated budget
    pub section: ContextSection,
    /// Type of violation
    pub violation_type: ViolationType,
    /// Amount over budget
    pub over_amount: usize,
    /// Description of the violation
    pub description: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ViolationType {
    /// Section exceeded maximum tokens
    ExceededMaximum,
    /// Section fell below minimum tokens
    BelowMinimum,
    /// Total budget exceeded
    TotalExceeded,
    /// Critical content was truncated
    CriticalTruncated,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BudgetRecommendation {
    /// Type of recommendation
    pub recommendation_type: RecommendationType,
    /// Section this applies to
    pub section: ContextSection,
    /// Description of the recommendation
    pub description: String,
    /// Estimated impact on token usage
    pub token_impact: isize,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum RecommendationType {
    /// Increase section budget
    IncreaseBudget,
    /// Decrease section budget
    DecreaseBudget,
    /// Prioritize different content
    ReprioritizeContent,
    /// Use more efficient representation
    OptimizeRepresentation,
    /// Split into multiple requests
    SplitRequest,
}

impl ContextBudget {
    /// Create a new context budget with default settings
    pub fn new(total_budget: usize) -> Self {
        let mut sections = HashMap::new();
        
        // Default section budgets
        sections.insert(ContextSection::Files, SectionBudget {
            max_tokens: (total_budget as f32 * 0.4) as usize, // 40% for files
            min_tokens: (total_budget as f32 * 0.2) as usize,  // 20% minimum
            priority: 100, // Highest priority
            can_truncate: true,
            can_omit: false,
        });
        
        sections.insert(ContextSection::Symbols, SectionBudget {
            max_tokens: (total_budget as f32 * 0.2) as usize, // 20% for symbols
            min_tokens: (total_budget as f32 * 0.1) as usize,  // 10% minimum
            priority: 90,
            can_truncate: true,
            can_omit: false,
        });
        
        sections.insert(ContextSection::Dependencies, SectionBudget {
            max_tokens: (total_budget as f32 * 0.15) as usize, // 15% for dependencies
            min_tokens: (total_budget as f32 * 0.05) as usize, // 5% minimum
            priority: 70,
            can_truncate: true,
            can_omit: true,
        });
        
        sections.insert(ContextSection::Tests, SectionBudget {
            max_tokens: (total_budget as f32 * 0.1) as usize, // 10% for tests
            min_tokens: 0, // Optional
            priority: 60,
            can_truncate: true,
            can_omit: true,
        });
        
        sections.insert(ContextSection::Documentation, SectionBudget {
            max_tokens: (total_budget as f32 * 0.1) as usize, // 10% for documentation
            min_tokens: 0, // Optional
            priority: 50,
            can_truncate: true,
            can_omit: true,
        });
        
        sections.insert(ContextSection::BuildConfig, SectionBudget {
            max_tokens: (total_budget as f32 * 0.05) as usize, // 5% for build config
            min_tokens: 0, // Optional
            priority: 40,
            can_truncate: true,
            can_omit: true,
        });
        
        sections.insert(ContextSection::Errors, SectionBudget {
            max_tokens: (total_budget as f32 * 0.05) as usize, // 5% for errors
            min_tokens: 0, // Optional
            priority: 80, // High priority for debugging
            can_truncate: true,
            can_omit: true,
        });
        
        sections.insert(ContextSection::Imports, SectionBudget {
            max_tokens: (total_budget as f32 * 0.05) as usize, // 5% for imports
            min_tokens: 0, // Optional
            priority: 30,
            can_truncate: true,
            can_omit: true,
        });
        
        Self {
            total_budget,
            sections,
            current_usage: HashMap::new(),
            policy: BudgetPolicy::Adaptive,
        }
    }
    
    /// Create a context budget with custom policy
    pub fn with_policy(total_budget: usize, policy: BudgetPolicy) -> Self {
        let mut budget = Self::new(total_budget);
        budget.policy = policy;
        budget
    }
    
    /// Enforce budget on provided context sections
    pub fn enforce_budget(
        &mut self,
        context_sections: HashMap<ContextSection, String>,
    ) -> Result<BudgetEnforcementResult> {
        let mut final_context = String::new();
        let mut total_tokens_used = 0;
        let mut section_usage = HashMap::new();
        let mut violations = Vec::new();
        let mut recommendations = Vec::new();
        
        // Sort sections by priority (highest first)
        let mut sorted_sections: Vec<_> = context_sections.into_iter()
            .collect();
        sorted_sections.sort_by(|a, b| {
            let priority_a = self.sections.get(&a.0).map(|s| s.priority).unwrap_or(0);
            let priority_b = self.sections.get(&b.0).map(|s| s.priority).unwrap_or(0);
            priority_b.cmp(&priority_a) // Reverse order (highest priority first)
        });
        
        let mut remaining_budget = self.total_budget;
        
        for (section, content) in sorted_sections {
            let section_budget = self.sections.get(&section).cloned()
                .unwrap_or_else(|| SectionBudget {
                    max_tokens: remaining_budget,
                    min_tokens: 0,
                    priority: 50,
                    can_truncate: true,
                    can_omit: true,
                });
            
            // Calculate content tokens (rough estimation)
            let content_tokens = self.estimate_tokens(&content);
            
            let (processed_content, tokens_used, usage) = if content_tokens > section_budget.max_tokens {
                // Need to truncate or omit
                if section_budget.can_truncate {
                    let truncated_content = self.truncate_content(&content, section_budget.max_tokens);
                    let truncated_tokens = self.estimate_tokens(&truncated_content);
                    
                    let mut usage = SectionUsage {
                        tokens_used: truncated_tokens,
                        items_included: vec![format!("truncated_{}", section as u8)],
                        items_excluded: vec![format!("excluded_{}", section as u8)],
                        truncated: true,
                    };
                    
                    // Check for violations
                    if truncated_tokens < section_budget.min_tokens && !section_budget.can_omit {
                        violations.push(BudgetViolation {
                            section,
                            violation_type: ViolationType::BelowMinimum,
                            over_amount: section_budget.min_tokens.saturating_sub(truncated_tokens),
                            description: format!(
                                "Section {:?} has {} tokens below minimum of {}",
                                section, truncated_tokens, section_budget.min_tokens
                            ),
                        });
                    }
                    
                    if truncated_tokens > section_budget.max_tokens {
                        violations.push(BudgetViolation {
                            section,
                            violation_type: ViolationType::ExceededMaximum,
                            over_amount: truncated_tokens.saturating_sub(section_budget.max_tokens),
                            description: format!(
                                "Section {:?} has {} tokens exceeding maximum of {}",
                                section, truncated_tokens, section_budget.max_tokens
                            ),
                        });
                    }
                    
                    (truncated_content, truncated_tokens, usage)
                } else if section_budget.can_omit {
                    // Omit the entire section
                    let usage = SectionUsage {
                        tokens_used: 0,
                        items_included: vec![],
                        items_excluded: vec![format!("omitted_{}", section as u8)],
                        truncated: false,
                    };
                    
                    if section_budget.min_tokens > 0 {
                        violations.push(BudgetViolation {
                            section,
                            violation_type: ViolationType::BelowMinimum,
                            over_amount: section_budget.min_tokens,
                            description: format!(
                                "Section {:?} was omitted but requires minimum {} tokens",
                                section, section_budget.min_tokens
                            ),
                        });
                    }
                    
                    (String::new(), 0, usage)
                } else {
                    // Cannot truncate or omit, include as-is
                    violations.push(BudgetViolation {
                        section,
                        violation_type: ViolationType::ExceededMaximum,
                        over_amount: content_tokens.saturating_sub(section_budget.max_tokens),
                        description: format!(
                            "Section {:?} exceeds budget and cannot be truncated",
                            section
                        ),
                    });
                    
                    let usage = SectionUsage {
                        tokens_used: content_tokens,
                        items_included: vec![format!("full_{}", section as u8)],
                        items_excluded: vec![],
                        truncated: false,
                    };
                    
                    (content, content_tokens, usage)
                }
            } else {
                // Content fits within budget
                let usage = SectionUsage {
                    tokens_used: content_tokens,
                    items_included: vec![format!("full_{}", section as u8)],
                    items_excluded: vec![],
                    truncated: false,
                };
                
                (content, content_tokens, usage)
            };
            
            // Update usage tracking
            section_usage.insert(section, usage);
            total_tokens_used += tokens_used;
            remaining_budget = remaining_budget.saturating_sub(tokens_used);
            
            // Add to final context
            if !processed_content.is_empty() {
                if !final_context.is_empty() {
                    final_context.push_str("\n\n");
                }
                final_context.push_str(&processed_content);
            }
        }
        
        // Check total budget violations
        if total_tokens_used > self.total_budget {
            violations.push(BudgetViolation {
                section: ContextSection::Files, // Representative section
                violation_type: ViolationType::TotalExceeded,
                over_amount: total_tokens_used.saturating_sub(self.total_budget),
                description: format!(
                    "Total tokens used {} exceeds budget of {}",
                    total_tokens_used, self.total_budget
                ),
            });
        }
        
        // Generate recommendations
        recommendations = self.generate_recommendations(&section_usage, &violations);
        
        // Update current usage
        self.current_usage = section_usage.clone();
        
        Ok(BudgetEnforcementResult {
            final_context,
            total_tokens_used,
            section_usage,
            violations,
            recommendations,
        })
    }
    
    /// Estimate token count for content (rough approximation)
    fn estimate_tokens(&self, content: &str) -> usize {
        // Rough estimation: ~4 characters per token for code
        (content.len() / 4) + 1
    }
    
    /// Truncate content to fit within token budget
    fn truncate_content(&self, content: &str, max_tokens: usize) -> String {
        let target_chars = max_tokens * 4; // Rough conversion back to characters
        
        if content.len() <= target_chars {
            return content.to_string();
        }
        
        // Try to truncate at logical boundaries
        let truncated = &content[..target_chars];
        
        // Find the last complete line
        if let Some(last_newline) = truncated.rfind('\n') {
            truncated[..last_newline].to_string()
        } else {
            truncated.to_string()
        }
    }
    
    /// Generate budget optimization recommendations
    fn generate_recommendations(
        &self,
        section_usage: &HashMap<ContextSection, SectionUsage>,
        violations: &[BudgetViolation],
    ) -> Vec<BudgetRecommendation> {
        let mut recommendations = Vec::new();
        
        // Analyze usage patterns
        for (section, usage) in section_usage {
            let section_budget = self.sections.get(section);
            
            if let Some(budget) = section_budget {
                // Check if section is underutilized
                if usage.tokens_used < budget.min_tokens && budget.can_omit {
                    recommendations.push(BudgetRecommendation {
                        recommendation_type: RecommendationType::DecreaseBudget,
                        section: *section,
                        description: format!(
                            "Section {:?} is underutilized ({} tokens vs {} min)",
                            section, usage.tokens_used, budget.min_tokens
                        ),
                        token_impact: -(budget.min_tokens as isize - usage.tokens_used as isize),
                    });
                }
                
                // Check if section is consistently truncated
                if usage.truncated {
                    recommendations.push(BudgetRecommendation {
                        recommendation_type: RecommendationType::IncreaseBudget,
                        section: *section,
                        description: format!(
                            "Section {:?} is frequently truncated",
                            section
                        ),
                        token_impact: budget.max_tokens as isize - usage.tokens_used as isize,
                    });
                }
            }
        }
        
        // Check for specific violations
        for violation in violations {
            match violation.violation_type {
                ViolationType::TotalExceeded => {
                    recommendations.push(BudgetRecommendation {
                        recommendation_type: RecommendationType::SplitRequest,
                        section: ContextSection::Files,
                        description: "Consider splitting into multiple requests".to_string(),
                        token_impact: -violation.over_amount as isize,
                    });
                }
                ViolationType::CriticalTruncated => {
                    recommendations.push(BudgetRecommendation {
                        recommendation_type: RecommendationType::ReprioritizeContent,
                        section: violation.section,
                        description: "Critical content was truncated, consider reprioritization".to_string(),
                        token_impact: 0,
                    });
                }
                _ => {}
            }
        }
        
        recommendations
    }
    
    /// Get current budget usage statistics
    pub fn get_usage_stats(&self) -> HashMap<ContextSection, f32> {
        let mut stats = HashMap::new();
        
        for (section, usage) in &self.current_usage {
            let section_budget = self.sections.get(section);
            
            if let Some(budget) = section_budget {
                let usage_percentage = if budget.max_tokens > 0 {
                    (usage.tokens_used as f32 / budget.max_tokens as f32) * 100.0
                } else {
                    0.0
                };
                
                stats.insert(*section, usage_percentage);
            }
        }
        
        stats
    }
    
    /// Adjust section budgets based on usage patterns
    pub fn adjust_budgets(&mut self, usage_history: &[HashMap<ContextSection, SectionUsage>]) {
        if usage_history.is_empty() {
            return;
        }
        
        // Calculate average usage for each section
        let mut avg_usage = HashMap::new();
        
        for usage in usage_history {
            for (section, section_usage) in usage {
                let entry = avg_usage.entry(*section).or_insert(0);
                *entry += section_usage.tokens_used;
            }
        }
        
        // Calculate averages
        for (_, total) in avg_usage.iter_mut() {
            *total /= usage_history.len();
        }
        
        // Adjust budgets based on average usage
        for (section, avg_tokens) in avg_usage {
            if let Some(budget) = self.sections.get_mut(&section) {
                // Increase budget if consistently over limit
                if avg_tokens > budget.max_tokens {
                    budget.max_tokens = ((avg_tokens as f32 * 1.2) as usize).min(budget.max_tokens * 2);
                }
                // Decrease budget if consistently under limit
                else if avg_tokens < budget.min_tokens && budget.can_omit {
                    budget.min_tokens = avg_tokens;
                }
            }
        }
    }
    
    /// Validate budget configuration
    pub fn validate_configuration(&self) -> Result<Vec<String>> {
        let mut issues = Vec::new();
        
        // Check if total budget is reasonable
        if self.total_budget < 1000 {
            issues.push("Total budget is very small (< 1000 tokens)".to_string());
        }
        
        if self.total_budget > 100000 {
            issues.push("Total budget is very large (> 100k tokens)".to_string());
        }
        
        // Check section budgets
        let mut total_max_tokens = 0;
        let mut total_min_tokens = 0;
        
        for (section, budget) in &self.sections {
            total_max_tokens += budget.max_tokens;
            total_min_tokens += budget.min_tokens;
            
            // Validate individual section budgets
            if budget.max_tokens < budget.min_tokens {
                issues.push(format!(
                    "Section {:?} has max tokens ({}) less than min tokens ({})",
                    section, budget.max_tokens, budget.min_tokens
                ));
            }
            
            if budget.max_tokens == 0 && !budget.can_omit {
                issues.push(format!(
                    "Section {:?} has zero budget but cannot be omitted",
                    section
                ));
            }
        }
        
        // Check if section budgets exceed total budget
        if total_max_tokens > self.total_budget {
            issues.push(format!(
                "Total section budgets ({}) exceed total budget ({})",
                total_max_tokens, self.total_budget
            ));
        }
        
        // Check if minimum requirements exceed total budget
        if total_min_tokens > self.total_budget {
            issues.push(format!(
                "Total minimum requirements ({}) exceed total budget ({})",
                total_min_tokens, self.total_budget
            ));
        }
        
        Ok(issues)
    }
}
