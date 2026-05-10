use anyhow::Result;
use async_trait::async_trait;
use prometheos_lite::harness::edit_protocol::{EditOperation, SearchReplaceEdit};
use prometheos_lite::harness::patch_provider::{
    AggregatePatchProvider, GenerateRequest, GenerateResponse, PatchProvider, PatchProviderContext,
    ProviderCandidate, ProviderCapabilities, RepairRequest, RepairResponse, RiskEstimate,
};
use std::path::PathBuf;

#[derive(Clone)]
struct EmptyWithoutNotesProvider;

#[async_trait]
impl PatchProvider for EmptyWithoutNotesProvider {
    fn name(&self) -> &str {
        "empty-without-notes"
    }

    async fn generate(&self, _request: GenerateRequest) -> Result<GenerateResponse> {
        Ok(GenerateResponse {
            candidates: vec![],
            generation_time_ms: 1,
            provider_notes: None,
        })
    }

    async fn repair(&self, _request: RepairRequest) -> Result<RepairResponse> {
        unreachable!("not used in diagnostics tests")
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            can_generate: true,
            can_repair: false,
            max_candidates: 1,
            supported_operations: vec![],
            typical_latency_ms: 1,
        }
    }
}

#[derive(Clone)]
struct EmptyWithNotesProvider;

#[async_trait]
impl PatchProvider for EmptyWithNotesProvider {
    fn name(&self) -> &str {
        "empty-with-notes"
    }

    async fn generate(&self, _request: GenerateRequest) -> Result<GenerateResponse> {
        Ok(GenerateResponse {
            candidates: vec![],
            generation_time_ms: 1,
            provider_notes: Some("no candidates for this request".to_string()),
        })
    }

    async fn repair(&self, _request: RepairRequest) -> Result<RepairResponse> {
        unreachable!("not used in diagnostics tests")
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            can_generate: true,
            can_repair: false,
            max_candidates: 1,
            supported_operations: vec![],
            typical_latency_ms: 1,
        }
    }
}

#[derive(Clone)]
struct ErrorProvider;

#[async_trait]
impl PatchProvider for ErrorProvider {
    fn name(&self) -> &str {
        "error-provider"
    }

    async fn generate(&self, _request: GenerateRequest) -> Result<GenerateResponse> {
        anyhow::bail!("provider execution failed")
    }

    async fn repair(&self, _request: RepairRequest) -> Result<RepairResponse> {
        unreachable!("not used in diagnostics tests")
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            can_generate: true,
            can_repair: false,
            max_candidates: 1,
            supported_operations: vec![],
            typical_latency_ms: 1,
        }
    }
}

#[derive(Clone)]
struct CandidateProvider;

#[async_trait]
impl PatchProvider for CandidateProvider {
    fn name(&self) -> &str {
        "candidate-provider"
    }

    async fn generate(&self, _request: GenerateRequest) -> Result<GenerateResponse> {
        Ok(GenerateResponse {
            candidates: vec![ProviderCandidate {
                edits: vec![EditOperation::SearchReplace(SearchReplaceEdit {
                    file: PathBuf::from("src/main.rs"),
                    search: "old".to_string(),
                    replace: "new".to_string(),
                    replace_all: None,
                    context_lines: None,
                })],
                source: "candidate-provider".to_string(),
                strategy: "unit-test".to_string(),
                confidence: 90,
                reasoning: "deterministic candidate for test".to_string(),
                estimated_risk: RiskEstimate::Low,
            }],
            generation_time_ms: 1,
            provider_notes: Some("candidate returned".to_string()),
        })
    }

    async fn repair(&self, _request: RepairRequest) -> Result<RepairResponse> {
        unreachable!("not used in diagnostics tests")
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            can_generate: true,
            can_repair: false,
            max_candidates: 1,
            supported_operations: vec!["search_replace".to_string()],
            typical_latency_ms: 1,
        }
    }
}

fn request() -> GenerateRequest {
    GenerateRequest {
        context: PatchProviderContext::default(),
        preferred_strategies: vec![],
    }
}

#[tokio::test]
async fn empty_without_notes_is_rejected() {
    let mut aggregate = AggregatePatchProvider::new();
    aggregate.add_provider(Box::new(EmptyWithoutNotesProvider));

    let result = aggregate.generate(request()).await;
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("without diagnostic information")
    );
}

#[tokio::test]
async fn empty_with_notes_is_returned_as_diagnostic() {
    let mut aggregate = AggregatePatchProvider::new();
    aggregate.add_provider(Box::new(EmptyWithNotesProvider));

    let result = aggregate
        .generate(request())
        .await
        .expect("generate should pass");
    assert!(result.candidates.is_empty());
    assert_eq!(
        result.provider_notes.as_deref(),
        Some("no candidates for this request")
    );
}

#[tokio::test]
async fn provider_error_bubbles_when_no_success() {
    let mut aggregate = AggregatePatchProvider::new();
    aggregate.add_provider(Box::new(ErrorProvider));

    let result = aggregate.generate(request()).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("execution failed"));
}

#[tokio::test]
async fn candidates_short_circuit_generation() {
    let mut aggregate = AggregatePatchProvider::new();
    aggregate.add_provider(Box::new(CandidateProvider));
    aggregate.add_provider(Box::new(ErrorProvider));

    let result = aggregate
        .generate(request())
        .await
        .expect("generate should pass");
    assert_eq!(result.candidates.len(), 1);
    assert_eq!(result.candidates[0].source, "candidate-provider");
}
