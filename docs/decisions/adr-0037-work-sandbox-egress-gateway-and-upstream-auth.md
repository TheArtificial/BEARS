# ADR-0037 — Work sandbox, egress gateway, and multi-identity upstream auth

**Status:** Accepted (2026-06-09)  
**Deciders:** Hans  
**Related:**
- [ADR-0035 — Den-native in-process agent runtime](adr-0035-den-native-in-process-agent-runtime.md)
- [ADR-0036 — Bear profile registry](adr-0036-bear-profile-registry.md)
- [ADR-0034 — Jobs and tasks (Docket)](adr-0034-jobs-and-tasks-work-management.md)
- [ADR-0006 — Bear work surfaces](adr-0006-bear-work-surfaces.md)
- [ADR-0028 — Environment affordance and resource boundaries](adr-0028-environment-affordance-and-resource-boundaries.md)
- [`interactive-profiles-and-role-axes.md`](../architecture/interactive-profiles-and-role-axes.md)
- [`work-surfaces-and-conversations.md`](../guides/work-surfaces-and-conversations.md)
- [`den-native-runtime.md`](../architecture/den-native-runtime.md)
- [`DEN_NATIVE_RUNTIME_PLAN.md`](../roadmap/DEN_NATIVE_RUNTIME_PLAN.md) Phase 7

## Context

Phase 7 replaces Codepool and Letta Code with a Den-native **`work`** execution path: shell, filesystem, and filtered network inside an isolated environment, while the agent loop stays in-process in Den ([ADR-0035](adr-0035-den-native-in-process-agent-runtime.md)).

Design discussions settled several product constraints:

- **`chat` never gets a sandbox.** When execution armature is required, `chat` delegates to a **`work`** run (typically via Docket). The chat channel streams phase events so UX is better than a blocking “please wait.”
- **`pair` defaults to client armature** (ACP client tools, local workspace). Server-hosted checkout for mobile/web is the same sandbox machinery with different profile policy — deferred to Phase 7.1.
- **Cold start is acceptable in v1.** No warm pool required at launch; **telemetry from day one** on sandbox acquire, git materialize, and first exec.
- **Git upstreams are Den-managed** at bear level (origins UI, auth to GitHub / GitLab / Gitea). Legacy `memfs_repo_path` is not the long-term checkout source of truth.
- **Upstream auth is inherently multi-actor.** A single token per sandbox is wrong for UX and security. The desired default on GitHub: the **Bear service identity commits**, the **requesting human opens the PR** — without exposing multiple real tokens inside the sandbox.
- **Arbitrary repos must work without Bear-specific scaffold.** Users configure a remote origin and auth in Den; the runner shallow-clones and discovers tooling from whatever the repo already contains. No required `mise.toml`, repo manifest, or local checkout on the operator machine.

**External references:**

