# Sandbox server operations

Running the `RUN_SANDBOX` provider and wiring it to a Den instance: topology,
configuration, the roots/catalog file, credentials, networking, monitoring,
and cleanup. Companion docs: [user guide](running-background-work.md),
[developer internals](work-sandbox-internals.md).

## Topology

```
 Den host                              Sandbox host
┌───────────────────────┐            ┌──────────────────────────────────────┐
│ bears-den             │  /sandbox/ │ sandbox provider (RUN_SANDBOX)       │
│  RUN_API + RUN_WORKERS├───v1──────►│  roots (pristine clones)             │
│  dispatch worker      │  bearer    │  workspaces (per-run clones)         │
│  Postgres             │  token     │  docker daemon                       │
└───────▲───────────────┘            │   ├─ den-sbx-<id>   (sandbox)        │
        │ BearWire (armature token)  │   ├─ den-sbx-gw-<id> (egress relay)  │
        └────────────────────────────┤   └─ den-sbx-net-<id> (internal net) │
                                     └──────────────────────────────────────┘
```

- The provider is **standalone**: no Postgres, no Den database access. All
  durable work state (runs, results, publish records) lives on Den. The
  provider's in-memory registry is rebuilt after restart by adopting
  containers labeled `den.sandbox=1`, so the host can be wiped or the
  service restarted without losing anything durable.
- Two independent auth channels: Den → provider uses the static
  `SANDBOX_SERVICE_TOKEN` bearer; in-sandbox armatures dial back to Den's
  API with **per-run ephemeral tokens** minted at provision time and revoked
  at harvest. Root credentials (deploy keys, tokens) live only on the
  sandbox host and never enter sandboxes or transit Den.
- Two supported deployments:
  - **Compose (`COMPOSE_PROFILES=sandbox`)** — `bears-sandbox-provider`
    (the API service with a docker CLI) plus `bears-sandbox-engine` (a
    dedicated dind Docker Engine hosting all sandbox containers, relays,
    and networks). Nothing touches the host daemon; the shared workspaces
    volume mounts at the same path in both, and `docker compose down -v
    --profile sandbox` removes every trace. Catalog images must exist in
    the *engine's* store — use the GHCR-published `bears-sandbox*` images
    or the `bears-sandbox-images` one-shot builder
    (`docker compose --profile sandbox-build run --rm bears-sandbox-images`).
  - **Bare-metal provider on a dedicated sandbox host** — the setup
    described in the bring-up checklist below; sandboxes run on that
    host's own docker daemon.

## Bring-up checklist

1. **Images**: `scripts/build-sandbox-image.sh all` builds
   `bears/sandbox:latest` plus `bears/sandbox-{rust,node,godot}:latest`.
   Rebuild whenever `tools/bear-armature` changes (the armature binary is
   baked in).
2. **Roots + catalog file** (`SANDBOX_ROOTS_CONFIG`), the provider's single
   source of what can be worked on and what can run:

   ```json
   {
     "images": [
       { "name": "base",  "image": "bears/sandbox:latest", "default": true },
       { "name": "rust",  "image": "bears/sandbox-rust:latest" },
       { "name": "node",  "image": "bears/sandbox-node:latest" }
     ],
     "roots": [
       { "name": "scratch", "path": "/srv/scratch" },
       { "name": "site", "default_image": "node",
         "upstream": {
           "url": "git@github.com:you/site.git",
           "default_ref": "main",
           "credential": { "ssh_key_path": "/etc/bears/keys/site" } } }
     ]
   }
   ```

   - Git roots are **pristine bare mirrors** under
     `SANDBOX_WORKSPACES_DIR/pristine/`, synced fetch/fast-forward-only
     before every provisioning. A force-pushed upstream shows up as a sync
     *error*, never a local overwrite — resolve by deleting the pristine
     clone and letting it re-clone.
   - Credentials are declared per root as `{"ssh_key_path": …}` (key file on
     this host) or `{"token_env": "NAME"}` (env var on this host holding an
     HTTPS token). Grant **write** access only to roots whose jobs publish.
   - The `images` catalog is a **security boundary**: dispatch selects by
     name; anything not listed cannot run. Registry references work — the
     daemon pulls on first use.
3. **Run the provider**:

   ```sh
   RUN_SANDBOX=true \
   SANDBOX_PORT=3002 \
   SANDBOX_SERVICE_TOKEN=<random> \
   SANDBOX_ROOTS_CONFIG=/etc/bears/sandbox-roots.json \
   SANDBOX_WORKSPACES_DIR=/var/lib/bears/sandbox-workspaces \
   den serve
   ```

   Only a standalone `RUN_SANDBOX` process may run without `DATABASE_URL`.
4. **Point Den at it** (Den worker env):

   ```sh
   SANDBOX_SERVER_URL=http://sandbox-host:3002
   SANDBOX_SERVER_TOKEN=<same token>
   SANDBOX_CALLBACK_API_URL=http://den-host:3001   # http, reachable from containers
   ```
5. **Smoke it**: `curl http://sandbox-host:3002/sandbox/v1/health` (open
   endpoint) shows backend availability and per-root status;
   `curl -H "Authorization: Bearer <token>" …/sandbox/v1/catalog` shows what
   dispatch will see. `scripts/work-e2e.sh` walks the whole flow against a
   throwaway local upstream.

