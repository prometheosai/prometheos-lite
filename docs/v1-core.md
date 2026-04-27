# PrometheOS Lite V1 Core Architecture

## Overview

PrometheOS Lite V1 implements a flow-centric architecture where all execution paths are unified under a single `FlowRunner`. The system is designed to be production-grade with proper error handling, persistence, and guardrails.

## Core Components

### Flow Engine
- **Flow**: The core execution unit that orchestrates nodes through transitions
- **Node**: Individual processing units that accept input and produce output
- **SharedState**: Explicit state management with typed sections (input, context, working, output, meta)
- **FlowRunner**: Executes flows with proper lifecycle management

### Node Factory
- **NodeFactory**: Trait for creating nodes from configurations
- **DefaultNodeFactory**: Built-in implementation for standard node types
- **Built-in Nodes**: PlannerNode, CoderNode, ReviewerNode, LlmNode, ToolNode, etc.

### Persistence
- **RunDb**: SQLite-backed persistence for flow runs and events
- **ContinuationEngine**: Checkpoint management for state snapshots
- **FlowRun**: Metadata tracking flow run lifecycle and status

### Intelligence
- **ModelRouter**: Routes LLM requests to appropriate models
- **ToolRuntime**: Executes tools with sandboxing and permission checks
- **MemoryService**: Manages long-term memory storage and retrieval

### Guardrails
- **ToolPermission**: Declarative permission system (Network, FileRead, FileWrite, Shell, Env)
- **ToolPolicy**: Defines allowed permissions and approval requirements
- **ToolSandboxProfile**: Runtime enforcement of tool restrictions
- **PersonalityMode**: AI personality modes (Companion, Navigator, Anchor, Mirror)
- **ConstitutionalFilter**: Post-generation filtering for personality constraints

## Execution Flow

1. **Intent Classification**: User input is classified into an intent
2. **Flow Selection**: Intent is mapped to a specific flow file
3. **Flow Loading**: Flow is loaded from YAML/JSON using FlowLoader
4. **Runtime Construction**: RuntimeContext is built with required services
5. **Flow Execution**: FlowRunner executes the flow with proper state management
6. **Persistence**: Run metadata and checkpoints are saved
7. **Output**: FinalOutput is returned with evaluation metrics

## Key Design Principles

- **Explicit State**: All state is explicitly managed through SharedState
- **Deterministic Execution**: Flows are deterministic given the same inputs
- **Observability**: All execution is traced and can be replayed
- **Safety**: Tool execution is sandboxed with conservative defaults
- **Extensibility**: New node types and flows can be added without core changes
