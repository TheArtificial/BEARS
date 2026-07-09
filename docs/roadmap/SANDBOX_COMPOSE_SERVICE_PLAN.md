# Plan: distinct sandbox services in docker-compose (with docker included)

Goal: make the work-sandbox system deployable as part of the compose stack —
dedicated sandbox services **bundling their own docker**, so a Coolify
deploy of this repository yields a working end-to-end `work` flow with no
bare-metal provider step. This replaces the current `sandbox` compose
profile (Den image, no docker CLI, host `docker.sock` mount) entirely;
compose gets exactly one way to run sandboxes. The bare-metal provider on a
dedicated sandbox host remains the documented **non-compose** alternative.

## Architecture

Two services under the `sandbox` profile — the provider manages, the engine
executes:

```
bears-sandbox-provider   RUN_SANDBOX API + docker CLI (no daemon)
   │  DOCKER_HOST=tcp://bears-sandbox-engine:2376 (TLS)
   ▼
bears-sandbox-engine     docker:dind (privileged) — the Docker Engine that
   │                     hosts everything sandbox-shaped:
   ├─ den-sbx-<id>           sandbox containers
   ├─ den-sbx-gw-<id>        egress relays
   └─ den-sbx-net-<id>       internal networks
```

Why a dedicated engine instead of the host daemon:

- **Containment**: `docker.sock` is host-root-equivalent; a privileged dind
  container is root over *its own* world only. Sandboxes, networks, images,
  and disk all live in the engine's volume — `docker compose down -v
  --profile sandbox` removes every trace, and redeploys leak nothing onto
  the deploy host.
- **No host-path coupling**: provider and engine share one named workspaces
  volume mounted at the **same path in both**, so bind sources resolve
  without `SANDBOX_WORKSPACES_HOST_DIR` (the engine *is* the daemon
  resolving them). The Coolify `${…}`-in-volume-target problem disappears.

Why two services rather than one container running both: `docker:dind` is a
maintained image that already handles storage-driver/cgroup/iptables/TLS
bootstrapping; merging would mean supervising two processes in a custom
image and coupling their lifecycles — a provider restart would kill the
daemon and every running sandbox, where the split lets a restarted provider
**re-adopt** still-running sandboxes from labels (a designed-in recovery
path).

Trade-offs, stated honestly: the engine is `privileged`; nested containers
add a storage/network hop; sandbox images must be made available *inside*
the engine (step 4).

## Steps

### 1. Provider image (`packaging/sandbox-provider/Dockerfile`)

- `FROM` the built Den image (build arg `DEN_IMAGE`, so compose reuses the
  image it already built) + the docker CLI (distro package or static
  tarball). No daemon, no socket handling — `DOCKER_HOST` is TCP, so the
  `appuser` drop in `docker-entrypoint.sh` keeps working unchanged.
- Health: the existing wget check against `/sandbox/v1/health`; its
  `backend_available` field now reflects engine reachability.

### 2. Compose services (profile `sandbox`)

```yaml
bears-sandbox-engine:
    image: docker:27-dind
    privileged: true
    profiles: ["sandbox"]
    environment:
        DOCKER_TLS_CERTDIR: /certs
    volumes:
        - bears-sandbox-engine-data:/var/lib/docker
        - bears-sandbox-certs:/certs
        - bears-sandbox-workspaces:/var/lib/bears/sandbox-workspaces
    healthcheck: docker version against the daemon

bears-sandbox-provider:
    build: packaging/sandbox-provider (DEN_IMAGE build arg)
    profiles: ["sandbox"]
    depends_on: bears-sandbox-engine (healthy), preflight
    environment:
        RUN_SANDBOX: "true"
        DOCKER_HOST: tcp://bears-sandbox-engine:2376
        DOCKER_TLS_VERIFY: "1"
        DOCKER_CERT_PATH: /certs/client
        SANDBOX_WORKSPACES_DIR: /var/lib/bears/sandbox-workspaces
        SANDBOX_ROOTS_CONFIG: /etc/bears/sandbox-roots.json
        SANDBOX_SERVICE_TOKEN / SANDBOX_PORT / limits as today
    volumes:
        - bears-sandbox-certs:/certs:ro
        - bears-sandbox-workspaces:/var/lib/bears/sandbox-workspaces
        - ${SANDBOX_ROOTS_FILE:-./data/sandbox-roots.json}:/etc/bears/sandbox-roots.json:ro
