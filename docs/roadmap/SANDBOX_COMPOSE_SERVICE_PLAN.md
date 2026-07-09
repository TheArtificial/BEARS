# Plan: distinct sandbox service in docker-compose (with docker included)

Goal: make the work-sandbox system deployable as part of the compose stack —
a dedicated `bears-sandbox` service **with its own docker**, so a Coolify
deploy of this repository yields a working end-to-end `work` flow with no
bare-metal provider step. Today the `sandbox` compose profile reuses the Den
image (which has no docker CLI) and mounts the host socket; this plan
replaces that.

## Recommended architecture: provider + dedicated dind daemon

Two new services (profile `sandbox`), so sandboxes never touch the host
docker daemon and the whole system lives and dies with the stack:

```
bears-sandbox           provider (RUN_SANDBOX) + docker CLI
   │  DOCKER_HOST=tcp://bears-sandbox-dockerd:2376 (TLS)
   ▼
bears-sandbox-dockerd   docker:dind (privileged), own image store & nets
   ├─ den-sbx-<id>          sandbox containers (nested)
   ├─ den-sbx-gw-<id>       egress relays (nested)
   └─ den-sbx-net-<id>      internal networks (nested)
```

Why dind over the current host-socket approach:

- **Containment**: `docker.sock` is host-root-equivalent; a privileged dind
  container is root over *its own* world only. Sandbox containers, networks,
  images, and disk are all inside the dind service's volume — `docker
  compose down -v --profile sandbox` removes every trace.
- **No host-path coupling**: provider and dind share one named volume for
  workspaces mounted at the **same path in both**, so bind sources resolve
  without `SANDBOX_WORKSPACES_HOST_DIR` (dind *is* the "host" daemon). The
  Coolify `${…}`-in-volume-target problem disappears entirely.
- **Coolify-friendly**: everything is project-scoped; no orphaned sibling
  containers on the deploy host after a redeploy.

Trade-offs, stated honestly: dind is `privileged`; nested containers add a
storage/network hop (build/pull happens inside dind's volume); sandbox
images must be made available *inside* dind (step 4).

## Steps

### 1. Provider image (`packaging/sandbox-provider/Dockerfile`)

- `FROM` the built Den image (build arg `DEN_IMAGE`, so compose reuses the
  image it already built) + `apt-get install docker-cli` (or the static
  docker CLI tarball for slim images). No daemon, no socket handling —
  `DOCKER_HOST` is TCP, so the `appuser` drop in `docker-entrypoint.sh`
  keeps working unchanged.
- Health: reuse the existing wget check against `/sandbox/v1/health`; the
  health payload's `backend_available` now reflects dind reachability.

### 2. Compose services (profile `sandbox`)

```yaml
bears-sandbox-dockerd:
    image: docker:27-dind
    privileged: true
    profiles: ["sandbox"]
    environment:
        DOCKER_TLS_CERTDIR: /certs
    volumes:
        - bears-sandbox-docker:/var/lib/docker
        - bears-sandbox-certs:/certs
        - bears-sandbox-workspaces:/var/lib/bears/sandbox-workspaces
    healthcheck: docker version against the daemon

bears-sandbox:
    build: packaging/sandbox-provider (DEN_IMAGE build arg)
    profiles: ["sandbox"]
    depends_on: bears-sandbox-dockerd (healthy), preflight
    environment:
        RUN_SANDBOX: "true"
        DOCKER_HOST: tcp://bears-sandbox-dockerd:2376
        DOCKER_TLS_VERIFY: "1"
        DOCKER_CERT_PATH: /certs/client
        SANDBOX_WORKSPACES_DIR: /var/lib/bears/sandbox-workspaces
        # deliberately NOT SANDBOX_WORKSPACES_HOST_DIR — same path in dind
        SANDBOX_ROOTS_CONFIG: /etc/bears/sandbox-roots.json
        SANDBOX_SERVICE_TOKEN / SANDBOX_PORT / limits as today
    volumes:
        - bears-sandbox-certs:/certs:ro
        - bears-sandbox-workspaces:/var/lib/bears/sandbox-workspaces
        - ${SANDBOX_ROOTS_FILE:-./data/sandbox-roots.json}:/etc/bears/sandbox-roots.json:ro
