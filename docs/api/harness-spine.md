# Harness Spine API Documentation

## Overview

The Harness Spine API provides REST endpoints for orchestrating persistent work contexts through the WorkOrchestrator and PlaybookResolver.

## Base URL

```
http://localhost:3000
```

## WorkContext Endpoints

### Submit Intent

Submit a user intent to create or attach to a WorkContext.

**Endpoint**: `POST /work-contexts/submit-intent`

**Request Body**:
```json
{
  "user_id": "string",
  "message": "string",
  "conversation_id": "string (optional)"
}
```

**Response**:
```json
{
  "id": "string",
  "title": "string",
  "domain": "string",
  "goal": "string",
  "status": "string",
  "phase": "string",
  "created_at": "string (ISO 8601)",
  "updated_at": "string (ISO 8601)"
}
```

**Example**:
```bash
curl -X POST http://localhost:3000/work-contexts/submit-intent \
  -H "Content-Type: application/json" \
  -d '{
    "user_id": "user-123",
    "message": "Build a REST API for user management"
  }'
```

### Continue Context

Continue a blocked WorkContext.

**Endpoint**: `POST /work-contexts/:id/continue`

**Parameters**:
- `id` (path): WorkContext ID

**Response**: Same as Submit Intent

**Example**:
```bash
curl -X POST http://localhost:3000/work-contexts/ctx-123/continue
```

### Run Until Complete

Run a WorkContext until blocked or complete with execution limits.

**Endpoint**: `POST /work-contexts/:id/run-until-complete`

**Parameters**:
- `id` (path): WorkContext ID

**Request Body**:
```json
{
  "max_iterations": "number (optional, default: 10)",
  "max_runtime_ms": "number (optional, default: 300000)",
  "max_tool_calls": "number (optional, default: 50)",
  "max_cost": "number (optional, default: 1.0)"
}
```

**Response**: Same as Submit Intent

**Example**:
```bash
curl -X POST http://localhost:3000/work-contexts/ctx-123/run-until-complete \
  -H "Content-Type: application/json" \
  -d '{
    "max_iterations": 20,
    "max_runtime_ms": 600000
  }'
```

## Playbook Endpoints

### List Playbooks

List all playbooks for a user.

**Endpoint**: `GET /playbooks`

**Response**:
```json
[
  {
    "id": "string",
    "user_id": "string",
    "domain_profile_id": "string",
    "name": "string",
    "description": "string",
    "preferred_flows": ["string"],
    "default_research_depth": "string",
    "default_creativity_level": "string",
    "confidence": "number",
    "usage_count": "number",
    "updated_at": "string (ISO 8601)"
  }
]
```

**Example**:
```bash
curl http://localhost:3000/playbooks
```

### Get Playbook

Get a specific playbook.

**Endpoint**: `GET /playbooks/:id`

**Parameters**:
- `id` (path): Playbook ID

**Response**: Same as List Playbooks (single object)

**Example**:
```bash
curl http://localhost:3000/playbooks/pb-123
```

### Create Playbook

Create a new playbook.

**Endpoint**: `POST /playbooks`

**Request Body**:
```json
{
  "user_id": "string",
  "domain_profile_id": "string",
  "name": "string",
  "description": "string",
  "preferred_flows": ["string"],
  "default_research_depth": "string",
  "default_creativity_level": "string"
}
```

**Response**: Same as List Playbooks (single object)

**Example**:
```bash
curl -X POST http://localhost:3000/playbooks \
  -H "Content-Type: application/json" \
  -d '{
    "user_id": "user-123",
    "domain_profile_id": "software",
    "name": "Software Development",
    "description": "For software development tasks",
    "preferred_flows": ["codegen.flow.yaml"],
    "default_research_depth": "standard",
    "default_creativity_level": "balanced"
  }'
```

### Update Playbook

Update an existing playbook.

**Endpoint**: `PUT /playbooks/:id`

**Parameters**:
- `id` (path): Playbook ID

**Request Body**:
```json
{
  "name": "string (optional)",
  "description": "string (optional)",
  "preferred_flows": ["string"] (optional),
  "default_research_depth": "string (optional)",
  "default_creativity_level": "string (optional)"
}
```

**Response**: Same as List Playbooks (single object)

**Example**:
```bash
curl -X PUT http://localhost:3000/playbooks/pb-123 \
  -H "Content-Type: application/json" \
  -d '{
    "description": "Updated description"
  }'
```

## Execution Limits

The `run-until-complete` endpoint accepts execution limits to control autonomous execution:

| Parameter | Default | Description |
|-----------|---------|-------------|
| `max_iterations` | 10 | Maximum number of execution iterations |
| `max_runtime_ms` | 300000 | Maximum runtime in milliseconds (5 minutes) |
| `max_tool_calls` | 50 | Maximum number of tool calls |
| `max_cost` | 1.0 | Maximum cost in dollars |

## Error Responses

All endpoints may return error responses:

```json
{
  "error": "string"
}
```

**Status Codes**:
- `200 OK`: Successful request
- `400 Bad Request`: Invalid request body
- `404 Not Found`: Resource not found
- `500 Internal Server Error`: Server error

## Research Depth Values

- `minimal`: Minimal research
- `standard`: Standard research depth
- `deep`: Deep research
- `exhaustive`: Exhaustive research

## Creativity Level Values

- `conservative`: Conservative creativity
- `balanced`: Balanced creativity
- `creative`: High creativity

## Status Values

WorkContext status values:
- `draft`: Initial state
- `in_progress`: Currently executing
- `awaiting_approval`: Waiting for user approval
- `completed`: Successfully completed
- `blocked`: Blocked on user input or external factor

## Phase Values

WorkContext phase values:
- `intake`: Initial intake phase
- `planning`: Planning phase
- `execution`: Execution phase
- `review`: Review phase
- `finalization`: Finalization phase
