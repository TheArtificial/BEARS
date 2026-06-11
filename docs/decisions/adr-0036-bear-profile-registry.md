# ADR-0036 — Bear profile registry and binding vocabulary

**Status:** Accepted (2026-06-09)  
**Related:** [ADR-0035 — Den-native in-process agent runtime](../decisions/adr-0035-den-native-in-process-agent-runtime.md), [`bear-roles.md`](../architecture/bear-roles.md), [`interactive-profiles-and-role-axes.md`](../architecture/interactive-profiles-and-role-axes.md)

## Context

Bear Den historically modeled capability-specific runtimes as **roles** backed by Letta agents (`bear_agents`, `letta_agent_id`). Native runtime (ADR-0035) already provisions stable ids as `den-native:{bear_id}:{profile}` but the schema and APIs still treated Letta agent ids as primary.

Product language also settled on **`chat`** (not `talk`) and **`curate`** (not `review`) for the integrator profile name.

## Decision

1. **Vocabulary**
   - **Profile** — one of `chat`, `pair`, `curate`, `work`, `watch` (capability + trust boundary).
   - **Memory branch** — logical path prefix (`chat/`, `pair/`, `core/`, …); unchanged.
   - **Binding** — Den-owned runtime identity for a profile on a Bear (`binding_id`).

2. **Schema**
   - Rename `bear_agents` → `bear_profile_bindings`; column `role` → `profile`.
   - Add required `binding_id TEXT`; backfill from `letta_agent_id` or `den-native:{bear_id}:{profile}`.
   - Keep nullable `letta_agent_id` for legacy Letta-only escape hatch (`AGENT_RUNTIME=letta`).
   - Rename `prompt_memory_blocks.role_slug` → `profile_slug`.
   - Rename `bear_skills_manifest.applies_to_roles` → `applies_to_profiles`.

3. **Rust / API**
   - `BearAgentRole` → `BearProfile`; `BearAgent` → `BearProfileBinding`.
   - Public JSON uses `profile` / `profile_slug` / `binding_id` (no `role` alias for capability profiles).
   - Do **not** rename `user_bear.role` (membership) or message `role` (user/assistant).

4. **Aliases removed**
   - No `talk` profile slug in parsers or CHECK constraints.
   - No `role_agent_id` / `role_runtime_binding_id` compatibility shims.

## Consequences

- Breaking change for clients reading bear capability as `role` or tool context as `agent_role` / `role_agent_id`.
- Letta provisioning continues behind `AGENT_RUNTIME=letta`; native path writes `binding_id` and leaves `letta_agent_id` NULL.
- Phase 7 sandbox and upstream auth: [ADR-0037](adr-0037-work-sandbox-egress-gateway-and-upstream-auth.md).
- Phase 7 may rename remaining `owner_role` / `source_role` columns on work-plan and memory-proposal tables for full consistency.
