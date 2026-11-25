# ADR — Topology-aware local inference admission

- Status: PROPOSED (non-normative research)
- Date: 2026-11-25
- Tracking issue: https://github.com/prometheosai/prometheos-lite/issues/360
- Observed baseline: `b9553f3f21bb79cc7e73b0b5c54390e8d649398d`

## Context and shared findings

Apple Silicon UMA allows processors to access shared physical memory. MLX exposes common arrays with processor-specific execution and lazy evaluation; shared buffers can avoid some copies, not synchronization or all temporary allocations. Metal's recommended working-set size is guidance, not total GPU-exclusive capacity. CUDA Unified Memory provides managed address/access semantics; on discrete devices physical RAM and VRAM remain separate and migration/remote access has cost, especially with platform-specific restrictions. Windows shared GPU memory is not local GDDR. Capacity determines possible fit; bandwidth, latency and compute determine performance. SSD spill is not equivalent to VRAM. A shared immutable model can reduce duplicated weight allocations, but KV, scratch, authorization, isolation and concurrency still matter.

```text
Declared work + effective authority
              |
     compatible resource plan
              |
   physical pool + device topology
              |
    backend execution + observation
              |
    attributable evidence / release
```

Adopt the abstraction, not the silicon or vendor-specific implementation. Keep logical interfaces stable while physical constraints remain explicit. Existing backend APIs and supported placement strategies take precedence over theoretical designs.

## Evidence status and constraints

PROPOSED / NOT RUN. No runtime, code, schema, dependency, infrastructure or governance change is authorized by this ADR. No local benchmark, exact-head verification or independent review has been executed. No claims about actual hardware capacity, throughput or project implementation. No paid infrastructure, cloud fallback without policy approval, deployment, self-approval, direct main push, automatic merge, or bypass of any existing security hold or dependency gate. Source documentation is conceptual evidence, not benchmark evidence. Exact backend revisions and rights/licensing must be qualified before adoption; no external source code copied.

## Source references (reviewed 2026-11-25)

- Apple Metal: https://developer.apple.com/documentation/metal/mtldevice/recommendedmaxworkingsetsize
- Apple Metal storage modes: https://developer.apple.com/documentation/metal/setting-resource-storage-modes
- MLX unified memory: https://ml-explore.github.io/mlx/build/html/usage/unified_memory.html
- NVIDIA CUDA Unified Memory: https://docs.nvidia.com/cuda/cuda-programming-guide/04-special-topics/unified-memory.html
- llama.cpp multi-GPU: https://github.com/ggml-org/llama.cpp/blob/master/docs/multi-gpu.md
- vLLM prefix caching (candidate reference; version qualification required): https://docs.vllm.ai/en/latest/features/automatic_prefix_caching.html

## Coordinated records (distinct owners)

- prometheosai/prometheos-lite: https://github.com/prometheosai/prometheos-lite/issues/360
- prometheosai/foundry: https://github.com/prometheosai/foundry/issues/232
- prometheosai/soma: https://github.com/prometheosai/soma/issues/221
- diegorhoger/HIVE: https://github.com/diegorhoger/HIVE/issues/12
- prometheosai/OS: https://github.com/prometheosai/OS/issues/120
- diegorhoger/graphengine: https://github.com/diegorhoger/graphengine/issues/147
- diegorhoger/engineering-os: https://github.com/diegorhoger/engineering-os/issues/75
- diegorhoger/orbit: https://github.com/diegorhoger/orbit/issues/52
- diegorhoger/Atlas: https://github.com/diegorhoger/Atlas/issues/41
- diegorhoger/.agents: https://github.com/diegorhoger/.agents/issues/8

## Proposed project-specific decision

Lite owns governed execution, model/provider/harness adapters and authoritative work state. This is an experimental inference-control-plane proposal, not a stable alpha promise or permission to implement a tensor engine. Extend #133 (routing) and #137 (pressure/recovery). Use the existing adapter/service boundary; external engines own tensor allocation and actual placement.

Proposed flow: discover topology and backend versions → estimate per-pool demand → validate effective policy and freshness → reserve atomically → request a backend-supported placement → observe actual execution → release/reconcile → issue a receipt. Expose requested vs granted resources, device and pool IDs, checkpoint/quantization/context, reservation generation, observed residency, peak allocations, transfer bytes if available, fallback reason, cancellation and release. Retain unmeasured fields as UNKNOWN. Do not promise a placement a backend cannot expose.

For each pool, check active allocations + reservations + new request + reserve against usable capacity. Avoid double-counting shared physical storage. Estimate model bytes as parameter count × bits / 8 plus metadata and actual allocation overhead; KV for standard full attention is approximately 2 × layers × KV heads × head dimension × cached tokens × element bytes per sequence; handle different attention architectures explicitly. Include scratch, fragmentation, concurrent sessions, OS/display use and headroom. For MoE distinguish total resident weights and active experts; measure offload and transfer cost.

Compare CPU-only, single-GPU, layer split across unequal GPUs, separate workers, and UMA Metal/MLX where supported. No silent context/precision/quality decrease, cloud fallback, or device migration. The runtime must reject or offer a policy-authorized alternative. Do not treat the proposed resource fabric as a new project.

## Falsification and acceptance criteria

Stale inventory, unsupported driver/kernel, duplicate physical pool, integer overflow, reservation race, oversized KV, OOM, cancellation, lease expiry, crash/recovery, unavailable fallback, concurrent sessions, no silent external routing; compare actual allocations against estimates.

Inspect existing implementation first; report exact commands, versioned artifacts, PASS/FAIL/SKIP/UNKNOWN, limitations and independent review. A proposal or draft PR is not evidence of implementation or acceptance. Promotion requires the project's existing review and approval gates.
