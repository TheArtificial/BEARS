# Clients, Channels, Armatures, and Adapters

Den uses these terms deliberately so core runtime concepts do not inherit a wire
protocol name.

## Client

A **client** is any external surface connected to Den. Client terminology is the
right default for protocol-neutral runtime identity:

- `client_session_id`
- `client_session`
- `client_turn`
- `client_tool_turn`

Use `client` when the concept is about connection/session/turn identity and does
not imply a particular capability surface.

## Channel

A **channel** is a conversational client surface. Examples include web chat,
Slack, WhatsApp, or a future desktop companion chat. A channel may let a human
talk with a Bear, but it does not inherently grant trusted local workspace tools.

Use `channel` for conversation-only routing, presentation, and metadata.

## Armature

An **armature** is a trusted client type that gives a Bear a work-surface harness:
local filesystem, git, terminal/process execution, browser control, editor state,
or forwarded MCP tools. ACP/Zed and BearWire armature clients are examples.

Use `armature` for trusted local action capability concepts:

- tool IDs: `armature.fs.edit_file`, `armature.git.status`,
  `armature.terminal.run_command`, `armature.process.run`
- modules: `armature_tools`
- policy scopes: `ToolScopeKind::ArmatureWorkspace`
- policy basis strings: `scope_basis: "armature:tools"`

Prefer `client_session_id` over `armature_session_id` unless the session itself
cannot exist without trusted local tools. A web chat session is a client session,
not an armature session.

## Adapter

An **adapter** is implementation glue for a specific protocol or integration.
Adapters translate protocol-specific requests/events into Den-owned concepts and
project Den-owned events back to the wire.

Use `adapter` for bridge code, not core domain names. For example, `den-acp` is an
ACP adapter; ACP-specific names should stay there rather than in `den-core`,
`den-service`, or `den-runtime`.

## Rule Of Thumb

- If it identifies a connection or turn, use `client`.
- If it is conversation-only, use `channel`.
- If it grants trusted local work tools, use `armature`.
- If it translates a protocol, use `adapter`.

This keeps Den protocol-agnostic while still naming the special trust boundary
that lets a Bear take action on a user's work surface.