## Configuration reference (provider)

| Env | Default | Meaning |
|---|---|---|
| `SANDBOX_PORT` | 3002 | Provider API port |
| `SANDBOX_SERVICE_TOKEN` | *(empty = auth off, dev only)* | Static bearer for `/sandbox/v1` |
| `SANDBOX_ROOTS_CONFIG` | *(unset = no roots)* | Roots + image catalog JSON |
| `SANDBOX_WORKSPACES_DIR` | `./data/sandbox-workspaces` | Pristine clones + per-run workspaces |
| `SANDBOX_WORKSPACES_HOST_DIR` | *(unset)* | Daemon-side path of the workspaces dir when the provider's filesystem view differs from its docker daemon's (bind sources are resolved by the daemon). Not needed in the compose profile — the shared volume mounts at identical paths in provider and engine |
| `SANDBOX_IMAGE` | *(empty)* | Fallback image when the catalog is empty |
| `SANDBOX_MAX_CONCURRENT` | 2 | Sandbox slots; excess dispatches queue Den-side |
| `SANDBOX_DEFAULT_TIMEOUT_SECS` | 900 | Hard wall clock; the reaper destroys overdue sandboxes |
| `SANDBOX_MAX_LOG_BYTES` | 2 MiB | Cap on retained/served log bytes |
| `DOCKER_BIN` | `docker` | Set `podman` for a podman host |

Den-side knobs: `WORK_DISPATCH_AUTO` (auto-enqueue runnable work tasks; off
by default), `WORK_MAX_ATTEMPTS` (infrastructure-failure retries, default 2),
`SANDBOX_PRESERVE_FAILED` (keep failed runs' workspaces for debugging),
`WORK_SANDBOX_NETWORK` (`restricted` default / `open`).

## Networking

Default **restricted** mode, per sandbox:

- `den-sbx-net-<id>` — an `--internal` docker network (no external route).
- `den-sbx-gw-<id>` — a socat relay attached to both that network and the
  default bridge, forwarding **only** to the Den callback host:port. The
  sandbox's `DEN_API_URL` is rewritten to the relay, so task code can reach
  Den and nothing else — no package registries, no external APIs. Bake
  toolchains into images instead.
- Requirement: `SANDBOX_CALLBACK_API_URL` must be **plain http** in
  restricted mode (the relay forwards raw TCP under its own name, so TLS
  verification against the original hostname cannot succeed). Use
  `WORK_SANDBOX_NETWORK=open` if you must call back over https.
- Every run's `sandbox_strength` states the mode actually in effect; it is
  visible on the run page and in `get_work_run`.

## Monitoring and troubleshooting

- `GET /sandbox/v1/health` (no auth): `backend_available` (docker probe) and
  per-root `ok`/`detail`. `GET /sandbox/v1/metrics` (auth): Prometheus
  counters — provisioned/failed/destroyed/timed-out sandboxes, cleanup
  failures, log bytes served, active gauge.
- On the host, everything is labeled: `docker ps --filter label=den.sandbox`
  (sandboxes, `work_run_id` in labels), `--filter label=den.sandbox.relay`
  (relays). Names: `den-sbx-<id>`, `den-sbx-gw-<id>`, `den-sbx-net-<id>`.
- The Den-side `/work` UI is the operator view of runs: active runs, log
  tails, diffs, publish results, cleanup failures (⚠ badges), usage.

Common failures:

| Symptom | Likely cause / fix |
|---|---|
| run fails `provision` / `runtime_unavailable` | docker daemon down or socket unreachable (`docker info` as the provider user) |
| run fails `unknown_root` / `unknown_image` | dispatch references names missing from the roots file; check `/sandbox/v1/catalog` |
| run fails `root_sync_failed` | upstream unreachable or credential missing/expired; sync manually with `POST /sandbox/v1/roots/{name}/sync` |
| run succeeded but `PUBLISH FAILED: …` | push rejected (credential lacks write, or non-fast-forward from a raced branch); the diff is still on the run page |
| run fails `turn_lost` | armature crashed or hit the deadline — read the attached `log_tail`; often an image missing a required toolchain |
| ⚠ cleanup failed on a run | container/workspace removal failed; the reaper retries, but check disk and `docker ps -a` |
| armature can't reach Den | `SANDBOX_CALLBACK_API_URL` not reachable *from a container* (test with `docker run --rm curlimages/curl <url>/up`), or https in restricted mode |

## Capacity and cleanup

- Disk: each run costs a full clone under
  `SANDBOX_WORKSPACES_DIR/workspaces/<id>` (deleted on teardown unless
  preserved) plus one pristine mirror per git root. Watch the volume;
  preserved failed workspaces are yours to delete.
- Concurrency: `SANDBOX_MAX_CONCURRENT` guards the host; Den queues the
  rest. Timeouts are enforced by the provider's reaper independently of Den.
- Leaks are self-healing in both directions: the provider re-adopts labeled
  containers after a restart, and Den's hourly orphan sweep destroys
  provider sandboxes whose runs are already terminal. A relay whose sandbox
  was *manually* removed is the one thing neither side cleans — remove
  `den-sbx-gw-*` / `den-sbx-net-*` strays by hand if you bypass the API.
