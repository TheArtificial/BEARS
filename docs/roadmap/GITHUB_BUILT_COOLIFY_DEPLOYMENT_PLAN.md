# GitHub-built Den images and Coolify deployment plan

> **Status:** Proposed  
> **Scope:** Build and publish Den-derived container images in GitHub Actions; make Coolify pull and deploy those images. This plan preserves local source builds and does not, by itself, move Bifrost or preflight image builds out of Coolify.

## Objective

Remove Den container compilation and its cache-maintenance burden from the Coolify deployment path.

GitHub Actions becomes the build authority. Coolify remains the runtime/deployment authority: it pulls a published image after a successful build and deploys the existing Compose stack.

The deployment lanes are fully automatic:

| Source branch | Mutable image tag | Coolify environment | Deployment behavior |
| --- | --- | --- | --- |
| `test` | `testing` | Testing | Build, publish, and deploy automatically after checks pass. |
| `main` | `latest` | Production | Build, publish, and deploy automatically after checks pass. |
| Either branch | `sha-<short-commit>` | N/A | Immutable artifact provenance and rollback reference. |

`main` is therefore a production deployment branch. It must be protected so that only reviewed and passing changes reach it.

## Current state

The repository already provides useful pieces of the target design:

- [`.github/workflows/den-image.yml`](../../.github/workflows/den-image.yml) publishes a standard Den image to GHCR using GitHub Actions and registry-backed BuildKit caches.
- [`.github/workflows/coolify-deploy.yml`](../../.github/workflows/coolify-deploy.yml) calls a Coolify webhook and polls `/status.json`.
- [`docker-compose.yaml`](../../docker-compose.yaml) runs `bears-den`, `bears-den-migrate`, and `bears-sandbox-provider` from Den Dockerfile targets.
- [`scripts/smoke-stack.sh`](../../scripts/smoke-stack.sh) builds local images before running the local stack.

The current workflows are not yet safe for image-driven deployment:

1. Den publication only runs when `services/den/Cargo.toml` changes, so ordinary Den source changes do not necessarily publish a new image.
2. The image workflow only publishes on a version change.
3. Coolify still builds Den from source through Compose.
4. The webhook workflow triggers from a source push independently of image publication, allowing a deployment to begin before a fresh image is available.
5. The sandbox-provider image is a different Dockerfile target but is not independently published.

## Design principles

1. **Build once, run the same artifact.** `bears-den` and `bears-den-migrate` must pull the same Den image reference during a deployment.
2. **Only deploy after successful publication.** A source push alone must never call a Coolify deployment webhook.
3. **Use mutable tags only as lane pointers.** `testing` and `latest` are convenient automatic-deployment pointers. Every published build also receives an immutable SHA tag.
4. **Keep cache ownership with GitHub Actions.** Retain both the GitHub Actions cache and GHCR registry cache, where builders can reuse them consistently.
5. **Keep runtime configuration out of images.** Coolify owns secrets, URLs, databases, domains, and persistent volumes. GitHub Actions must not bake runtime secrets into images.
6. **Preserve developer workflows.** Local Compose and smoke tests must keep building local source images rather than requiring access to GHCR.

## Image contract

Two image repositories are required because the normal Den server and sandbox provider use different Dockerfile targets.

| Compose services | Image repository | Dockerfile target | Lane tags |
| --- | --- | --- | --- |
| `bears-den`, `bears-den-migrate` | `ghcr.io/<owner>/den` | final/default | `testing`, `latest`, `sha-<commit>` |
| `bears-sandbox-provider` | `ghcr.io/<owner>/den-sandbox-provider` | `sandbox-provider` | `testing`, `latest`, `sha-<commit>` |

Optional package-version tags (`<version>` and `v<version>`) may continue to be published, but they are not the automatic deployment mechanism.

The SHA tag must be applied to every image publication, independent of whether the Cargo package version changed.

## Phase 0 — Establish deployment lanes and platform configuration

### 0.1 Define the branch policy

