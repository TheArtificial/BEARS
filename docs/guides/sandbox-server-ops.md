# Sandbox server operations

Running the `RUN_SANDBOX` provider and wiring it to a Den instance: topology,
managed configuration (work surfaces + image catalog), credentials,
networking, monitoring, and cleanup. Companion docs:
[user guide](running-background-work.md),
[developer internals](work-sandbox-internals.md).

## Topology

```
 Den host                              Sandbox host
┌───────────────────────┐            ┌──────────────────────────────────────┐
│ bears-den             │  /sandbox/ │ sandbox provider (RUN_SANDBOX)       │
│  RUN_API + RUN_WORKERS├───v1──────►│  managed config (synced from Den)    │
│  dispatch worker      │  bearer    │  roots (pristine clones)             │
│  Postgres:            │  token     │  workspaces (per-run clones)         │
│   work_surfaces,      │            │  docker daemon                       │
│   sandbox catalog     │            │   ├─ den-sbx-<id>   (sandbox)        │
└───────▲───────────────┘            │   ├─ den-sbx-gw-<id> (egress relay)  │
        │ BearWire (armature token)  │   └─ den-sbx-net-<id> (internal net) │
        └────────────────────────────┴──────────────────────────────────────┘
```

- The provider is **standalone**: no Postgres, no Den database access. All
  durable work state (runs, results, publish records) lives on Den. The
  provider's in-memory registry is rebuilt after restart by adopting
  containers labeled `den.sandbox=1`, so the host can be wiped or the
  service restarted without losing anything durable.
- Two independent auth channels: Den → provider uses the static
  `SANDBOX_SERVICE_TOKEN` bearer; in-sandbox armatures dial back to Den's
  API with **per-run ephemeral tokens** minted at provision time and revoked
  at harvest.
- Two supported deployments:
  - **Compose (default stack)** — `bears-sandbox-provider` (the API
    service with a docker CLI) plus `bears-sandbox-engine` (a dedicated
    dind Docker Engine hosting all sandbox containers, relays, and
    networks) run by default, wired to the Den workers out of the box
    (`RUN_WORKERS=true`, `SANDBOX_SERVER_URL`, and a matched
    token default — override `SANDBOX_SERVER_TOKEN` in production).
    Nothing touches the host daemon; the shared workspaces volume mounts
    at the same path in both, and `docker compose down -v` removes every
    trace.
  - **Bare-metal provider on a dedicated sandbox host** — the setup
    described in the bring-up checklist below; sandboxes run on that
    host's own docker daemon. Cross-host, put TLS in front of the
    provider port: the managed-config push carries decrypted surface
    credentials protected only by the bearer token.

## Managed configuration (surfaces + image catalog)

Den's Postgres is the **source of truth** for what work can run on and what
images can run. There is no config file on the provider
(`SANDBOX_ROOTS_CONFIG` is deprecated: warned about and ignored).

- **Work surfaces** (`/work/surfaces` in the web UI, any logged-in user):
  a named git upstream + default ref + default image + optional credential.
  The creator owns the surface, grants other managers, and assigns bears —
  only members of an assigned bear can create/dispatch jobs on it (enforced
  at job creation and again at enqueue).
- **Image catalog** (`/admin/sandbox`, site admins): name → container image
  reference. The catalog is a **security boundary**: dispatch selects by
  name; anything not listed cannot run. Registry references are pulled by
  the engine on first use; the `/admin/sandbox` page also runs pulls and
  one-click builds of the shipped `bears/sandbox*` variants and shows the
  engine's store and disk usage.
- **Sync**: Den pushes the full set declaratively
  (`PUT /sandbox/v1/managed-config`) after every mutation, on dispatch-
  worker startup, and on a 5-minute version-checked reconcile. The provider
  persists it under `SANDBOX_WORKSPACES_DIR/managed/` so provisioning works
  between pushes and across restarts; a wiped provider volume is re-seeded
  by the next push.
