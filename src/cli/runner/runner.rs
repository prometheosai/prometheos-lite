//! CLI Runner for executing flows

use anyhow::{Context, Result};
use std::sync::Arc;

use prometheos_lite::flow::{Flow, FlowBuilder, SharedState, NodeFactory, DefaultNodeFactory, IdWrapper, JsonLoader, YamlLoader, validate_flow_file, FlowLoader};
use prometheos_lite::flow::loader::FlowFile;

/// CLI Runner for executing flows
pub struct FlowRunner {
    flow: Flow,
    tracer: Option<prometheos_lite::flow::tracing::SharedTracer>,
    debug_mode: bool,
    runtime: Option<prometheos_lite::flow::RuntimeContext>,
}

impl FlowRunner {
    /// Create a new FlowRunner from a Flow
    pub fn new(flow: Flow) -> Self {
        Self {
            flow,
            tracer: None,
            debug_mode: false,
            runtime: None,
        }
    }

    /// Enable debug mode
    pub fn enable_debug_mode(&mut self) {
        self.debug_mode = true;
    }

    /// Get the tracer if set
    pub fn tracer(&self) -> Option<&prometheos_lite::flow::tracing::SharedTracer> {
        self.tracer.as_ref()
    }

    /// Get the flow
    pub fn get_flow(&self) -> &Flow {
        &self.flow
    }

    /// Set the runtime context for service injection
    pub fn with_runtime(mut self, runtime: prometheos_lite::flow::RuntimeContext) -> Self {
        self.runtime = Some(runtime);
        self
    }

    /// Load a flow from a JSON file
    pub fn from_json_file(path: std::path::PathBuf) -> Result<Self> {
        Self::from_json_file_with_runtime(path, None)
    }

    /// Load a flow from a YAML file
    pub fn from_yaml_file(path: std::path::PathBuf) -> Result<Self> {
        Self::from_yaml_file_with_runtime(path, None)
    }

    /// Load a flow from a JSON file with a RuntimeContext
    pub fn from_json_file_with_runtime(
        path: std::path::PathBuf,
        runtime: Option<prometheos_lite::flow::RuntimeContext>,
    ) -> Result<Self> {
        let loader = JsonLoader::new();
        let flow_file = loader.load_from_path(&path)?;

        // Validate flow file structure according to Flow JSON Schema v1
        validate_flow_file(&flow_file).context("Flow file validation failed")?;

        let flow = Self::build_flow_from_file(flow_file, runtime.clone())?;

        // Create tracer for this flow
        let tracer = prometheos_lite::flow::tracing::create_tracer();

        Ok(Self {
            flow,
            tracer: Some(tracer),
            debug_mode: false,
            runtime,
        })
    }

    /// Load a flow from a YAML file with a RuntimeContext
    pub fn from_yaml_file_with_runtime(
        path: std::path::PathBuf,
        runtime: Option<prometheos_lite::flow::RuntimeContext>,
    ) -> Result<Self> {
        let loader = YamlLoader::new();
        let flow_file = loader.load_from_path(&path)?;

        // Validate flow file structure according to Flow JSON Schema v1
        validate_flow_file(&flow_file).context("Flow file validation failed")?;

        let flow = Self::build_flow_from_file(flow_file, runtime.clone())?;

        // Create tracer for this flow
        let tracer = prometheos_lite::flow::tracing::create_tracer();

        Ok(Self {
            flow,
            tracer: Some(tracer),
            debug_mode: false,
            runtime,
        })
    }

    /// Build a Flow from a FlowFile using the NodeFactory
    fn build_flow_from_file(
        file: FlowFile,
        runtime: Option<prometheos_lite::flow::RuntimeContext>,
    ) -> Result<Flow> {
        let mut builder = FlowBuilder::new().start(file.start_node.clone());
        let factory: Box<dyn NodeFactory> = if let Some(rt) = runtime {
            Box::new(DefaultNodeFactory::from_runtime(rt))
        } else {
            Box::new(DefaultNodeFactory::new())
        };

        for node_def in &file.nodes {
            let node = factory.create(&node_def.node_type, node_def.config.clone())?;
            // Wrap the node in an IdWrapper to override the id
            let wrapped_node = Arc::new(IdWrapper::new(node_def.id.clone(), node));
            builder = builder.add_node(node_def.id.clone(), wrapped_node);
        }

        // Add transitions
        for transition in &file.transitions {
            builder = builder.add_transition(
                transition.from.clone(),
                transition.action.clone(),
                transition.to.clone(),
            );
        }

        builder.build().context("Failed to build flow from file")
    }

    /// Execute the flow with the given state
    pub async fn run(&mut self, state: &mut SharedState) -> Result<()> {
        self.flow.run(state).await
    }

    /// Run the flow with input data
    pub async fn run_with_input(&mut self, input: serde_json::Value) -> Result<SharedState> {
        let mut state = SharedState::new();

        // Inject input into state
        if let Some(obj) = input.as_object() {
            for (key, value) in obj {
                state.set_input(key.clone(), value.clone());
            }
        }

        // Attach tracer to flow if available
        if let Some(tracer) = &self.tracer {
            let flow_with_tracer = self.flow.clone().with_tracer(tracer.clone());
            self.flow = flow_with_tracer;
        }

        self.flow.run(&mut state).await?;
        Ok(state)
    }
}
