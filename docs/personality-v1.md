# Personality System V1

## Overview

The personality system allows PrometheOS Lite to adapt its communication style based on user preferences and context. It provides four distinct personality modes that influence how the AI responds.

## Personality Modes

### Companion
- **Description**: Friendly, conversational companion
- **Tone**: Warm, engaging, supportive
- **Use Case**: General conversation, casual interactions
- **System Prompt**: "You are a friendly, conversational companion. Be warm, engaging, and supportive in your responses."

### Navigator
- **Description**: Helpful guide that explains reasoning
- **Tone**: Explanatory, step-by-step, educational
- **Use Case**: Learning, understanding complex topics, guidance
- **System Prompt**: "You are a helpful guide that explains your reasoning. Break down complex topics and show your thought process clearly."

### Anchor
- **Description**: Stable, reassuring presence with gentle tone
- **Tone**: Calming, gentle, supportive
- **Use Case**: Emotional support, stressful situations, reassurance
- **System Prompt**: "You are a stable, reassuring presence. Use a gentle, calming tone and provide emotional support when needed."

### Mirror
- **Description**: Direct, reflective mirror that shows things as they are
- **Tone**: Direct, honest, straightforward
- **Use Case**: Honest feedback, direct answers, reflection
- **System Prompt**: "You are a direct, reflective mirror. Show things as they are without unnecessary qualifiers. Be honest and straightforward."

## Mode Selection

### Automatic Selection

The `ModeSelector` automatically selects a personality mode based on text input:

```rust
use prometheos_lite::personality::{ModeSelector, PersonalityMode};

let selector = ModeSelector::new(PersonalityMode::default());
let mode = selector.select_from_text("help me understand this");
// Returns PersonalityMode::Navigator
```

**Heuristics:**
- Contains "help", "explain", "guide" → Navigator
- Contains "calm", "reassure", "gentle" → Anchor
- Contains "direct", "honest", "reflect" → Mirror
- Default → Companion

### Manual Selection

Explicitly select a mode by name:

```rust
let mode = selector.select_by_name("anchor")?;
// Returns Some(PersonalityMode::Anchor)
```

## Prompt Injection

### PromptContext

The `PromptContext` injects personality into LLM prompts:

```rust
use prometheos_lite::personality::PromptContext;

let context = PromptContext::new(PersonalityMode::Navigator);
let enhanced_prompt = context.inject_into_prompt("Explain quantum computing");
```

This adds the system prompt and personality instructions to the base prompt.

## Constitutional Filter

### Post-Generation Filtering

The `ConstitutionalFilter` applies personality constraints after generation:

```rust
use prometheos_lite::personality::ConstitutionalFilter;

let filter = ConstitutionalFilter::new(PersonalityMode::Anchor);
let filtered = filter.filter("This is definitely the answer");
// Returns "This is likely the answer" (Anchor mode softens certainty)
```

### Mode-Specific Filters

**Anchor Mode:**
- Softens "must" → "should"
- Softens "have to" → "might want to"
- Softens "need to" → "could consider"

**Mirror Mode:**
- Removes "I think"
- Removes "I believe"
- Removes "It seems like"

### Universal Filters

All modes apply:
- **Shorten excessive output**: Limits output to 2000 characters
- **Remove false certainty**: "definitely" → "likely", "certainly" → "probably"

## State Storage

The selected personality mode is stored in `SharedState.meta`:

```rust
use prometheos_lite::flow::SharedState;

let mut state = SharedState::new();
state.set_personality_mode("companion");
let mode = state.get_personality_mode();
// Returns Some("companion")
```

## CLI Integration

The CLI displays the selected personality mode in verbose mode:

```bash
prometheos flow run flow.yaml --input "help me" --verbose
# Output:
# Personality mode: Navigator
#   Helpful guide that explains reasoning
```

## Implementation

### Adding to Flow Execution

To use personality in flow execution:

1. Select mode based on input
2. Store mode in `SharedState.meta.personality_mode`
3. Inject personality context into LLM node prompts
4. Apply constitutional filter to LLM outputs

### Example

```rust
let mode_selector = ModeSelector::new(PersonalityMode::default());
let mode = mode_selector.select_from_text(&input_text);
state.set_personality_mode(mode.display_name());

// In LLM node:
let prompt_context = PromptContext::new(mode);
let enhanced_prompt = prompt_context.inject_into_prompt(&base_prompt);
let response = llm.generate(&enhanced_prompt).await?;

// Apply filter
let filter = ConstitutionalFilter::new(mode);
let filtered_response = filter.filter(&response);
```

## Best Practices

- Use automatic selection for general use cases
- Allow manual override for specific scenarios
- Apply constitutional filter consistently
- Store mode in state for observability
- Display selected mode in debug output
