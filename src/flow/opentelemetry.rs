//! OpenTelemetry integration for trace export

use anyhow::Result;
use opentelemetry::trace::{
    Span as SpanTrait, SpanKind, Status, Tracer as OtelTracer, TracerProvider as OtelTracerProvider,
};
use opentelemetry::{Key, KeyValue};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::trace::{self as sdktrace, TracerProvider};

use super::tracing::{HierarchicalTrace, LlmCall, NodeRun, ToolCall};

/// Span type alias for the SDK
type Span = sdktrace::Span;

/// OpenTelemetry exporter configuration
#[derive(Debug, Clone)]
pub struct OtelConfig {
    /// Service name for trace identification
    pub service_name: String,
    /// OTLP endpoint (e.g., "http://localhost:4317")
    pub endpoint: Option<String>,
    /// Whether to export to stdout (for debugging)
    pub export_to_stdout: bool,
}

impl Default for OtelConfig {
    fn default() -> Self {
        Self {
            service_name: "prometheos-lite".to_string(),
            endpoint: None,
            export_to_stdout: false,
        }
    }
}

impl OtelConfig {
    /// Create a new config with custom service name
    pub fn with_service_name(service_name: impl Into<String>) -> Self {
        Self {
            service_name: service_name.into(),
            ..Default::default()
        }
    }

    /// Set the OTLP endpoint
    pub fn with_endpoint(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: Some(endpoint.into()),
            ..Default::default()
        }
    }

    /// Enable stdout export
    pub fn with_stdout_export() -> Self {
        Self {
            export_to_stdout: true,
            ..Default::default()
        }
    }
}

/// OpenTelemetry exporter for hierarchical traces
pub struct OtelExporter {
    tracer: sdktrace::Tracer,
    config: OtelConfig,
}

impl OtelExporter {
    /// Create a new OpenTelemetry exporter
    pub fn new(config: OtelConfig) -> Result<Self> {
        let tracer_provider = if config.export_to_stdout {
            // Export to stdout for debugging
            let exporter = opentelemetry_stdout::SpanExporter::default();
            sdktrace::TracerProvider::builder()
                .with_simple_exporter(exporter)
                .build()
        } else if let Some(endpoint) = &config.endpoint {
            // Export to OTLP endpoint
            let exporter = opentelemetry_otlp::new_exporter()
                .tonic()
                .with_endpoint(endpoint)
                .with_timeout(std::time::Duration::from_secs(3))
                .build_span_exporter()?;

            sdktrace::TracerProvider::builder()
                .with_batch_exporter(exporter, opentelemetry_sdk::runtime::Tokio)
                .build()
        } else {
            // No export configured, use no-op tracer
            sdktrace::TracerProvider::builder().build()
        };

        let tracer = tracer_provider.tracer(config.service_name.clone());

        Ok(Self { tracer, config })
    }

    /// Export a hierarchical trace to OpenTelemetry
    pub fn export_trace(&self, trace: &HierarchicalTrace) -> Result<()> {
        let mut root_span = self
            .tracer
            .span_builder("flow_execution")
            .with_kind(SpanKind::Server)
            .start(&self.tracer);

        root_span.set_attributes(vec![
            KeyValue::new("trace_id", trace.trace_id.clone()),
            KeyValue::new("flow_run_id", trace.flow_run_id.clone()),
        ]);

        if let Some(work_context_id) = &trace.work_context_id {
            root_span.set_attribute(KeyValue::new("work_context_id", work_context_id.clone()));
        }

        // Export node runs as independent spans (simplified for now)
        for node_run in &trace.node_runs {
            self.export_node_run(node_run)?;
        }

        // Export tool calls as independent spans (simplified for now)
        for tool_call in &trace.tool_calls {
            self.export_tool_call(tool_call)?;
        }

        // Export LLM calls as independent spans (simplified for now)
        for llm_call in &trace.llm_calls {
            self.export_llm_call(llm_call)?;
        }

        root_span.end();

        Ok(())
    }

