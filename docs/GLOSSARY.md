**runtime session**

a conversation, background run, etc.

**trust profile**

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
the resource a runtime session is able to act upon

**acceptance criteria**
a job's durable definition of done
(`bear_job_criteria`); narrative, command (hard
gate), or check_ref. Injected into dispatch as the
success contract. See ADR-0034.

**task focus**
the ephemeral per-turn working state the native
loop uses to judge whether a candidate yield is
premature. A projection of governance mode ×
acceptance-criteria state × run/task status, not a
durable record. See ADR-0023 (re-homed by ADR-0035
and ADR-0039).

**continuation bias**
how aggressively a run continues vs. yields. Owned
by the governance mode, defaulted/modulated by the
trust profile — not defined by the profile. See
ADR-0039.
