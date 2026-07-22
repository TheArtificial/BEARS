# Work-surface egress allowlist implementation plan

**Status:** In progress — surface persistence, typed provider configuration, and provider-side HTTPS relay enforcement are implemented; dispatch snapshots and setup UI remain.
**Decision:** [ADR-0037 — Work sandbox, egress gateway, and multi-identity upstream auth](../decisions/adr-0037-work-sandbox-egress-gateway-and-upstream-auth.md#51-surface-owned-outbound-host-allowlist)

## Goal

Let a managed work surface explicitly permit outbound HTTPS to a small set of hostnames while keeping `work` sandboxes network-denied by default. Base images may suggest sensible setup defaults; a saved surface owns the final list.

The immediate user-visible outcome is that a Rust surface can accept `index.crates.io` and `static.crates.io`, allowing Cargo dependency resolution without granting general Internet access. A surface can also add a specific integration host such as `api.staging.example.com`.

## Current implementation boundary

The first slice is present in Den:

- `work_surfaces.allowed_outbound_hosts` persists the surface-owned list, defaulting to empty (deny egress);
- `AllowedOutboundHosts` normalizes and validates exact hostname inputs at the service/protocol boundary;
- the managed provider configuration carries the validated list and includes it in its configuration hash;
- restricted Docker sandboxes receive one internal-network DNS alias per allowed hostname, backed by a paired TCP relay that can connect only to that hostname on HTTPS/443. Unlisted names have no internal DNS answer or route.

This does **not** yet create a dedicated DNS service: Docker's internal network DNS supplies aliases for the explicitly allowed names. The relay resolves its target outside the internal sandbox network; a later gateway hardening slice must reject unsafe resolved addresses and bind TLS/SNI to the approved authority. Until then, the provided route is exact-host/443 but does not meet the full rebinding/private-address defense described below.

- General Internet access, URL allowlists, wildcard domains, CIDRs/IP ranges, arbitrary ports, or access to private/control-plane networks.
- Egress profiles, profile versioning, reusable grant objects, or per-run exception workflows.
- Supplying credentials when a hostname is allowed. Connection and gateway injection policy remains separate.
- Retroactively changing the network policy of an active sandbox.

## Implementation sequence

### 1. Persist the minimal policy

Add a forward migration for `work_surfaces` with a non-null hostname collection defaulting to an empty list. Add optional image-catalog metadata containing the hostnames an image suggests at surface setup. Derive the work surface's upstream hostname as a separate setup suggestion so the initial repository checkout remains possible. Keep all values typed in the Den repository/domain boundary rather than passing arbitrary JSON through dispatch.

Validate every stored hostname at the trust boundary:

- lowercase canonical DNS hostname only;
- no scheme, path, query, fragment, userinfo, port, wildcard, or IP literal;
- reject loopback, link-local, private, and provider/control-plane names **at resolution/enforcement time**, because a hostname alone does not reveal its resolved address;
- de-duplicate while preserving a predictable display order.

Use a single `Vec<Hostname>`-style value in application code. Do not introduce profile or grant tables. Add a migration self-check or focused repository test covering empty defaults, normalized accepted Cargo hosts, and rejected URL/IP/wildcard inputs. **Implemented for protocol validation; add the database migration case when migration tests are available.**

### 2. Surface create/read/update API and UI

Extend managed work-surface request/response types and handlers to read and replace the saved hostname list. On creation or selected-image change, return the catalog image's suggested hostnames and the upstream hostname for the client to present; require the client to save the accepted/final list explicitly. Do not auto-merge later catalog or upstream edits into existing surfaces.

The UI should use one field labelled **Allowed outbound hosts**, with image and upstream suggestions visibly marked but otherwise identical to manually added hosts. The Rust image should initially suggest `index.crates.io` and `static.crates.io`; the configured upstream host should also be offered to preserve repository materialization. Keep the help text short: “Sandboxes can use HTTPS only to these hosts.”

Authorize edits using the existing surface owner/manager rules and emit the normal surface audit/update event. Avoid a new approval or policy-management screen.

### 3. Freeze and dispatch the policy

At work-run dispatch, load the surface's saved list and place it in the typed sandbox/gateway session request. The managed provider configuration already carries the field; add the dispatch/run-level snapshot so a later surface edit cannot affect a queued or active run. Persist either the exact resolved list or a surface-policy revision on the workspace/run record so the dispatch can be audited and reproduced. Existing running sessions keep their resolved policy; surface changes affect only later sessions.

Fail closed if the surface cannot be loaded or its policy is malformed. An empty list remains valid and means no network egress. Include the resolved host count (not credentials or full request URLs) in run telemetry and recap diagnostics.

### 4. Enforce at the paired gateway

Make the workspace peer use only the paired gateway for DNS and outbound HTTPS; **DNS must be explicitly provided to the sandbox through that controlled resolver, not merely unblocked.** The gateway must:

1. expose a sandbox DNS resolver (for example, via the sandbox's `resolv.conf`) that answers only exact allowed names, forwards those queries through the gateway's controlled resolver, and returns denial/NXDOMAIN for all others;
2. resolve each allowed name through that controlled resolver and reject loopback, link-local, private, multicast, link-local IPv6, and provider/control-plane destination addresses;
3. allow TCP/TLS only for resolution results associated with the requested allowed hostname on port 443;
4. reject direct IP connections, hostname aliases not on the list, and all other outbound protocols/ports;
5. re-check the hostname/SNI/HTTP authority as applicable before forwarding, binding the connection to the checked DNS answer to prevent DNS rebinding;
6. preserve the existing credential-injection rules as an independent, narrower authorization layer.

Test both the intended Cargo path and denial cases: an empty list, an unlisted hostname, direct IP egress, a non-443 port, and a hostname resolving to a prohibited address. Add the smallest runner/gateway integration check that proves an allowed host resolves/connects while an unlisted host does not.

### 5. Rollout and validation

1. Seed catalog suggestions for the existing `rust` image only.
2. Release the schema/API/UI before turning on strict runtime enforcement for existing surfaces, so owners can save required hosts.
3. Enable default-deny enforcement, with empty legacy surfaces deliberately receiving no egress until configured.
4. Verify a Rust surface accepting the defaults can run a clean Cargo metadata or dependency-fetch command through the gateway.
5. Verify a surface with an added staging API hostname can reach only that API over HTTPS and still cannot reach another public host.
6. Review audit records and denial telemetry for missing legitimate hosts; update the individual surface rather than widening a global rule.

## Affected areas

- `services/den/migrations/`: managed work-surface and sandbox image catalog schema migration.
- Den work-surface persistence/API/UI and their typed request models.
- Work dispatch/workspace records and the runner RPC contract.
- `bears-sandbox-runner` paired gateway DNS/TLS/connection enforcement and its integration tests.
- Sandbox image catalog seeds for the Rust suggestions.

## Acceptance criteria

- A new surface with no accepted hosts has no DNS answers and no outbound network access.
- The Rust image offers, but does not silently grant, `index.crates.io` and `static.crates.io`.
- A manager can add `api.staging.example.com` on the same surface list; it is enforced as HTTPS/443 only.
- An image catalog change cannot change an existing surface's egress policy.
- A policy edit affects future dispatches only; each run has an auditable resolved policy.
- Allowed-host routing never exposes secrets in the workspace and is not a substitute for Connection-based credential authorization.