- **Credentials** (deploy key or HTTPS token, per surface): entered
  write-only in the surface UI, stored **encrypted** in Den's Postgres
  (`DEN_SECRET_ENCRYPTION_KEY`), pushed over the authed channel, and written
  to per-surface `0600` files under
  `SANDBOX_WORKSPACES_DIR/managed/credentials/`. They are used host-side for
  git sync/publish only — never in the persisted config JSON, on command
  lines, in logs, or inside sandboxes.
- Git roots are **pristine bare mirrors** under
  `SANDBOX_WORKSPACES_DIR/pristine/<surface>.git`, synced
  fetch/fast-forward-only before every provisioning. A force-pushed
  upstream shows up as a sync *error*, never a local overwrite — resolve by
  deleting the pristine clone and letting it re-clone.

## Multiple instances on one Docker host

Production leaves `BEARS_INSTANCE_SUFFIX` empty and retains `bears-sandbox-provider` / `bears-sandbox-engine`. A secondary Compose deployment on the same host and shared predefined network sets a DNS-safe suffix such as `BEARS_INSTANCE_SUFFIX=-test`; Compose then uses `bears-sandbox-provider-test` / `bears-sandbox-engine-test`, an isolated `bears-stack-test` project, and separate sandbox engine, certificate, and workspace volumes. The suffix is also included in the Docker-in-Docker server certificate SAN and every generated client endpoint.

## Bring-up checklist

1. **Run the provider** (compose does all of this for you):

   ```sh
   RUN_SANDBOX=true \
   SANDBOX_PORT=3002 \
   SANDBOX_SERVICE_TOKEN=<random> \
   SANDBOX_WORKSPACES_DIR=/var/lib/bears/sandbox-workspaces \
   den serve
   ```

   Only a standalone `RUN_SANDBOX` process may run without `DATABASE_URL`.
2. **Point Den at it** (Den worker env):

   ```sh
   SANDBOX_SERVER_URL=http://sandbox-host:3002
   SANDBOX_SERVER_TOKEN=<same token>
   SANDBOX_CALLBACK_API_URL=http://den-host:3001   # http, reachable from containers
   ```

   The dispatch worker pushes the managed config at startup — watch for
   `surface_sync: managed config pushed to sandbox provider`.
3. **Images**: the migration seeds catalog entries for
   `bears/sandbox:latest` (+ rust/node/godot variants). Get them into the
   engine's store from **`/admin/sandbox`** (build buttons or a registry
   pull), or with `scripts/build-sandbox-image.sh all` /
   `docker compose --profile sandbox-build run --rm bears-sandbox-images`.
   Rebuild whenever `tools/bear-armature` changes (the armature binary is
   baked in).
4. **Surfaces**: create one at **`/work/surfaces/new`** (upstream URL,
   default ref, default image, credential), then assign the bears that may
   use it.
5. **Smoke it**: `curl http://sandbox-host:3002/sandbox/v1/health` (open
   endpoint) shows backend availability and per-root status;
   `…/sandbox/v1/managed-config` (auth) shows the applied surface/image
   counts + version; `…/sandbox/v1/catalog` (auth) shows what dispatch will
   see. `scripts/work-e2e.sh` walks the whole flow against a throwaway
   local upstream.

## Configuration reference (provider)

| Env | Default | Meaning |
|---|---|---|
| `SANDBOX_PORT` | 3002 | Provider API port |
| `SANDBOX_SERVICE_TOKEN` | *(empty = auth off, dev only)* | Static bearer for `/sandbox/v1` |
| `SANDBOX_WORKSPACES_DIR` | `./data/sandbox-workspaces` | Pristine clones, per-run workspaces, persisted managed config |
| `SANDBOX_WORKSPACES_HOST_DIR` | *(unset)* | Daemon-side path of the workspaces dir when the provider's filesystem view differs from its docker daemon's (bind sources are resolved by the daemon). Not needed in compose — the shared volume mounts at identical paths in provider and engine |
| `SANDBOX_BUILD_CONTEXT_DIR` | *(unset = builds disabled)* | Read-only mount of the repo's `packaging/sandbox-image` + `tools/bear-armature` trees for `/admin/sandbox` one-click builds |
| `SANDBOX_IMAGE` | *(empty)* | Fallback image when the catalog is empty |
| `SANDBOX_MAX_CONCURRENT` | 2 | Sandbox slots; excess dispatches queue Den-side |
| `SANDBOX_DEFAULT_TIMEOUT_SECS` | 900 | Hard wall clock; the reaper destroys overdue sandboxes |
| `SANDBOX_MAX_LOG_BYTES` | 2 MiB | Cap on retained/served log bytes |
| `DOCKER_BIN` | `docker` | Set `podman` for a podman host |
| `SANDBOX_ROOTS_CONFIG` | *(deprecated)* | Ignored; a warning is logged if set |