- Treat `test` as the automatic testing deployment branch.
- Treat `main` as the automatic production deployment branch.
- Require the normal test/quality checks before merging to `main`.
- Decide whether production deployments may be superseded while still running:
  - testing should cancel superseded runs;
  - production should normally queue or serialize runs rather than cancel a deployment already in progress.

### 0.2 Configure two Coolify resources

Maintain distinct testing and production Compose resources. They require separate database URLs, secrets, public domains, and persistent volumes.

Testing resource values:

```dotenv
BEARS_INSTANCE_SUFFIX=-test
DEN_IMAGE=ghcr.io/<owner>/den:testing
SANDBOX_PROVIDER_IMAGE=ghcr.io/<owner>/den-sandbox-provider:testing
```

Production resource values:

```dotenv
BEARS_INSTANCE_SUFFIX=
DEN_IMAGE=ghcr.io/<owner>/den:latest
SANDBOX_PROVIDER_IMAGE=ghcr.io/<owner>/den-sandbox-provider:latest
```

For both resources:

- Disable Coolify Git-push auto-deploys.
- Configure Coolify/Docker with read-only access to the GHCR packages if they are private.
- Confirm an explicit redeploy pulls a fresh digest for mutable image tags.
- Preserve the existing Den SQLite volume and database wiring.

### 0.3 Configure GitHub Environments

Create GitHub Environments named `testing` and `production`.

Each environment has its own secrets:

| Secret | Purpose |
| --- | --- |
| `COOLIFY_WEBHOOK` | Deployment webhook for that environment's Coolify resource. |
| `COOLIFY_TOKEN` | Authorization token required by the webhook, if configured. |
| `BEARS_DEPLOY_URL` | Public base URL used by post-deploy health/provenance verification. |

The initial configuration should leave production automatic, per this plan. GitHub Environment required reviewers can later be enabled to add a production promotion gate without changing the image contract.

**Exit criteria:** both Coolify resources are configured to use their lane tags and can pull from GHCR.

## Phase 1 — Publish every required Den image from GitHub Actions

Refactor [`.github/workflows/den-image.yml`](../../.github/workflows/den-image.yml) into the authoritative Den image workflow.

### 1.1 Correct the triggers

Run on pushes to `test` and `main` when any image input changes:

```yaml
paths:
  - services/den/**
  - packaging/sandbox-image/**
  - tools/bear-armature/**
  - .github/workflows/den-image.yml
```

Remove the current `Cargo.toml`-only and package-version-change gate. Every relevant source change must produce a new artifact.

### 1.2 Validate before publishing

Add or depend on the appropriate existing Den validation job(s). Publishing lane tags must occur only after focused Rust checks and any required image-level validation pass.

The exact command selection should reuse the repository's existing CI conventions rather than introduce a parallel test definition.

### 1.3 Build and publish standard Den

Build the default target from `services/den/Dockerfile` with `SQLX_OFFLINE=true`.

Publish:

- `ghcr.io/<owner>/den:sha-<short-commit>` for every build;
- `ghcr.io/<owner>/den:testing` for `test` builds;
- `ghcr.io/<owner>/den:latest` for `main` builds;
- optional semantic-version tags when appropriate.

Retain the existing two-layer cache strategy:

```yaml
cache-from: |
  type=gha,scope=den-image
  type=registry,ref=ghcr.io/<owner>/den:buildcache
cache-to: |
  type=gha,scope=den-image,mode=max,ignore-error=true
  type=registry,ref=ghcr.io/<owner>/den:buildcache,mode=max
```

### 1.4 Build and publish sandbox provider

Build the `sandbox-provider` target from the repository root so Docker BuildKit can resolve its required named contexts:

- `sandbox_packaging=./packaging/sandbox-image`
- `bear_armature=./tools/bear-armature`

Publish the matching SHA and lane tags to `ghcr.io/<owner>/den-sandbox-provider`.

Use a distinct cache scope and registry-cache reference for this target. Its build context and contents differ from the standard Den server image.

### 1.5 Set workflow concurrency

Use a lane-specific concurrency group:

```yaml
concurrency:
  group: den-image-${{ github.ref_name }}
  cancel-in-progress: true
```

For testing this is intentional: intermediate commits may be skipped, while `:testing` converges on the latest successful build.