    /// Export a node run as a span
    fn export_node_run(&self, node_run: &NodeRun) -> Result<()> {
        let mut span = self
            .tracer
            .span_builder("node_execution")
            .with_kind(SpanKind::Internal)
            .start(&self.tracer);

        span.set_attributes(vec![
            KeyValue::new("node_id", node_run.node_id.clone()),
            KeyValue::new("trace_id", node_run.trace_id.clone()),
            KeyValue::new("status", node_run.status.clone()),
            KeyValue::new("duration_ms", node_run.duration_ms as i64),
        ]);

        if let Some(input_summary) = &node_run.input_summary {
            span.set_attribute(KeyValue::new("input_summary", input_summary.clone()));
        }

        if let Some(output_summary) = &node_run.output_summary {
            span.set_attribute(KeyValue::new("output_summary", output_summary.clone()));
        }

        if let Some(error) = &node_run.error {
            span.set_status(Status::error(error.clone()));
            span.set_attribute(KeyValue::new("error", error.clone()));
        }

        span.end();

        Ok(())
    }

    /// Export a tool call as a span
    fn export_tool_call(&self, tool_call: &ToolCall) -> Result<()> {
        let mut span = self
            .tracer
            .span_builder("tool_call")
            .with_kind(SpanKind::Client)
            .start(&self.tracer);

        span.set_attributes(vec![
            KeyValue::new("tool_name", tool_call.tool_name.clone()),
            KeyValue::new("trace_id", tool_call.trace_id.clone()),
            KeyValue::new("args_hash", tool_call.args_hash.clone()),
            KeyValue::new("result_hash", tool_call.result_hash.clone()),
            KeyValue::new("success", tool_call.success),
            KeyValue::new("duration_ms", tool_call.duration_ms as i64),
        ]);

        if !tool_call.success {
            span.set_status(Status::error("Tool call failed"));
        }

        span.end();

        Ok(())
    }

    /// Export an LLM call as a span
    fn export_llm_call(&self, llm_call: &LlmCall) -> Result<()> {
        let mut span = self
            .tracer
            .span_builder("llm_call")
            .with_kind(SpanKind::Client)
            .start(&self.tracer);

        span.set_attributes(vec![
            KeyValue::new("node_id", llm_call.node_id.clone()),
            KeyValue::new("trace_id", llm_call.trace_id.clone()),
            KeyValue::new("provider", llm_call.provider.clone()),
            KeyValue::new("model", llm_call.model.clone()),
            KeyValue::new("prompt_tokens", llm_call.prompt_tokens as i64),
            KeyValue::new("completion_tokens", llm_call.completion_tokens as i64),
            KeyValue::new("latency_ms", llm_call.latency_ms as i64),
        ]);

        if let Some(error) = &llm_call.error {
            span.set_status(Status::error(error.clone()));
            span.set_attribute(KeyValue::new("error", error.clone()));
        }

        span.end();

        Ok(())
    }