Den-side knobs: `WORK_DISPATCH_AUTO` (auto-enqueue runnable work tasks; off
by default), `WORK_MAX_ATTEMPTS` (infrastructure-failure retries, default 2),
`SANDBOX_PRESERVE_FAILED` (keep failed runs' workspaces for debugging),
`WORK_SANDBOX_NETWORK` (`restricted` default / `open`),
`DEN_SECRET_ENCRYPTION_KEY` (required to store surface credentials).

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
  `GET /sandbox/v1/managed-config` (auth): applied surface/image counts +
  config version.
- On the host, everything is labeled: `docker ps --filter label=den.sandbox`
  (sandboxes, `work_run_id` in labels), `--filter label=den.sandbox.relay`
  (relays). Names: `den-sbx-<id>`, `den-sbx-gw-<id>`, `den-sbx-net-<id>`.
- The Den-side `/work` UI is the operator view of runs: active runs, log
  tails, diffs, publish results, cleanup failures (⚠ badges), usage.
  `/admin/sandbox` shows the engine store, disk usage, and running image
  operations (pulls/builds; these live in provider memory and do not
  survive restarts — the image list is the durable truth).

Common failures:

| Symptom | Likely cause / fix |
|---|---|
| run fails `provision` / `runtime_unavailable` | docker daemon/engine down (`docker info` from the provider environment) |
| run fails `unknown_root` / `unknown_image` | the surface/catalog entry doesn't exist or hasn't synced — check `/sandbox/v1/managed-config` counts and the worker's `surface_sync` log lines, or hit "Sync now" on the surface page |
| dispatch rejected `bear is not assigned to work surface '…'` | assign the bear on the surface's manage page |
| run fails `root_sync_failed` | upstream unreachable or credential missing/expired — replace it on the surface page (rotation re-syncs automatically) |
| run succeeded but `PUBLISH FAILED: …` | push rejected (credential lacks write, or non-fast-forward from a raced branch); the diff is still on the run page |
| run fails `turn_lost` | armature crashed or hit the deadline — read the attached `log_tail`; often an image missing a required toolchain |
| ⚠ cleanup failed on a run | container/workspace removal failed; the reaper retries, but check disk and `docker ps -a` |
| armature can't reach Den | `SANDBOX_CALLBACK_API_URL` not reachable *from a container* (test with `docker run --rm curlimages/curl <url>/up`), or https in restricted mode |
| build button says `build_unavailable` | `SANDBOX_BUILD_CONTEXT_DIR` unset or the packaging mount missing on the provider |

## Capacity and cleanup

- Disk: each run costs a full clone under
  `SANDBOX_WORKSPACES_DIR/workspaces/<id>` (deleted on teardown unless
  preserved) plus one pristine mirror per git surface. Image layers and
  build cache accumulate in the engine volume — watch `/admin/sandbox`'s
  disk-usage panel and prune from there (image removal) or with
  `docker system prune` against the engine.
- Concurrency: `SANDBOX_MAX_CONCURRENT` guards the host; Den queues the
  rest. Timeouts are enforced by the provider's reaper independently of Den.
- Leaks are self-healing in both directions: the provider re-adopts labeled
  containers after a restart, and Den's hourly orphan sweep destroys
  provider sandboxes whose runs are already terminal. A relay whose sandbox
  was *manually* removed is the one thing neither side cleans — remove
  `den-sbx-gw-*` / `den-sbx-net-*` strays by hand if you bypass the API.
