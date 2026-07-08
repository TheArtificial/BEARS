# Repository Shape

This page records canonical source locations that are easy to confuse with legacy binary or package names.

| Path or name | Meaning |
| --- | --- |
| `tools/bear-armature/` | Canonical Rust source for the BearWire/ACP armature adapter and local workspace tool execution. |
| `bear-armature` | Current built executable name for the armature adapter. |
| `bears-acp-adapter` | Legacy binary/package/resource alias kept only at compatibility boundaries such as update manifests, bundled app resources, and installed symlinks. Do not use this as a source directory path. |
| `services/den/` | Den Rust workspace: runtime, BearWire methods, persistence, and Den-hosted tool execution. |
| `docs/` | Product, architecture, guide, ADR, and roadmap documentation. |

When adding implementation docs or model-facing troubleshooting context, prefer the canonical source path (`tools/bear-armature/`) and mention `bears-acp-adapter` only when discussing legacy executable/package compatibility.