    /// Get the tracer reference
    pub fn tracer(&self) -> &sdktrace::Tracer {
        &self.tracer
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::tracing::{RunId, TraceId};
    use chrono::Utc;

    #[test]
    fn test_otel_config_default() {
        let config = OtelConfig::default();
        assert_eq!(config.service_name, "prometheos-lite");
        assert!(config.endpoint.is_none());
        assert!(!config.export_to_stdout);
    }

    #[test]
    fn test_otel_config_with_service_name() {
        let config = OtelConfig::with_service_name("my-service");
        assert_eq!(config.service_name, "my-service");
    }

    #[test]
    fn test_otel_config_with_endpoint() {
        let config = OtelConfig::with_endpoint("http://localhost:4317");
        assert_eq!(config.endpoint, Some("http://localhost:4317".to_string()));
    }

    #[test]
    fn test_otel_config_with_stdout() {
        let config = OtelConfig::with_stdout_export();
        assert!(config.export_to_stdout);
    }

    #[test]
    fn test_otel_exporter_stdout() {
        let config = OtelConfig::with_stdout_export();
        let exporter = OtelExporter::new(config);
        assert!(exporter.is_ok());
    }

    #[test]
    fn test_otel_exporter_no_endpoint() {
        let config = OtelConfig::default();
        let exporter = OtelExporter::new(config);
        assert!(exporter.is_ok());
    }

    #[test]
    fn test_export_trace() {
        let config = OtelConfig::default();
        let exporter = OtelExporter::new(config).unwrap();

        let trace = HierarchicalTrace {
            trace_id: TraceId::from("trace-123"),
            work_context_id: Some("ctx-123".to_string()),
            flow_run_id: RunId::from("run-123"),
            node_runs: vec![],
            tool_calls: vec![],
            llm_calls: vec![],
            started_at: Utc::now(),
            completed_at: Some(Utc::now()),
        };

        let result = exporter.export_trace(&trace);
        assert!(result.is_ok());
    }

    #[test]
    fn test_export_node_run() {
        let config = OtelConfig::default();
        let exporter = OtelExporter::new(config).unwrap();

        let node_run = NodeRun {
            node_id: "planner".to_string(),
            trace_id: TraceId::from("trace-123"),
            input_summary: Some("input".to_string()),
            output_summary: Some("output".to_string()),
            status: "success".to_string(),
            duration_ms: 100,
            error: None,
            started_at: Utc::now(),
            completed_at: Utc::now(),
        };

        let result = exporter.export_node_run(&node_run);
        assert!(result.is_ok());
    }

    #[test]
    fn test_export_tool_call() {
        let config = OtelConfig::default();
        let exporter = OtelExporter::new(config).unwrap();

        let tool_call = ToolCall {
            tool_name: "file_writer".to_string(),
            trace_id: TraceId::from("trace-123"),
            args_hash: "hash123".to_string(),
            result_hash: "hash456".to_string(),
            success: true,
            duration_ms: 50,
            called_at: Utc::now(),
        };

        let result = exporter.export_tool_call(&tool_call);
        assert!(result.is_ok());
    }

    #[test]
    fn test_export_llm_call() {
        let config = OtelConfig::default();
        let exporter = OtelExporter::new(config).unwrap();

        let llm_call = LlmCall {
            node_id: "planner".to_string(),
            trace_id: TraceId::from("trace-123"),
            provider: "openai".to_string(),
            model: "gpt-4".to_string(),
            prompt_tokens: 100,
            completion_tokens: 200,
            latency_ms: 1500,
            error: None,
            started_at: Utc::now(),
            completed_at: Some(Utc::now()),
        };

        let result = exporter.export_llm_call(&llm_call);
        assert!(result.is_ok());
    }

    #[test]
    fn test_export_complex_hierarchical_trace() {
        let config = OtelConfig::default();
        let exporter = OtelExporter::new(config).unwrap();

        let node_run = NodeRun {
            node_id: "planner".to_string(),
            trace_id: TraceId::from("trace-123"),
            input_summary: Some("Create API".to_string()),
            output_summary: Some("Plan created".to_string()),
            status: "success".to_string(),
            duration_ms: 100,
            error: None,
            started_at: Utc::now(),
            completed_at: Utc::now(),
        };

        let tool_call = ToolCall {
            tool_name: "file_writer".to_string(),
            trace_id: TraceId::from("trace-123"),
            args_hash: "hash123".to_string(),
            result_hash: "hash456".to_string(),
            success: true,
            duration_ms: 50,
            called_at: Utc::now(),
        };

        let llm_call = LlmCall {
            node_id: "planner".to_string(),
            trace_id: TraceId::from("trace-123"),
            provider: "openai".to_string(),
            model: "gpt-4".to_string(),
            prompt_tokens: 100,
            completion_tokens: 200,
            latency_ms: 1500,
            error: None,
            started_at: Utc::now(),
            completed_at: Some(Utc::now()),
        };

        let trace = HierarchicalTrace {
            trace_id: TraceId::from("trace-123"),
            work_context_id: Some("ctx-123".to_string()),
            flow_run_id: RunId::from("run-123"),
            node_runs: vec![node_run],
            tool_calls: vec![tool_call],
            llm_calls: vec![llm_call],
            started_at: Utc::now(),
            completed_at: Some(Utc::now()),
        };

        let result = exporter.export_trace(&trace);
        assert!(result.is_ok());
    }

    #[test]
    fn test_export_node_run_with_error() {
        let config = OtelConfig::default();
        let exporter = OtelExporter::new(config).unwrap();

        let node_run = NodeRun {
            node_id: "failing_node".to_string(),
            trace_id: TraceId::from("trace-123"),
            input_summary: Some("input".to_string()),
            output_summary: None,
            status: "failed".to_string(),
            duration_ms: 100,
            error: Some("Timeout error".to_string()),
            started_at: Utc::now(),
            completed_at: Utc::now(),
        };

        let result = exporter.export_node_run(&node_run);
        assert!(result.is_ok());
    }

    #[test]
    fn test_export_tool_call_failure() {
        let config = OtelConfig::default();
        let exporter = OtelExporter::new(config).unwrap();

        let tool_call = ToolCall {
            tool_name: "file_writer".to_string(),
            trace_id: TraceId::from("trace-123"),
            args_hash: "hash123".to_string(),
            result_hash: "hash456".to_string(),
            success: false,
            duration_ms: 50,
            called_at: Utc::now(),
        };

        let result = exporter.export_tool_call(&tool_call);
        assert!(result.is_ok());
    }

    #[test]
    fn test_export_llm_call_with_error() {
        let config = OtelConfig::default();
        let exporter = OtelExporter::new(config).unwrap();

        let llm_call = LlmCall {
            node_id: "planner".to_string(),
            trace_id: TraceId::from("trace-123"),
            provider: "openai".to_string(),
            model: "gpt-4".to_string(),
            prompt_tokens: 100,
            completion_tokens: 0,
            latency_ms: 5000,
            error: Some("Rate limit exceeded".to_string()),
            started_at: Utc::now(),
            completed_at: Some(Utc::now()),
        };

        let result = exporter.export_llm_call(&llm_call);
        assert!(result.is_ok());
    }

    #[test]
    fn test_otel_config_builder_methods() {
        let config1 = OtelConfig::with_service_name("custom-service");
        assert_eq!(config1.service_name, "custom-service");

        let config2 = OtelConfig::with_endpoint("http://localhost:4317");
        assert_eq!(config2.endpoint, Some("http://localhost:4317".to_string()));

        let config3 = OtelConfig::with_stdout_export();
        assert!(config3.export_to_stdout);
    }

    #[test]
    fn test_exporter_tracer_reference() {
        let config = OtelConfig::default();
        let exporter = OtelExporter::new(config).unwrap();

        let tracer = exporter.tracer();
        assert!(!std::ptr::eq(tracer, std::ptr::null()));
    }

    #[test]
    fn test_export_trace_with_no_completion() {
        let config = OtelConfig::default();
        let exporter = OtelExporter::new(config).unwrap();

        let trace = HierarchicalTrace {
            trace_id: TraceId::from("trace-123"),
            work_context_id: None,
            flow_run_id: RunId::from("run-123"),
            node_runs: vec![],
            tool_calls: vec![],
            llm_calls: vec![],
            started_at: Utc::now(),
            completed_at: None, // Incomplete trace
        };

        let result = exporter.export_trace(&trace);
        assert!(result.is_ok());
    }

    #[test]
    fn test_export_node_run_no_output_summary() {
        let config = OtelConfig::default();
        let exporter = OtelExporter::new(config).unwrap();

        let node_run = NodeRun {
            node_id: "planner".to_string(),
            trace_id: TraceId::from("trace-123"),
            input_summary: Some("input".to_string()),
            output_summary: None, // No output
            status: "in_progress".to_string(),
            duration_ms: 100,
            error: None,
            started_at: Utc::now(),
            completed_at: Utc::now(),
        };

        let result = exporter.export_node_run(&node_run);
        assert!(result.is_ok());
    }
}
