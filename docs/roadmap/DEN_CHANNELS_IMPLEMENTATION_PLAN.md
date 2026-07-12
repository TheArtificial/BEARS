# Den channels — implementation plan

**Status:** Draft  
**Date:** 2026-06-20  
**Related:** [BearWire armature wire plan](BEARWIRE_ARMATURE_WIRE_IMPLEMENTATION_PLAN.md), [macOS Bears client app plan](MACOS_BEARS_CLIENT_APP_PLAN.md), [Bears macOS app implementation plan](BEARS_MACOS_APP_IMPLEMENTATION_PLAN.md)

## Purpose

Make it easy to add human conversation **channels** to a Den server — Slack, WhatsApp, web chat, and app-local chat — without building an over-general plugin system.

A channel is a communication surface. It carries conversation between humans and Bears, may include rich media or interaction affordances, and may have channel-specific policy constraints. This is distinct from an **armature** such as ACP/Zed, which gives the Bear a trusted work-surface harness with local tools, permissions, and editor context.

## Design stance

Do **not** build a plugin marketplace, dynamic extension runtime, or third-party adapter ABI yet.

Instead, build a small, first-party `den-channels` layer:

- a canonical channel event/message model;
- a static internal adapter registry;
- per-channel configuration and capability descriptors;
- dedicated first-party adapters for webhook, web chat, Slack, WhatsApp/Twilio, and macOS app chat;
- shared handoff into the same Den run service used by BearWire.

BearWire remains the Den ↔ armature protocol. Channel adapters may use BearWire later if they run out of process and need interactive run semantics, but first-party in-process channel adapters should call the shared Den runtime service directly.

## Research summary

### Microsoft Bot Framework

Bot Framework treats a channel as a connection between a communication application and a bot. Bots are written against a normalized Activity schema; adapters/connectors transform between channel schemas and the normalized schema. It also explicitly acknowledges that channels may not support all features and that schema transformation behavior may need versioning.

**Takeaway:** Den should have one normalized channel model plus per-channel renderers and capability-aware degradation.

### Slack / Bolt

Slack apps are event-driven and configured around scopes, app manifests, event subscriptions or Socket Mode, interaction callbacks, and Web API calls. Slack-specific affordances such as threads, Block Kit buttons, modals, message updates, app mentions, DMs, and file events matter.

**Takeaway:** Slack deserves a dedicated adapter, but only the Slack edge should know Slack payloads and Block Kit details.

### Rasa channels / Twilio

Rasa uses configured connectors. The Twilio connector uses credentials and a webhook endpoint. WhatsApp can ride through Twilio by using `whatsapp:`-prefixed phone numbers. Channel-specific payloads such as location data are normalized into assistant-level concepts.

**Takeaway:** static connector registration plus config is enough for initial Den channels.

### Twilio / WhatsApp

Twilio’s WhatsApp support is webhook/API based and policy-heavy. WhatsApp has opt-in requirements, a 24-hour customer service window for freeform replies, template requirements for business-initiated messages outside that window, media URLs for inbound attachments, and status callbacks for delivery state.

**Takeaway:** Den channel adapters need explicit policy and capability metadata, not just `send_text()`.

### Web chat / Direct Line style systems

Web chat systems usually use a first-party widget, server-minted short-lived tokens, and a live event connection such as SSE, WebSocket, WebRTC, or a Direct Line-like protocol. The browser should not receive long-lived server secrets.

**Takeaway:** Den web chat should be a first-class channel with stronger live event support than Slack or WhatsApp.

### Botpress-style platforms

Botpress exposes conversations, users, messages, events, state, integrations, web chat, HITL, and channel mapping docs. This is a mature integration platform shape, but more than Den needs immediately.

**Takeaway:** use its concepts as validation, not as a reason to implement a plugin hub now.

## Target architecture

```text
External channel payload
        │
        ▼
Channel adapter
  - verify/authenticate webhook or client token
  - dedupe inbound events
  - normalize sender/thread/message/media
  - bind external conversation to Den conversation
  - map channel-specific interactions
        │
        ▼
Canonical ChannelInboundEvent
        │
        ▼
Den run service / native runtime
        │
        ▼
Canonical ChannelOutboundEvent
        │
        ▼
Channel renderer
  - send/update message
  - render buttons/modals/templates
  - upload/link media
  - respect channel policy
```

