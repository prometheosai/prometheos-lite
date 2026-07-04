# PrometheOS Lite - Available Flows

## Code Generation Flow
- File: flows/code-generation.json
- Intent: CODING_TASK
- Input: task description
- Output: generated files, code review
- Nodes: planner → coder → reviewer → file_writer → memory_write
- Description: Full code generation flow with planning, coding, and review phases
