# Matching ACP and the Den Runtime: the Concurrency Model

Audience: developers comfortable with concurrency/async concepts but new to this codebase and to Rust's async model. This explains how a single ACP prompt is matched to Den's turn runtime when client-side tools are involved, and why the concurrency is structured the way it is.

See also: [`acp-runtime-invariants.md`](acp-runtime-invariants.md), [`acp-lessons.md`](acp-lessons.md), [`acp-troubleshooting.md`](acp-troubleshooting.md).

## The shape of the problem

ACP (the protocol our IDE/client integrations speak) turns a single user prompt into a **long-lived, multi-party exchange**, not a request/response. One prompt can involve three concurrent actors:

1. **The client** (IDE/editor) — opens the prompt as a server-sent-events (SSE) stream and reads events off it.
2. **Den** — runs the "turn": it talks upstream to the model runtime, emits events down the SSE stream, and persists state.
3. **The model runtime** — streams back assistant text, tool-call requests, and stop/pause signals.

The wrinkle that drives everything: some tools execute **on the client, not in Den**. When the model asks to read a file, Den emits a `tool_request` down the stream; the *client* performs it and POSTs the result back to a **separate HTTP endpoint** (`/tool-results`). Den must then **resume** the model turn with that result.

So a single logical turn spans **two independent HTTP requests** — the still-open prompt stream and the later tool-result POST — that have to rendezvous. That rendezvous is the core concurrency challenge.

## How the two requests rendezvous

Den keeps a small **in-memory registry of "open tool obligations,"** keyed by `(session, tool_call_id)`. It is the shared state both requests coordinate through:

- When the turn emits a `tool_request`, Den **registers an obligation** in this registry. The obligation holds the *sending* half of a one-shot channel.
- The prompt stream keeps the *receiving* half and **parks** — it suspends, waiting for that channel to fire, instead of finishing the turn.
- When the `/tool-results` POST arrives, its handler **looks up the obligation** in the registry and fires the channel. That wakes the parked stream, which settles the obligation and continues the turn.

Two HTTP handlers, one shared registry, a one-shot channel as the handoff. The registry is the single source of truth for "is this turn waiting on the client?" — and it is what determines whether an incoming tool-result POST is honored or treated as late.

## Why timing matters

In Rust's async model (tokio), an SSE response body is **lazy**: the turn's state machine only advances when something *reads* the stream. A client that reads continuously naturally drives the turn forward — it sees the `tool_request`, Den registers the obligation, and the client then posts the result. Registration precedes the POST.

That ordering, though, is an emergent property of "the client is reading," not something the protocol guarantees. A result can arrive **before** the obligation is registered — a fast client, a reconnect, or any caller that posts without first draining the stream. If registration is tied to stream consumption, the registry lookup can miss and the result is rejected as late, leaving the turn parked until it times out.

The runtime therefore treats one ordering property as an invariant to enforce, not to hope for: **the obligation must exist the moment the prompt response is returned.**

## The model: register eagerly, stream lazily

Den separates the one thing that must happen eagerly from everything that can stay lazy:

- **Eager, before the response is returned:** Den drives the turn just far enough to register any tool obligation, bounded by a short timeout so a slow turn (or one with no tool call) doesn't delay the stream from opening. This runs on the request handler's own task — not a detached background task.
- **Lazy, afterward:** once the obligation exists, the rest of the turn (waiting for the result, resuming the model, streaming output) is driven by the client reading the body, exactly as the protocol intends.

This guarantees the rendezvous invariant while preserving normal streaming semantics for everything after registration. A tool-result POST is honored regardless of when it arrives relative to the client starting to read.

## Why not run the turn on a background task

A natural alternative is to run the whole turn on a **detached background task** feeding a buffer, fully independent of whether the client reads. The reason Den does not do this is a useful constraint to understand: a detached turn that parks waiting for a client result **holds resources** — database connections, a per-turn "active turn" guard — and nothing is tied to the client's lifecycle to release them. Under many concurrent turns that exhausts the connection pool and leaves stale "active" state.

The principle: **independent execution requires explicit cancellation.** A background turn task is sound only if its lifetime is bound to the client connection (cancel-on-disconnect), so resources release deterministically. That is the right foundation for a turn that must *survive* a reconnect, but it is a larger mechanism than matching ACP's tool-result rendezvous requires. The bounded eager-drive achieves the necessary invariant without it.

## A note on the eager-drive bound

The eager-drive is bounded by a short fixed timeout. This is adequate because registration is effectively instant in practice, but it is a duration guess rather than a guarantee. A more precise design replaces it with an explicit **registration signal**: the registration path notifies the handler the instant the obligation exists, so the handler waits exactly that long and no longer.