```

Plus named volumes `bears-sandbox-docker`, `bears-sandbox-certs`,
`bears-sandbox-workspaces`. The old host-socket variant and its
`SANDBOX_WORKSPACES_HOST_DIR` mapping remain supported for bare-metal-ish
setups but stop being the documented default. Backend code change: none —
`DockerCliBackend` already talks through the CLI, which honors
`DOCKER_HOST`/TLS env as-is; only `probe()`'s failure message should mention
`DOCKER_HOST` alongside the socket.

### 3. Callback DNS for nested containers (small code change)

Containers **nested inside dind** do not resolve compose service names
(their DNS chain ends at dind, not the compose network's resolver), so
`SANDBOX_CALLBACK_API_URL=http://bears-den:3001` works for the provider but
not for relays/sandboxes. Fix provider-side, once, for all nested setups:

- At provision time, resolve the callback host to an IP
  (`tokio::net::lookup_host` — the provider *can* resolve compose names) and
  pass `--add-host <host>:<ip>` to the relay (restricted mode) or the
  sandbox container (open mode). Skip when the host is already an IP
  literal.
- This also removes the current `host.docker.internal` caveat for
  single-host dev.

### 4. Sandbox images inside dind

The catalog images must exist in **dind's** store. Two mechanisms, both
supported:

- **Registry pull (production default)**: CI publishes
  `ghcr.io/<owner>/bears-sandbox{,-rust,-node,-godot}` (new workflow step
  reusing the existing GHCR credentials); the roots-file catalog references
  those; dind pulls on first use. Add `docker login` material to dind via a
  standard `config.json` mount when the packages are private.
- **One-shot builder (dev / air-gapped)**: `bears-sandbox-images` compose
  service (profile `sandbox-build`), running the docker CLI image with the
  repo mounted and `DOCKER_HOST` pointed at dind, executing
  `scripts/build-sandbox-image.sh all`. `restart: "no"`, run manually or as
  a deploy hook.

### 5. Wire Den to it

Compose defaults (den/worker env): `SANDBOX_SERVER_URL=http://bears-sandbox:3002`,
`SANDBOX_CALLBACK_API_URL=http://bears-den:3001` (http — satisfies
restricted mode), tokens shared via one env var. Document the profile in
`services/den/README.md` and `docs/guides/sandbox-server-ops.md`, replacing
the "needs a docker CLI" ponytail.

### 6. Verification

- `docker compose --profile sandbox up -d` on a clean host: health goes
  green, `/sandbox/v1/catalog` lists the images, `docker -H
  tcp://…dockerd ps` shows nothing until a dispatch.
- `scripts/work-e2e.sh` variant pointed at the compose provider (root added
  to the mounted roots file) — the full job → sandbox → publish walk.
- Egress check inside a nested sandbox: external `curl` fails, Den callback
  succeeds. Redeploy check: `compose down && up` re-adopts nothing (dind
  volume persists) and leaks nothing (project-scoped).
- Coolify: deploy the repo with `COMPOSE_PROFILES=sandbox` and confirm the
  validator accepts the file (no `${…}` volume targets anywhere).

## Effort and sequencing

Roughly 2–3 days: image + compose (1), callback DNS change + tests (½–1),
CI image publishing (½), docs + e2e verification (½–1). No schema changes;
no provider API changes. Sequencing: steps 1–2 land together behind the
profile; 3 is independently useful (fixes `host.docker.internal` dev pain)
and can ship first.

## Rejected alternative, for the record

Keeping the host-socket design and just adding a docker CLI to the Den image
is less work (~½ day) but leaves host-root-equivalent access in a deployed
service, leaks sandbox containers outside the compose project's lifecycle,
and keeps the host-path coupling that broke Coolify. Retain it only as the
escape hatch for hosts where `privileged` containers are forbidden — in that
case prefer the truly bare-metal provider instead.
