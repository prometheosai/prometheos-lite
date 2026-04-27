# Runtime Context

## Overview

`RuntimeContext` is a service registry that provides all necessary services for flow execution. It acts as a dependency injection container for the flow engine.

## Components

### ModelRouter
Routes LLM requests to appropriate models based on configuration and requirements.

```rust
use prometheos_lite::flow::intelligence::ModelRouter;

let model_router = ModelRouter::new(config)?;
let response = model_router.generate(&prompt, &model_config).await?;
```

### ToolRuntime
Executes tools with sandboxing and permission checks.

```rust
use prometheos_lite::flow::intelligence::ToolRuntime;

let tool_runtime = ToolRuntime::with_default_profile();
let output = tool_runtime.execute_command("ls", vec!["-la"]).await?;
```

### MemoryService
Manages long-term memory storage and retrieval.

```rust
use prometheos_lite::flow::memory::MemoryService;

let memory_service = MemoryService::new()?;
memory_service.write(&memory).await?;
let retrieved = memory_service.read(&query).await?;
```

## Building RuntimeContext

### Using RuntimeBuilder

```rust
use prometheos_lite::cli::RuntimeBuilder;
use prometheos_lite::config::AppConfig;

let config = AppConfig::load()?;
let runtime = RuntimeBuilder::new(config)
    .build_full()?;
```

### Minimal Runtime

```rust
let runtime = RuntimeBuilder::new(config)
    .build_minimal()?;
```

### Custom Runtime

```rust
let runtime = RuntimeBuilder::new(config)
    .with_model_router(model_router)
    .with_tool_runtime(tool_runtime)
    .with_memory_service(memory_service)
    .build()?;
```

## Service Access

Services can be accessed from `RuntimeContext`:

```rust
let model_router = runtime.model_router.as_ref().unwrap();
let tool_runtime = runtime.tool_runtime.as_ref().unwrap();
let memory_service = runtime.memory_service.as_ref().unwrap();
```

## Thread Safety

`RuntimeContext` uses `Arc` for thread-safe sharing across async tasks. All services are wrapped in `Arc` to enable concurrent access.

## Configuration

Runtime behavior is controlled through `AppConfig`:

```toml
[provider]
name = "openai"
api_key = "your-api-key"
model = "gpt-4"

[memory]
enabled = true
backend = "sqlite"
path = ".prometheos/memory.db"

[tools]
sandbox_enabled = true
```
