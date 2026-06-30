// Verb mapping for thinking indicator
// Maps backend node names to verb categories for intelligent UX

export const NODE_VERB_CATEGORY: Record<string, VerbCategory | null> = {
  system: "fallback",
  planner: "strategist",
  coder: "builder",
  reviewer: "reviewer",
  memory_write: "memory",
  assistant: null, // final response, no spinner
}

export type VerbCategory = "strategist" | "builder" | "reviewer" | "memory" | "fallback" | "error"

export const VERBS: Record<VerbCategory, string[]> = {
  strategist: ["Orchestrating", "Structuring", "Prioritizing", "Sequencing"],
  builder: ["Processing", "Executing", "Integrating", "Refactoring"],
  reviewer: ["Analyzing", "Validating", "Stress-testing", "Calibrating"],
  memory: ["Integrating", "Indexing", "Grounding", "Remembering"],
  fallback: ["Thinking", "Calibrating", "Resolving"],
  error: ["Recalibrating", "Recovering", "Adjusting"],
}

/**
 * Get an appropriate verb for a given node name
 * Returns null if no verb should be displayed (e.g., for assistant responses)
 */
export function getVerbForNode(node: string): string | null {
  const category = NODE_VERB_CATEGORY[node]

  if (!category) return null

  const pool = VERBS[category] ?? VERBS.fallback
  return pool[Math.floor(Math.random() * pool.length)]
}

/**
 * Get an error recovery verb
 */
export function getErrorVerb(): string {
  const pool = VERBS.error
  return pool[Math.floor(Math.random() * pool.length)]
}
