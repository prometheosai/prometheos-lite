So you finally cleaned the house enough that it doesn’t collapse when someone opens a window. Impressive. Let’s not ruin that by jumping into V1.3 like it’s a hackathon.

Here’s the **real V1.3 PRD**, not the “we’ll figure it out later” version.

---

# ## V1.3 PRD — WorkContext Playbooks & Evolution Engine

## Executive Context

You now have:

```txt
✔ WorkContext lifecycle
✔ Flow execution engine
✔ Orchestrator (real, not imaginary anymore)
✔ Metadata propagation (finally)
✔ No fake nodes pretending to work
```

What you **don’t have**:

```txt
✘ Learning
✘ Personalization
✘ Strategy reuse
✘ Performance optimization across contexts
```

Right now PrometheOS executes.

V1.3 makes it **improve**.

---

# ## Core Idea

```txt
WorkContext = execution
Playbook = strategy
Evolution Engine = learning
```

---

# ## EPIC 1 — WorkContextPlaybook System

## Goal

Create a persistent strategy layer per user + domain.

---

## Issue: Define Playbook Schema

**File:** `src/work/playbook/types.rs`

```rust
pub struct WorkContextPlaybook {
    pub id: String,
    pub user_id: String,

    pub domain_profile_id: String,
    pub name: String,
    pub description: String,

    pub preferred_flows: Vec<FlowPreference>,
    pub preferred_nodes: Vec<NodePreference>,

    pub default_autonomy: AutonomyLevel,
    pub approval_policy: ApprovalPolicy,

    pub research_depth: ResearchDepth,
    pub creativity_level: CreativityLevel,

    pub success_patterns: Vec<PatternRecord>,
    pub failure_patterns: Vec<PatternRecord>,

    pub evaluation_rules: Vec<EvaluationRule>,

    pub confidence: f32,
    pub usage_count: u32,
    pub updated_at: DateTime<Utc>,
}
```

---

## Issue: Playbook Repository

**File:** `src/db/repository/playbook.rs`

Requirements:

```txt
- create_playbook
- get_playbook_by_user_and_domain
- update_playbook
- increment_usage
- store patterns
```

---

## Issue: Playbook Resolver

**File:** `src/work/playbook/resolver.rs`

```rust
pub fn resolve_playbook(
    user_id: &str,
    domain: &WorkDomain
) -> Option<WorkContextPlaybook>
```

Fallback:

```txt
1. user+domain playbook
2. domain default
3. global default
```

---

# ## EPIC 2 — Playbook-Aware Orchestration

## Issue: Inject Playbook into WorkContext

**Modify:**
`WorkOrchestrator::submit_user_intent`

Add:

```rust
context.playbook_id = resolved_playbook.id
context.metadata["playbook"] = ...
```

---

## Issue: Flow Selection via Playbook

Replace:

```txt
domain → static flow
```

With:

```txt
playbook → preferred_flows → weighted selection
```

---

## Issue: Node Behavior Conditioning

Nodes should read:

```rust
state.get_playbook()
```

Examples:

```txt
PlannerNode → deeper plans if research_depth high
CoderNode → stricter output if approval required
ReviewerNode → more aggressive critique if evaluation strict
```

---

# ## EPIC 3 — Evolution Engine

## Goal

System learns from completed WorkContexts.

---

## Issue: Define Evolution Engine

**File:** `src/work/evolution/engine.rs`

```rust
pub struct WorkContextEvolutionEngine {
    repo: PlaybookRepository,
}
```

---

## Issue: Trigger Evolution

Hook into:

```rust
WorkOrchestrator::complete_context()
```

---

## Issue: Pattern Extraction

From:

```txt
- revision_count
- failure_reason
- user corrections
- success signals
- execution_metadata
```

Generate:

```rust
PatternRecord {
    pattern_type: Success | Failure,
    signal: String,
    weight: f32,
}
```

---

## Issue: Update Playbook

Rules:

```txt
- Increase weight of successful flows
- Penalize failing flows
- Adjust research_depth
- Adjust autonomy level
```

---

# ## EPIC 4 — Flow Performance Tracking

## Issue: FlowPerformanceRecord

```rust
pub struct FlowPerformanceRecord {
    pub flow_id: String,
    pub work_context_id: String,
    pub success_score: f32,
    pub duration_ms: u64,
    pub token_cost: u32,
    pub revision_count: u32,
}
```

---

## Issue: Store After Execution

Hook into:

```rust
WorkExecutionService
```

---

## Issue: Feed into Evolution Engine

```txt
FlowPerformance → Pattern → Playbook update
```

---

# ## EPIC 5 — Evaluation Layer

## Issue: Evaluation Engine

**File:** `src/work/evaluation/mod.rs`

```rust
pub fn evaluate_context(context: &WorkContext) -> EvaluationResult
```

Signals:

```txt
- artifact completeness
- number of retries
- latency
- user feedback (future)
```

---

# ## EPIC 6 — Strict Mode (No Regression Allowed)

## Issue: Production Strict Mode

```txt
- Missing inputs → error
- Missing services → error
- Empty outputs → error
- No silent fallbacks
```

---

# ## EPIC 7 — Observability (Minimum Viable)

## Issue: Execution Tracing

Add:

```txt
- run_id
- node execution logs
- tool calls
- LLM latency
```

Optional:

```txt
OpenTelemetry integration
```

---

# ## Definition of Done (V1.3)

```txt
✔ Playbook created automatically
✔ Playbook used during execution
✔ Evolution updates playbook after completion
✔ Flow selection influenced by history
✔ No silent fallbacks
✔ Evaluation score exists
✔ Metadata used for learning
```

---

# ## What NOT to do (seriously)

```txt
✘ Do NOT let LLM mutate playbooks directly
✘ Do NOT store arbitrary JSON chaos as “learning”
✘ Do NOT skip evaluation layer
✘ Do NOT mix Playbook with WorkContext
```

---

# ## V2 — Agents & Specialization

Focus:

```txt
- Specialized agents (Planner, Coder, Researcher)
- Tool-aware execution
- Skill abstraction layer
```

Outcome:

```txt
PrometheOS becomes a multi-agent system
```

---

# ## V3 — Coding Harness (Real One)

Focus:

```txt
- repo-aware tools (read_file, patch_file, search)
- deterministic execution
- test-driven loops
```

Outcome:

```txt
Competes with Claude Code / Copilot
```

---

# ## V4 — Swarm Execution

Focus:

```txt
- parallel WorkContexts
- agent coordination
- shared memory graph
```

Outcome:

```txt
true agentic swarm system
```

---

# ## V5 — Self-Improving System

Focus:

```txt
- automated skill creation
- flow synthesis
- meta-optimization
```

Outcome:

```txt
system evolves itself
```

---

# Final reality check

Right now:

```txt
You built a strong execution engine
```

V1.3:

```txt
You teach it how to get better
```

Everything after that:

```txt
You teach it how to outgrow you
```

Try not to mess that up.