## Principles

1. **Canonical core, specific edges** — Slack remains Slack-shaped only at the Slack edge; WhatsApp remains WhatsApp-shaped only at the Twilio/WhatsApp edge.
2. **No dynamic plugins yet** — compile first-party adapters into Den; revisit an SDK only after at least two production channels stabilize.
3. **Capability-aware rendering** — do not pretend every channel can stream, update messages, show buttons, or carry arbitrary media.
4. **Policy is part of channel behavior** — WhatsApp opt-in/template windows, Slack scope limits, web chat token lifetime, etc. are first-class constraints.
5. **Idempotent inbound processing** — all channel webhooks/events need stable external event dedupe.
6. **Separate inbound normalization from outbound rendering** — avoid one bidirectional blob of channel-specific logic.
7. **BearWire remains armature-first** — channels reuse runtime services, not ACP/armature assumptions.

## Core model sketch

```rust
pub enum ChannelKind {
    Webhook,
    WebChat,
    Slack,
    TwilioWhatsApp,
    MacosAppChat,
}

pub struct ChannelInboundEvent {
    pub installation_id: Uuid,
    pub channel_kind: ChannelKind,
    pub external_event_id: String,
    pub external_conversation_id: String,
    pub external_thread_id: Option<String>,
    pub external_user_id: String,
    pub received_at: OffsetDateTime,
    pub message: Option<ChannelMessage>,
    pub interaction: Option<ChannelInteraction>,
    pub raw_payload: serde_json::Value,
}

pub struct ChannelMessage {
    pub text: Option<String>,
    pub content_blocks: Vec<ChannelContentBlock>,
    pub attachments: Vec<ChannelAttachment>,
    pub locale: Option<String>,
}

pub struct ChannelAttachment {
    pub kind: ChannelAttachmentKind,
    pub media_url: Option<String>,
    pub mime_type: Option<String>,
    pub filename: Option<String>,
    pub size_bytes: Option<i64>,
    pub metadata: serde_json::Value,
}
```

## Adapter shape

This should be an internal Rust abstraction, not a public plugin ABI:

```rust
pub trait ChannelAdapter {
    fn kind(&self) -> ChannelKind;
    fn capabilities(&self) -> ChannelCapabilities;

    async fn verify_inbound(&self, request: ChannelHttpRequestParts) -> Result<(), ChannelError>;
    async fn normalize_inbound(&self, request: ChannelHttpRequest) -> Result<Vec<ChannelInboundEvent>, ChannelError>;

    async fn render_outbound(&self, event: ChannelOutboundEvent) -> Result<Vec<ChannelSendOp>, ChannelError>;
    async fn execute_send(&self, op: ChannelSendOp) -> Result<ChannelSendReceipt, ChannelError>;
}
```

Use static registration, for example:

```text
den-channels/
  model.rs
  registry.rs
  service.rs
  adapters/
    webhook.rs
    webchat.rs
    slack.rs
    twilio_whatsapp.rs
    macos_app_chat.rs
```

## Storage sketch

```text
channel_installations
- id
- bear_id
- kind
- enabled
- display_name
- encrypted_config_json
- created_by_user_id
- created_at
- updated_at

channel_conversation_bindings
- id
- installation_id
- bear_id
- external_conversation_id
- external_thread_id
- den_conversation_id
- created_at
- updated_at

channel_event_dedupe
- installation_id
- external_event_id
- processed_at

channel_delivery_receipts
- id
- installation_id
- den_conversation_id
- channel_message_id
- status
- diagnostic_json
- created_at
- updated_at
```

## Capability descriptor sketch

```rust
pub struct ChannelCapabilities {
    pub threads: bool,
    pub message_update: bool,
    pub typing_indicator: bool,
    pub buttons: bool,
    pub modals: bool,
    pub attachments: Vec<ChannelAttachmentKind>,
    pub proactive: ProactivePolicy,
    pub max_text_len: Option<usize>,
    pub requires_template_for_proactive: bool,
    pub supports_streaming: ChannelStreamingMode,
}
```