- **[DAM](https://github.com/dam-agents/dam)** — paired gateway pods, credential injection at egress, Connections scoped by owner. Bears adopts the structural pattern on Docker Compose, adapted to Bear profiles, Docket runs, and bear-level service identities.
- **[Locki](https://github.com/janpokorny/locki)** — Incus system containers inside a VM boundary, git/gh **command bridge** with allowlisted operations, parallel sandboxes without port/tag collisions, optional repo-level image hints. Locki targets **local dev**: harness runs inside the sandbox, worktrees live on the host, and it explicitly defers exfiltration protection to DAM. Bears takes Locki’s **isolation and git-policy** ideas, not its local-worktree or harness-in-box workflow.

## Decision

### 1. Sandbox scope by profile

| Profile | Den-managed sandbox in Phase 7 | Notes |
|---------|-------------------------------|--------|
| **`work`** | **Yes** | Docket dispatch → sandbox session bound to `(bear_id, run_id)` + work surface |
| **`chat`** | **No** | Delegates to `work` when shell/fs/network execution is needed |
| **`pair`** | **No** (default) | Client armature via ACP; Den loop only |
| **`pair` (hosted)** | Phase 7.1 | Same runner + gateway; conversation-scoped binding; interactive approvals |
| **`curate` / `watch`** | **No** | Narrow in-process tool rosters only |

### 2. `bears-sandbox-runner` and pluggable isolation backends

Add a compose service **`bears-sandbox-runner`** (name may be shortened in compose labels) that owns **execution substrate**, not control-plane policy.

The runner exposes a **`SandboxBackend`** abstraction. Den’s `SandboxManager` calls a stable RPC surface; the backend chooses how to isolate:

| Backend | Phase | Role |
|---------|-------|------|
| **`docker_workspace`** | **7 (v1)** | Paired workspace + egress gateway containers on an isolated Docker network. Sufficient for clone, build, test, and shell/fs on typical repos. |
| **`incus_system`** | **7.2+** | Incus (or equivalent) **system container** per session — full init, nested containers, local k8s/k3s, systemd. For repos that need “a machine,” not an app container. |

**Why two backends:** [Locki’s Incus thesis](https://github.com/janpokorny/locki) is correct that thin OCI app containers break down on multi-service stacks, in-sandbox Docker, and Kubernetes verification. Bears does not adopt Locki’s Lima+local-worktree stack, but **Incus as an optional runner backend** preserves support for arbitrary real-world repos without forcing every run through Docker-in-Docker in v1.

**Runner responsibilities (all backends):**

- Create/teardown **paired execution + egress gateway** instances per sandbox session
- Materialize **run workspaces** (git clone/checkout) from instructions issued by Den
- Execute **fs** and **shell** RPCs with path jail and resource limits
- Emit **timing telemetry** back to Den (`queued_at`, `instance_started_at`, `git_started_at`, `git_finished_at`, `ready_at`, byte counts, error class, `backend_kind`)

**Den responsibilities (`SandboxManager` client):**

- Decide **when** to acquire/release sessions (aligned with Docket run lifecycle and turn `CancellationToken`)
- Select **`sandbox_profile`** for the work surface / run (see §5)
- Resolve **work surface → origin** and build **`RunAuthContext`** (see §8)
- Push **gateway policy** (egress rules, injection map, git-bridge rules, HITL hooks) per session
- Store **Connections**, secrets, bear service identities, and operator UI
- Stream **delegation/phase events** to `chat` and run detail surfaces

**Ownership rule:** Den owns the **contract** (origins, credentials, policy, audit). The runner owns **materialized bits** (cloned trees, instance state, optional future bare mirrors). Operator configuration of “which GitHub repo is BEARS” lives only in Den.

**Parallel runs:** Multiple concurrent Docket runs on one Bear each get an isolated session (network, branch namespace, ports). This is a first-class requirement — one lesson taken directly from Locki.

### 3. Paired workspace + egress gateway (structural boundary)

Each sandbox session comprises **two isolation peers** on a dedicated network (implemented as Docker containers in v1; Incus instance + gateway sidecar in 7.2+):

1. **Workspace instance** — writable run tree, local git/fs/shell; **no route to TCP 80/443 except the paired gateway**
2. **Egress gateway** — sole path to upstream HTTPS; **credential injection on the wire**; **git/gh command bridge** (§4); optional **ext_authz** callback to Den for HITL

The workspace instance must not receive real upstream tokens in environment variables. Placeholders (e.g. `{{GITHUB_BEAR_PUSH}}`) may appear in git config, but injection happens only in the gateway — matching DAM [ADR-038](https://github.com/dam-agents/dam/blob/main/docs/adrs/038-paired-gateway-pod.md) (paired boundary, not cooperative `HTTPS_PROXY` inside a shared network namespace).

Den must not execute arbitrary user/agent code in the `bears-den` process for `work` turns. The **native agent loop stays outside** the sandbox; only tool execution (shell, fs, bridged git/gh) runs inside — unlike Locki, which runs the harness in the sandbox.

### 4. Git/gh command bridge (Locki-inspired, Den-policy-driven)

Relying on `HTTPS_PROXY` or a generic credential helper alone is insufficient. Adopt Locki’s **command bridge** pattern, implemented at the **egress gateway** (and/or as workspace shims that RPC to it):

- `git` and `gh` inside the workspace are **shims** forwarding to the gateway
- The gateway validates each invocation against an **allowlisted grammar** of subcommands and flags (Locki publishes a similar filter in its sandbox `AGENTS.md`)
- Policy is applied from **`RunAuthContext`** (§8), not from repo layout:
  - **Branch namespace** — pushes only to configured prefixes (e.g. `bear/{bear_slug}/{run_id}`), not Locki’s `#locki-<id>` suffix (same idea, Bears naming)
  - **Operation class** — `read`, `push`, `pr_create` map to actor + credential injection
  - **Scope** — current origin/repo only unless work surface grants broader access

Prefer **Den-brokered tools** (e.g. `github.open_pull_request`) for PR creation when HITL and audit matter; the bridge covers harness-agnostic `git`/`gh` usage for arbitrary repos.

Direct unbridged `git push` over HTTPS from the workspace without passing the bridge is **forbidden** in v1.

### 5. Arbitrary repos, origins, and sandbox profiles

**Access model (more general than Locki for cloud users):** Locki assumes the operator `cd`s to a **local** git repo and gets a host worktree. Bears assumes the operator configures a **remote origin** in Den; the runner **shallow-clones** into the session workspace. No local checkout required; no repo-specific Bear config required to start.

**Tooling discovery (opportunistic, not prescriptive):**

- The `work` profile **detects** toolchain hints from the repo (`mise.toml`, `.tool-versions`, `.nvmrc`, `pyproject.toml`, `package.json`, CI workflows, `README`, …) and runs appropriate install commands.
- **No repo file is required.** Absence of `mise.toml` must not fail materialization or block the run.
- Base workspace images may include common tools (git, gh, curl, build essentials, **optional mise**) as convenience, not as contract.

**Optional work-surface hints** (Locki’s optional `locki.toml` pattern, Bears-owned):

```text
work_surface.sandbox_profile ∈ { minimal, full_os }   // default minimal
work_surface.base_image_ref   // optional override
work_surface.resource_limits    // optional CPU/mem/disk
```

- **`minimal`** — `docker_workspace` backend; fast cold start.
- **`full_os`** — routes to `incus_system` when available (7.2+); for surfaces that routinely need in-sandbox Docker, compose, or k3s.

**Sandbox briefing:** Den injects a short **environment appendix** into the `work` turn context (cf. [ADR-0028](adr-0028-environment-affordance-and-resource-boundaries.md)): workspace root, branch policy, how push/PR actors differ, run id for observability, and explicit guidance to **detect tooling from the repo — do not assume mise or any single package manager**.

### 6. Bear-level origins

Den exposes operator UI and APIs to configure **bear-level upstream origins**:

- Provider kind: GitHub, GitLab, Gitea (extensible)
- Canonical remote URL / org-repo identity
- Default branch and branch namespace conventions (e.g. `bear/{bear_slug}/{run_id}`)
- Binding from **work surface slug** → **origin id**
- Optional **sandbox profile** fields (§5)

Auth attachments reference **Connections** (§7), not raw secrets in origin rows.

Legacy MemFS git worktrees and `memfs_repo_path` may be used only during migration; new work surfaces use Den-managed origins.

### 7. Connections and Contributions

Adopt a DAM-shaped **Connection** model in Den Postgres (exact table names are implementation detail):

```text
Connection.owner ∈ { user_id, bear_id }
Connection.template ∈ { github, gitlab, gitea, … }
Connection.auth     → encrypted secret refs (OAuth refresh, app credentials, PAT)
Connection.grants   → bear, work_surface, or run-scoped attachment
```

**Contribution kinds** (realized into gateway policy and/or runner config):

| Kind | Purpose |
|------|---------|
| `egress-host` | Allowlisted upstream host + path patterns |
| `credential-injection` | Map placeholder → secret ref + match rules (host, method, path) |
| `git-identity` | Committer name/email; optional signing key ref (gateway-only) |
| `operation-policy` | Which actor applies to `read`, `push`, `pr_create`, … |
| `git-bridge-rule` | Allowlisted git/gh subcommand patterns for the command bridge (§4) |

Secrets are stored in Den (encrypted at rest); gateway instances receive **short-lived lease material** or mount refs scoped to the session — never copied into the workspace instance.

### 8. Bear service identity (GitHub App and machine user)

Each Bear may have at most one **primary GitHub service identity** for autonomous git write, configured as either:

| Mode | Intended deployment | Configuration |
|------|---------------------|---------------|
| **`github_app`** | Public / hosted Den | GitHub App installation on org/user; installation id + app credentials in Den; fine-grained repo access |
| **`machine_user`** | Self-hosted Den | Dedicated GitHub user (bot account) + PAT or OAuth; documented seat/licensing expectations for operators |

Both modes are **first-class** in schema and UI (`identity_kind` discriminant). Policy, gateway injection, and the git bridge treat them identically at the **`push`** operation class — only acquisition and admin UX differ.

Product copy should reflect the mode (“Installed GitHub App” vs “Bear GitHub account @bears-bruno”) without branching the execution model.

**Requester identity** remains a separate Connection owned by `user_id` (human OAuth link). A single run may therefore draw on **two Connection owners** without merging tokens.

### 9. `RunAuthContext` and operation-scoped injection

Every **`work`** run (including those spawned from `chat` delegation) carries a **`RunAuthContext`** resolved at dispatch:

```text
RunAuthContext {
  bear_id
  run_id                      // Docket bear_job_run / task run
  work_surface_ref
  requester_user_id           // nullable for purely autonomous work
  origin_id
  bear_service_identity_id    // nullable until configured
  sandbox_profile             // minimal | full_os
  operations: {
    read:   ActorSelection
    push:   ActorSelection
    pr_create: ActorSelection
  }
}
```

**ActorSelection** resolves to a Connection + injection profile:

```text
ActorSelection ∈ {
  bear_service_identity,
  requester_user,
  operator_connection,   // install-wide fallback
  denied
}
```

**Default GitHub policy** (overridable per work surface in a later phase):

| Operation class | Default actor | Rationale |
|-----------------|---------------|-----------|
| **`read`** (clone/fetch) | First satisfied: bear app/install read → requester OAuth read → operator connection | Materialize workspace |
| **`push`** (commit to `bear/*` or configured prefix) | **Bear service identity** | Stable author, auditable bot identity |
| **`pr_create`** | **Requester user** (after HITL or pre-consent at dispatch) | Human accountability on review/merge |
| **`merge`** | **Denied** in v1 | Explicit human action outside autonomous loop |

If `push` cannot resolve (no bear service identity), dispatch **fails fast** with setup UX (“Connect Bear to GitHub”). If `pr_create` is missing, the run may complete through push and **pause** at PR time with a resumable “Connect GitHub to open PR” state — not a silent failure.

The gateway selects credentials from **`RunAuthContext.operations`** using request metadata (host, HTTP method, path, operation class from Den-brokered tools or git-bridge callbacks). The agent loop and workspace do not choose tokens.

### 10. `chat` → `work` delegation and observability

When `chat` requires execution:

1. Den creates or resumes a Docket **`work`** run with `requester_user_id` from the chat session.
2. `chat` returns immediately with a **correlation id** (`run_id`).
3. Den emits **phase events** on the chat SSE/web channel and run detail UI, for example:

   ```text
   dispatch.created
   sandbox.queued
   workspace.materializing
   workspace.ready
   work.turn.started
   work.tool.*
   work.git.pushed        // author=bear identity, branch=…
   work.github.pr.ready   // will_open_as=requester (awaiting approval)
   work.github.pr.opened
   work.turn.completed
   ```

4. Cold-start timings are recorded on every session for later warm-pool and backend tuning.

### 11. Phase scope

| Phase | Deliverables |
|-------|----------------|
| **7** | `bears-sandbox-runner` with **`docker_workspace`** backend; paired gateway; git/gh command bridge; Den origins + Connections; bear service identity (app + machine user); `RunAuthContext`; opportunistic tooling detection; sandbox briefing; `work` + Docket; chat delegation + phase SSE; cold start + telemetry |
| **7.1** | Hosted **`pair`** (conversation-scoped sessions, channel approvals) on same runner |
| **7.2+** | **`incus_system`** backend for `full_os` work surfaces; bare mirror cache / warm pool if telemetry warrants; configurable per-surface policy; fork-style “run entirely as requester” for personal repos |

### 12. Compose change

Phase 7 implementation **requires** adding `bears-sandbox-runner` to `docker-compose.yaml`. That edit remains subject to explicit approval per repository rules when implementation lands. Incus/Lima dependencies for 7.2+ are runner-host concerns and need not appear in the default dev compose stack.

### 13. Explicit non-adoptions from Locki

To preserve arbitrary-repo and cloud-hosted use cases, Bears **does not** adopt:

| Locki pattern | Bears choice |
|---------------|--------------|
| Local git worktree on operator host | Remote clone in runner workspace |
| Harness (Claude Code, etc.) inside sandbox | Den native loop outside; tools only inside |
| Required `mise.toml` / mise-first startup | Opportunistic detection; mise optional in base image |
| Open network; no credential gateway | Paired gateway + `RunAuthContext` (DAM model) |
| Human commits from host IDE as primary flow | Autonomous `work` + bear push / requester PR |
| `#locki-<id>` branch suffix | `bear/{slug}/{run_id}` (or configured prefix) |

## Consequences

### Positive

- Replaces Codepool’s long-lived harness with a **structurally enforced** isolation and credential boundary.
- Supports hosted multi-tenant GitHub (**App**) and self-hosted (**machine user**) without diverging execution logic.
- **Commit/PR split** matches team UX expectations while keeping tokens out of the workspace.
- **`chat` stays simple**; execution complexity lives under `work` with visible progress.
- **Arbitrary remotes** work without local checkout or repo-specific Bear configuration.
- **Git command bridge** gives Locki-grade git safety without Locki’s local-dev coupling.
- Pluggable backends allow **v1 simplicity** (Docker) and a **credible path** to Incus for heavy repos.
- Telemetry enables data-driven warm-pool and backend investment later.

### Negative / tradeoffs

- Two isolation peers per active run increases resource use vs a monolithic sandbox.
- Gateway + multi-identity policy + git bridge is more moving parts than a single PAT in env.
- GitHub App onboarding is heavier for self-hosters; machine user docs and warnings are required.
- Path from legacy MemFS checkouts needs a migration story (Phase 8 or parallel backfill).
- `incus_system` backend adds operational complexity (Incus on runner hosts) — deliberately deferred.

## Non-goals (Phase 7)

- Warm workspace pool (v1 cold start only).
- **`incus_system`** backend (Phase 7.2+).
- Hosted **`pair`** (Phase 7.1).
- Autonomous merge to protected branches.
- Kubernetes NetworkPolicy / Istio (Docker Compose network isolation is sufficient for v1).
- Requiring **`mise.toml`** or any repo manifest for sandbox materialization.
- Local host worktrees as the primary workspace model (optional future DX for self-host adjacency, not cloud default).
- Running external coding harnesses inside the sandbox.
- Replacing Den’s existing OAuth provider for **Den login** — upstream Connection OAuth is a separate concern, though it may reuse similar storage patterns.
- Full CaMeL / quarantined-LLM architecture for prompt injection (acknowledged open problem; mitigate via egress allowlists, git bridge, and HITL on `pr_create` and sensitive operations).

## References

- DAM architecture: [security model](https://github.com/dam-agents/dam/blob/main/docs/strategy/security-model.md), [paired gateway ADR-038](https://github.com/dam-agents/dam/blob/main/docs/adrs/038-paired-gateway-pod.md), [connections](https://github.com/dam-agents/dam/blob/main/docs/architecture/connections.md)
- Locki: [README](https://github.com/janpokorny/locki), [sandbox AGENTS.md (git bridge filters)](https://github.com/janpokorny/locki/blob/main/src/locki/data/AGENTS.md)
- Meta [**Agents Rule of Two**](https://ai.meta.com/blog/practical-ai-agent-security/)
- Simon Willison, [*The lethal trifecta for AI agents*](https://simonwillison.net/2025/Jun/16/the-lethal-trifecta/)
