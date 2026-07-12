# Phase 1 Web Chat and Onboarding Plan

**Status:** Active Phase 1 product slice.

This plan splits web-chat onboarding out of [`PHASE1_NATIVE_PRODUCT_DEBT_PLAN.md`](PHASE1_NATIVE_PRODUCT_DEBT_PLAN.md). It covers the first user path into a Den-native Bear, not Slack/WhatsApp/macOS channel work.

Related plans:

- [`DEN_CHANNELS_IMPLEMENTATION_PLAN.md`](DEN_CHANNELS_IMPLEMENTATION_PLAN.md) — broader channel layer.
- [`DEN_RUNTIME_PLAN.md`](DEN_RUNTIME_PLAN.md) — native runtime path.
- [`PERSONALIZATION_PLAN.md`](PERSONALIZATION_PLAN.md) — safe memory/personalization behavior after onboarding.

## Goal

Make first-run web use boring and obvious: a new user signs in, gets access to an appropriate Bear, lands in first-party web chat, and receives a streamed response with clear membership and configuration errors.

## Scope

### 1. Personal Bear assignment/provisioning

- Support assigning an existing Bear or provisioning a Personal Bear from a native template/config.
- Make the selected Bear and membership explicit in UI.
- Use `bear_id`/slug and native stances, not legacy runtime ids.

### 2. First-run guidance

- Explain what the Bear can and cannot do on the web channel.
- Invite safe personalization without implying blanket memory capture.
- Surface missing setup: no Bear, no membership, Den unavailable, model unavailable, or auth failure.

### 3. Browser chat happy path

- Keep `/bear/{slug}` or the current first-party web chat route as the reference browser client for `/v1/chat/send` SSE.
- Ensure membership failure is explicit and actionable.
- Ensure streamed responses and terminal errors are visible to the user.

## Non-goals

- Do not build a new chat client if the existing Den-hosted web chat can be fixed.
- Do not solve Slack, WhatsApp, Twilio, ACP, or macOS app chat here.
- Do not add a second memory UX or Letta memory block UI.
- Do not auto-enable local armature tools for web users.

## Implementation steps

1. Inventory current sign-in, Bear selection, and web-chat routes.
2. Define the minimal first-run decision tree:
   - user already has Bear membership;
   - user needs assignment;
   - operator/bootstrap setup is incomplete;
   - request is forbidden.
3. Implement or clean the first-run redirect into chat.
4. Add onboarding copy that reflects native Den, stances, and safe personalization.
5. Add clear empty/error states for missing Bear, missing membership, failed stream, and unavailable model/runtime.
6. Leave one lightweight smoke check for the first-run decision logic if it is non-trivial.

## Acceptance criteria

- A new user can sign in, get or select a Personal Bear, land in chat, and see a streamed response.
- A non-member receives a clear denial instead of a silent or confusing failure.
- Onboarding copy does not mention Letta, Codepool, MemFS, or memory blocks as current product mechanics.
- Web chat remains a channel surface, not an armature/local-tool surface.