For production, use a separate non-cancelling deployment concurrency policy if the team does not accept cancelling an in-flight production deploy.

### 1.6 Export artifact metadata

Expose the commit SHA and image digest outputs from the build workflow. The deployment phase will use this data to make provenance checks explicit.

**Exit criteria:** a non-version Den source change on `test` publishes both image repositories with `testing` and SHA tags; the equivalent change on `main` publishes `latest` and SHA tags.

## Phase 2 — Make the Coolify Compose path image-only for Den

Update [`docker-compose.yaml`](../../docker-compose.yaml) so Coolify does not build any Den-derived image.

### 2.1 Remove Den build sections in the deployment Compose configuration

For the following services, remove source `build:` definitions from the Coolify-used Compose configuration:

- `bears-den`
- `bears-den-migrate`
- `bears-sandbox-provider`

They should use only image variables:

```yaml
image: ${DEN_IMAGE}
```

for both `bears-den` and `bears-den-migrate`, and:

```yaml
image: ${SANDBOX_PROVIDER_IMAGE}
```

for `bears-sandbox-provider`.

The migration and server services must continue to receive the same `DEN_IMAGE` value.

### 2.2 Require fresh pulls for lane tags

Set an explicit pull policy for the image-driven services:

```yaml
pull_policy: always
```

This ensures that a webhook deployment resolves the newest registry digest behind `testing` or `latest` rather than reusing a stale local image with the same mutable tag.

### 2.3 Preserve local source builds

Create a development Compose override, for example `docker-compose.dev.yaml`, which restores the current local `build:` definitions for all three Den-derived services.

Update the local invocation paths, including [`scripts/smoke-stack.sh`](../../scripts/smoke-stack.sh), to include the development override explicitly. Existing local image tags such as `bears-den-dev:latest` remain local-only.

Do not silently turn local development or smoke tests into private-registry pulls.

### 2.4 Recognize the remaining Coolify builds

This phase deliberately does not change the Bifrost or preflight build definitions. Coolify may still build those services when it processes the Compose resource.

A future extension may publish those images from GitHub Actions as well, but it must be a separate decision because it broadens the artifact and release surface beyond Den.

**Exit criteria:** Coolify's Den, migration, and sandbox-provider services pull GHCR images and no longer invoke a Rust build during deploy; local smoke-stack behavior remains source-built.

## Phase 3 — Couple deployment to image publication

Refactor [`.github/workflows/coolify-deploy.yml`](../../.github/workflows/coolify-deploy.yml), or move its deployment job into `den-image.yml`.

### 3.1 Deployment must depend on publication

The deployment job must run only after both required image publication jobs succeed.

Do not retain an independent `push`-triggered webhook workflow. That race can deploy the previous mutable-tag image before GitHub has pushed the new one.

### 3.2 Select the destination from the source branch

| Branch | GitHub Environment | Coolify image tags |
| --- | --- | --- |
| `test` | `testing` | `den:testing`, `den-sandbox-provider:testing` |
| `main` | `production` | `den:latest`, `den-sandbox-provider:latest` |

The job calls the matching environment-scoped `COOLIFY_WEBHOOK` after publication has completed.

### 3.3 Serialize deployment work

Use a separate deployment concurrency group per destination:

```yaml
concurrency:
  group: coolify-deploy-${{ github.ref_name }}
  cancel-in-progress: true
```

For production, prefer `cancel-in-progress: false` if deployments should complete in order. Testing can retain cancellation because only the latest successful commit needs to be deployed.

### 3.4 Verify deployed health and provenance

After calling the webhook, retain and strengthen the existing polling logic:

1. poll `${BEARS_DEPLOY_URL}/health/ready` until it reports ready;
2. poll `${BEARS_DEPLOY_URL}/status.json`;
3. verify that the reported commit identity or deployed image digest corresponds to the image just published.

## Phase 4 — Make provenance verifiable without defeating cache reuse

The Den build scripts intentionally keep the compile-time Git SHA stable so that commit-specific build inputs do not invalidate Cargo compilation layers. See [`services/den/build.rs`](../../services/den/build.rs) and [`services/den/crates/den-http/build.rs`](../../services/den/crates/den-http/build.rs).