Examples:

| Channel | Key capabilities | Constraints |
| --- | --- | --- |
| Webhook | final text, basic attachments | no native UX assumptions |
| Web chat | streaming, typing, attachments, rich custom events | server-minted short-lived tokens |
| Slack | threads, updates, buttons, modals, files | scopes, event retries, Block Kit limits |
| WhatsApp/Twilio | text, media URLs, delivery status callbacks | opt-in, 24h window, templates for proactive sends |
| macOS app chat | native UI, notifications, local identity, app state | app sandboxing, local secrets, offline behavior |

## Phase A — Core channel primitives

**Goal:** Add the minimal internal channel model and storage without shipping a real external integration.

| Task | Location | Done when |
| --- | --- | --- |
| Add `den-channels` crate or `den-runtime::channels` module | `services/den/crates/den-channels/` or `den-runtime/src/channels/` | Types compile; no app behavior change |
| Define `ChannelKind`, inbound/outbound event types, attachments, interactions | model module | Covers text, multimodal attachments, buttons/actions, raw payload retention |
| Define `ChannelCapabilities` | model module | Webhook, web chat, Slack, WhatsApp, macOS app chat can be described |
| Add storage migrations | `services/den/migrations/` | Installations, bindings, event dedupe, delivery receipts exist |
| Add channel binding service | channel service | External conversation/thread maps to one Den conversation id |
| Add dedupe service | channel service | Duplicate external event ids are ignored idempotently |

**Exit gate:** Unit tests for binding and dedupe pass; no public routes required.

## Phase B — Generic webhook channel

**Goal:** Prove the channel abstraction with the simplest possible adapter.

| Task | Location | Done when |
| --- | --- | --- |
| Add static webhook adapter | `den-channels/adapters/webhook.rs` | Normalizes a documented JSON payload |
| Add inbound route | `POST /channels/webhook/{installation_id}` | Authenticated webhook can start a Den run |
| Add outbound final response mode | adapter/service | Sends final text to configured callback or returns synchronous response where appropriate |
| Add event dedupe | route/service | Replayed webhook id does not duplicate messages or runs |
| Add docs for generic webhook JSON | docs | A user can manually post a message into Den |

**Exit gate:** Integration test posts a webhook message and observes one persisted Den conversation turn.

## Phase C — First-party web chat

**Goal:** Provide a rich browser chat channel controlled by Den.

| Task | Location | Done when |
| --- | --- | --- |
| Token endpoint | `POST /channels/webchat/token` | Browser gets short-lived token; server secrets stay server-side |
| Conversation/message endpoints | `POST /channels/webchat/{conversation}/messages` | User text starts a Den run |
| Live event stream | SSE first; WSS later if needed | Message deltas/progress/errors stream to browser |
| Attachment upload or URL ingestion | web chat adapter | Images/files can be passed as `ChannelAttachment` |
| Capability-aware renderer | web chat adapter | Supports deltas, progress, errors, and final messages |
| Basic embeddable widget docs | docs/webchat or roadmap follow-up | Local test page can chat with a Bear |

**Exit gate:** Browser smoke test: token → send message → receive streaming assistant reply; no long-lived secret in browser.

## Phase D — Slack channel

**Goal:** Add a first-party Slack adapter without making Slack semantics leak into Den core.

| Task | Location | Done when |
| --- | --- | --- |
| Slack installation config | channel installation config | Bot token/signing secret stored securely |
| Slack events route | `POST /channels/slack/events` | URL verification, app mentions, DMs, and message events normalize to `ChannelInboundEvent` |
| Slack interactions route | `POST /channels/slack/interactions` | Buttons/actions normalize to `ChannelInteraction` |
| Thread binding | channel binding service | Slack channel/thread maps to one Den conversation |
| Message rendering | Slack adapter | Uses thread replies; optionally message updates for coalesced progress |
| Approval rendering | Slack adapter | Uses Block Kit buttons where possible |
| Retry/idempotency handling | Slack adapter/service | Slack retries do not duplicate runs |

**Exit gate:** Slack DM or mention starts a Bear run; reply lands in the same thread; replayed Slack event does not duplicate the turn.

## Phase E — WhatsApp via Twilio

