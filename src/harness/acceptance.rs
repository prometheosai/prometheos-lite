use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AcceptanceCriterion {
    pub id: String,
    pub description: String,
    pub verification_method: VerificationMethod,
    pub status: CriterionStatus,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum VerificationMethod {
    TestCommand(String),
    StaticCheck(String),
    Review,
    Manual,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum CriterionStatus {
    Pending,
    Passed,
    Failed,
    NotApplicable,
}
pub fn compile_acceptance_criteria(reqs: &[String]) -> Vec<AcceptanceCriterion> {
    reqs.iter()
        .enumerate()
        .map(|(i, d)| AcceptanceCriterion {
            id: format!("AC-{}", i + 1),
            description: d.clone(),
            verification_method: VerificationMethod::Review,
            status: CriterionStatus::Pending,
        })
        .collect()
}