The migration must preserve that optimization.

### 4.1 Do not use a changing Rust build input for Git SHA

Do not solve deploy verification by changing a `GIT_SHA` Docker build argument that affects Rust compilation. That would undermine the caching goal of this work.

### 4.2 Choose and implement one runtime provenance mechanism

Select one of the following, in preferred order:

1. **Image digest verification:** make the post-deploy check retrieve the actual pulled image digest through a supported Coolify API/inspection surface and compare it with the GitHub build output.
2. **OCI image labels:** publish `org.opencontainers.image.revision` and verify the running image label through supported Coolify/Docker inspection tooling.
3. **Runtime deployment metadata:** use a supported Coolify environment/deployment metadata mechanism to set `DEN_GIT_SHA_OVERRIDE` for the image being deployed.

The current `/status.json` endpoint already prefers `DEN_GIT_SHA_OVERRIDE`, `GIT_SHA`, and `SOURCE_COMMIT` over its stable compile-time fallback. Use that runtime seam if option 3 is selected.

**Exit criteria:** each successful automatic deployment is tied to an immutable SHA-tagged artifact or image digest, and the check does not require a cache-breaking Rust rebuild.

## Phase 5 — Documentation and operational readiness

Update the deployment documentation:

- [`docs/guides/deployment/deployment.md`](../guides/deployment/deployment.md)
- [`services/den/COOLIFY_DEPLOY.md`](../../services/den/COOLIFY_DEPLOY.md)
- relevant workflow comments

Document:

- branch-to-environment and branch-to-tag mapping;
- Coolify GHCR authentication requirements;
- the requirement to disable Coolify Git-push auto-deploy;
- mutable-tag pull behavior and `pull_policy: always`;
- cache ownership and locations;
- testing and production environment configuration;
- SHA-tag/digest provenance;
- rollback instructions.

### Rollback procedure

1. Identify the known-good immutable SHA-tagged image for both Den image repositories.
2. Promote that exact artifact back to the target lane tag (`testing` or `latest`) without rebuilding it.
3. Trigger the matching Coolify webhook.
4. Verify readiness and image provenance.

The operator must not rebuild an old source revision merely to roll back. Re-tagging the already-built artifact preserves the original tested bytes.

**Exit criteria:** an operator can configure, deploy, verify, and roll back either lane using the documented procedure.

## Validation checklist

### Local development

- Run Compose configuration validation for the base plus development override.
- Run `./scripts/smoke-stack.sh` and confirm it builds/runs local source images without GHCR access.

### GitHub Actions

- Push a harmless Den source change to `test`.
- Confirm the standard and sandbox-provider images both receive matching SHA tags and the `testing` tag.
- Confirm BuildKit imports/exports both GHA and registry cache layers.
- Confirm a changed Den source file publishes even when `Cargo.toml` did not change.

### Testing deployment

- Confirm the webhook runs only after both images are successfully pushed.
- Confirm Coolify pulls the new digest behind `:testing`.
- Confirm `bears-den-migrate` and `bears-den` use the same image digest.
- Confirm `/health/ready` and `/status.json` checks pass.

### Production deployment

- Merge a tested change to `main`.
- Confirm the matching artifacts receive `:latest` and production deploy begins only after publication.
- Confirm production resolves a new `:latest` digest and passes health/provenance verification.
- Exercise a rollback using a previously published SHA artifact.

## Acceptance criteria

- A Den-relevant change on `test` automatically builds, publishes, and deploys the `testing` lane.
- A Den-relevant change on `main` automatically builds, publishes, and deploys the `latest` lane.
- Coolify does not compile Den, run Cargo, or depend on deploy-host BuildKit caches for Den-derived services.
- The deployment webhook cannot run before the required GHCR images are published.
- `bears-den` and `bears-den-migrate` run the same immutable Den artifact.
- Local source-built Compose and smoke-stack workflows continue to function.
- Deployments have verifiable SHA/digest provenance and a no-rebuild rollback path.
