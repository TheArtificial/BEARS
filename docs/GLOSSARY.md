**runtime session**

a conversation, background run, etc.

**trust stance**

a bundle of capability boundaries
used to enforce the Rule of Two

**governance mode**
the run-scoped supervision contract for a
runtime session: how it is supervised right now
(interactive, grace, autonomous_continuation,
observational, frozen). Owns continuation bias —
how hard to drive vs. when to yield. See ADR-0039.

**armature**
the actuators and signals available to a run on a
given turn: client-side ACP tools vs. server-side
sandbox tools, plus the channel signals the
supervisor can observe. See ADR-0039.

**work surface**
the durable resource a runtime session is able to
act upon. **Internal / model-facing term only** — not
shown to users; the UI presents typed cards
(Repository, Design, Server, Document, …) under
Connections. See ADR-0006 and ADR-0040.

**connection**
a Den-level, owner-scoped authenticated link to an
external provider (GitHub, Figma, Google, SSH).
Set up once, reusable across resources, Bears, trust
stances, and governance modes. A work surface is
reached through a connection when externally backed.
See ADR-0037 and ADR-0040.

**acceptance criteria**
a job's durable definition of done
(`bear_job_criteria`); narrative, command (hard
gate), or check_ref. Injected into dispatch as the
success contract. See ADR-0034.

**task focus**
the ephemeral per-turn working state the in-process
Den loop uses to judge whether a candidate yield is
premature. A projection of governance mode ×
acceptance-criteria state × run/task status, not a
durable record. See ADR-0023 (re-homed by ADR-0035
and ADR-0039); ADR-0050 and the Agent Loop Control
implementation plan define the concrete budget,
checkpoint, and Docket reconciliation machinery.

**continuation bias**
how aggressively a run continues vs. yields. Owned
by the governance mode, defaulted/modulated by the
trust stance — not defined by the stance. See
ADR-0039. Tool-call budgets, failure thresholds, and
checkpoint nudges are Agent Loop Control concerns;
see ADR-0050.
