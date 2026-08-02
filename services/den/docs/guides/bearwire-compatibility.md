# BearWire compatibility

Den and Armature may be released from separate repositories. Work sandboxes therefore negotiate protocol behavior instead of comparing Git revisions or requiring the newest image.

Armature sends a `CompatibilityManifest` with `work.checkout`. Den validates the protocol generation and required capabilities before binding the session or mutating run/task state. An incompatible image fails during provisioning; it must never begin a task and fail later in the protocol.

## Changing the protocol

- Capability wire names are permanent. Never reuse a name or change its meaning.
- Additive behavior gets a new capability.
- Breaking semantic changes get a new capability or protocol generation.
- Den should require a capability only when it cannot operate safely without it. Supporting an optional feature is not a reason to invalidate old images.
- Every advertised capability needs a runnable conformance check covering the actual exchange, not only manifest serialization.

Adding a required capability is a compatibility boundary: deployed sandbox images may be older, so release a compatible Armature image before Den relies on it. Unrelated changes to either repository do not require sandbox rebuilds.

Build versions and Git revisions are useful diagnostic evidence, but are not compatibility gates.
