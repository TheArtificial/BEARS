# Den Crate Split and Rust Idiom Refactor Plan

**Status:** Complete. This roadmap is retained only as a historical pointer and can be deleted once downstream references are updated.

The Den crate split, Rust idiom refactor, edge extraction, runtime dependency diet, protocol-agnostic cleanup, and legacy ACP adapter removal have all landed. Durable architecture notes and lessons from this plan were moved to:

- [Den crate architecture](../architecture/den-crate-architecture.md)
- [Den-native runtime](../architecture/den-native-runtime.md)
- [Clients, channels, armatures, and adapters](../guides/clients-channels-armatures.md)
- [ADR-0043 — ACP is an edge adapter; the Den runtime is protocol-agnostic](../decisions/adr-0043-acp-as-edge-adapter-protocol-agnostic-core.md)

## Final Outcome

- The `den` binary remains the single deployable process and composition root.
- Workspace crates now separate stable leaves, shared services, runtime execution, and HTTP edges.
- `den-core::tools` owns the model-facing Den tool surface; concrete Den tool executors remain binary-local under `src/core/tools`.
- `RuntimeToolInvoker` keeps the runtime dependency-inverted from concrete builtin tool execution.
- `/internal/den-tools/invoke` lives in the binary (`src/internal_tools.rs`) to avoid pulling concrete tool execution into lower or sibling crates.
- `den-service` owns shared app state (`DenState`) and concrete services below the edges.
- `den-protocol` owns stable runtime DTOs/contracts.
- `den-acp` was removed; BearWire is the armature edge, and Den core/service/runtime/API code no longer carries ACP-specific naming.
- `ACP_GATEWAY_ENABLED` and `/acp` route mounting were retired.
- `den-core`/`den-db` was evaluated and deferred as low-value; see the architecture note for rationale.
- Native async fns replaced `async-trait` on non-`dyn` internal service traits; strict clippy gating is in place with a curated allow-list.

## If Deleting This File

Before deleting this roadmap, update or remove any remaining links to it. The intended replacement for future readers is [den-crate-architecture.md](../architecture/den-crate-architecture.md), not this historical plan.