```

Plus named volumes `bears-sandbox-engine-data`, `bears-sandbox-certs`,
`bears-sandbox-workspaces`. The old `bears-sandbox` service definition (host
socket + host-dir bind) is **removed** — compose has one sandbox story.
`SANDBOX_WORKSPACES_HOST_DIR` stays in the provider code (it is what makes
bind-source mapping explicit and testable, and non-compose containerized
setups may still need it) but no compose wiring uses it.

Backend code change: none — `DockerCliBackend` shells to the CLI, which
honors `DOCKER_HOST`/TLS env as-is; only `probe()`'s failure message should
mention `DOCKER_HOST` alongside the socket.

### 3. Callback DNS for nested containers (small code change)

Containers nested inside the engine do not resolve compose service names
(their DNS chain ends at the engine, not the compose resolver), so
`SANDBOX_CALLBACK_API_URL=http://bears-den:3001` works for the provider but
not for relays/sandboxes. Fix provider-side, once, for all nested setups:

- At provision time, resolve the callback host to an IP
  (`tokio::net::lookup_host` — the provider *can* resolve compose names) and
  pass `--add-host <host>:<ip>` to the relay (restricted mode) or the
  sandbox container (open mode). Skip for IP literals.
- This also removes the current `host.docker.internal` caveat for
  single-host dev.

### 4. Sandbox images inside the engine

The catalog images must exist in the **engine's** store. Two mechanisms:

- **Registry pull (production default)**: CI publishes
  `ghcr.io/<owner>/bears-sandbox{,-rust,-node,-godot}` (new workflow step
  reusing the existing GHCR credentials); the roots-file catalog references
  those; the engine pulls on first use. Mount a registry `config.json` into
  the engine when the packages are private.
- **One-shot builder (dev / air-gapped)**: `bears-sandbox-images` compose
  service (profile `sandbox-build`): docker CLI image, repo mounted,
  `DOCKER_HOST` pointed at the engine, running
  `scripts/build-sandbox-image.sh all`. `restart: "no"`, run manually or as
  a deploy hook.

### 5. Wire Den to it

Compose defaults (den/worker env):
`SANDBOX_SERVER_URL=http://bears-sandbox-provider:3002`,
`SANDBOX_CALLBACK_API_URL=http://bears-den:3001` (http — satisfies
restricted mode), tokens shared via one env var. Update
`services/den/README.md` and `docs/guides/sandbox-server-ops.md` to describe
the profile and drop the "needs a docker CLI" ponytail.

### 6. Verification

- `docker compose --profile sandbox up -d` on a clean host: health green,
  `/sandbox/v1/catalog` lists the images, `docker -H tcp://…engine ps`
  empty until a dispatch.
- `scripts/work-e2e.sh` variant pointed at the compose provider (root added
  to the mounted roots file) — the full job → sandbox → publish walk.
- Egress check inside a nested sandbox: external `curl` fails, Den callback
  succeeds. Redeploy check: `compose down && up` re-adopts running
  sandboxes (engine volume persists) and leaks nothing project-external.
- Coolify: deploy with `COMPOSE_PROFILES=sandbox`; validator accepts the
  file (no `${…}` volume targets anywhere).

## Effort and sequencing

Roughly 2–3 days: image + compose (1), callback DNS change + tests (½–1),
CI image publishing (½), docs + e2e verification (½–1). No schema changes;
no provider API changes. Sequencing: steps 1–2 land together behind the
profile; 3 is independently useful (fixes `host.docker.internal` dev pain)
and can ship first.

## Rejected alternatives, for the record

- **Host socket + docker CLI in the Den image** (the current profile's
  shape): less work (~½ day) but leaves host-root-equivalent access in a
  deployed service, leaks sandbox containers outside the compose project's
  lifecycle, and keeps the host-path coupling that broke the Coolify
  deploy. Removed rather than kept as a variant; hosts where `privileged`
  is forbidden should run the bare-metal provider instead.
- **One combined provider+daemon container**: trades a compose service for
  a custom two-process image and couples provider restarts to sandbox
  survival; see "why two services" above.