**Goal:** Add WhatsApp as a policy-aware multimodal channel via Twilio first.

| Task | Location | Done when |
| --- | --- | --- |
| Twilio WhatsApp installation config | channel installation config | Account SID/auth token/sender/settings stored securely |
| Inbound webhook route | `POST /channels/twilio/whatsapp` | Twilio form payload normalizes to `ChannelInboundEvent` |
| Status callback route | `POST /channels/twilio/status` | Delivery state recorded in receipts |
| Text + media ingestion | Twilio adapter | `Body`, `MediaUrl*`, MIME, filenames map to `ChannelAttachment` |
| Outbound freeform send | Twilio adapter | Replies inside customer service window work |
| Opt-in/session/template policy | Twilio adapter/service | Proactive sends are blocked or routed to template flow when required |
| Fallback/error handling | Twilio adapter | Twilio errors become diagnostics and delivery receipt state |

**Exit gate:** Inbound WhatsApp text and image produce one Den turn; Den reply is delivered through Twilio; policy prevents invalid proactive freeform sends.

## Phase F — Chat within the macOS app

**Goal:** Add a native macOS app chat surface as a first-party channel, not as an armature unless/until it exposes trusted local work-surface tools.

The macOS app chat channel should initially behave like a rich first-party conversation channel:

- native message list and composer;
- Bear selection / current Bear context;
- Den-authenticated human identity;
- local notifications;
- optional attachment picking;
- streaming assistant replies;
- diagnostics suitable for end users;
- no local filesystem/editor/terminal tool authority by default.

| Task | Location | Done when |
| --- | --- | --- |
| Define `MacosAppChat` channel kind/capabilities | channel model | Capabilities distinguish app chat from ACP armature |
| App session/token flow | macOS app + Den route | App can obtain/refresh a Den-scoped chat token without exposing long-lived secrets |
| Native chat endpoints | Den channel route or shared webchat route | App can send messages and subscribe to live events |
| Conversation binding | channel binding service | App conversation maps to a Den conversation and survives app restart |
| Attachment handling | macOS app + channel model | User-selected images/files become channel attachments under Den policy |
| Notifications | macOS app | Completed replies or background events can notify the user |
| Offline/error UX | macOS app | Den unreachable, token expired, and retry states are visible and recoverable |
| Capability boundary docs | docs | App chat is documented as a channel; ACP/editor integration remains the armature path |

**Exit gate:** macOS app can start/resume a Bear chat, stream replies, preserve history across restarts, and show Den connectivity/auth diagnostics.

## Future: external adapter SDK or BearWire-backed channel adapters

Only revisit a real plugin/SDK model after at least two first-party channel adapters are production-stable.

Potential future paths:

1. **Out-of-process adapters over BearWire** — useful if a channel adapter needs run/session/tool/permission semantics and is deployed separately from Den.
2. **Small HTTP channel adapter contract** — useful for community webhooks without arbitrary code execution inside Den.
3. **WASM/dynamic plugins** — defer until there is clear demand and a security model.

## Non-goals for the initial plan

- Dynamic plugin loading.
- Public marketplace/hub.
- Arbitrary third-party code execution inside Den.
- Full Bot Framework clone.
- Universal rich-card schema beyond concrete channel needs.
- Making every channel pretend to be BearWire or ACP.
- Giving Slack/WhatsApp local workspace authority.

## Open questions

1. Should `den-channels` be a separate crate immediately, or start inside `den-runtime` until two adapters exist?
2. Should web chat use BearWire event names, a channel-specific event schema, or a small shared subset?
3. How should channel conversations select Bear stance by default?
4. How do channel identities map to Den humans and Bear memberships?
5. What is the minimum safe attachment retention policy for inbound media?
6. Should Slack app installation be per Bear, per Den server, or per organization/workspace?
7. How much of WhatsApp template management belongs in Den versus Twilio/Meta dashboards?

## Recommended first slice

Start with Phase A plus Phase B.

That gives Den a canonical channel model, persistent bindings, event dedupe, and a generic webhook adapter without committing to Slack/WhatsApp details too early. Then build web chat as the first rich channel before introducing Slack and WhatsApp policy complexity.
