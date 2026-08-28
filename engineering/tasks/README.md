# Active work queue

**Lifecycle.** One file per actionable work item. A file is created when the
item enters the active wave and **deleted in the same commit as its fix**
(landed items are recorded in [`../agent/BACKLOG.md`](../agent/BACKLOG.md) §5
and git history; the queue README table is the only status journal here).
Claim a task by moving it into your session summary; do not edit a task
another agent owns.

**Scope rule.** A task file must contain everything an agent needs to start
without archaeology: objective, files, acceptance gate, validation battery,
and do-nots (with a pointer into [`../DECISIONS.md`](../DECISIONS.md) when a
naive version of the work was already measured negative).

## Active wave 1 (P0/P1, parallel-safe by subsystem)

| Task | Area | Files touched (conflict isolation) |
|------|------|------------------------------------|
| [TASK-RA-06A-RELOAD-AT-USE.md](TASK-RA-06A-RELOAD-AT-USE.md) | Register allocation | `live_range.rs`, `regalloc.rs`, `split_ranges.rs` |
| [TASK-OP-05B-MULTI-STORE-SCATTER.md](TASK-OP-05B-MULTI-STORE-SCATTER.md) | Vectorizer | `vectorize.rs` |
| [TASK-PF-17-LOOP-ROTATE-DEFAULT.md](TASK-PF-17-LOOP-ROTATE-DEFAULT.md) | Loop transforms / phi-elim | `src/passes/loop_rotate.rs`, `src/ir/mem2reg/phi_eliminate.rs` |
| [TASK-LK-19-IFUNC.md](TASK-LK-19-IFUNC.md) | Sema/lowering/linker | `sema/`, `ir/lowering/`, `linker_common/` |
| [TASK-LK-24-PIE-STARTUP.md](TASK-LK-24-PIE-STARTUP.md) | Linker/crt | `ld`, `crt`, TLS init |
| [TASK-LK-29-STMT-EXPR-ASM-TYPING.md](TASK-LK-29-STMT-EXPR-ASM-TYPING.md) | Sema types | `sema/` |
| [TASK-FE-25-MARCH-NATIVE.md](TASK-FE-25-MARCH-NATIVE.md) | Driver/codegen options | `driver/`, `common/` |
| [TASK-MS-08-RA-CONFIG.md](TASK-MS-08-RA-CONFIG.md) | RA plumbing | `regalloc.rs` + env knobs |
| [TASK-FIX-DASH.md](TASK-FIX-DASH.md) | RISC-V | needs qemu-user environment |

Conflict warning: TASK-MS-08 and TASK-RA-06A both touch `regalloc.rs` —
run them in that order or in separate sessions, never interleaved.
TASK-LK-19 and TASK-LK-24 both exercise the glibc campaign; coordinate
the oracle/ld.so builds.

## Wave 2 (queued, files created when wave 1 drains)

PF-06 isort secondary IV · IS-11 andn+cmov ffs · IS-29a remaining FP
call-result class · AB-01/02 SysV SSE-class aggregates · OP-36 OnExpr tag
coverage · MS-11 glibc `make check` triage harness · KERNEL objtool B1 +
vmlinux link (LK-28) · RA-01b marching pointers · LK-30 system
instructions · PERF-3 TLS addressing.

## Full open-item catalog

Every open item with evidence/accept/do-not columns lives in
[`../agent/BACKLOG.md`](../agent/BACKLOG.md) §1 — the wave tables above only
select what is *next*. Pull a BACKLOG item directly when your session has
capacity and no queue file exists yet; create the file using the template
below.

## Task file template

```markdown
# TASK-<ID> — <slug>
IDs: <BACKLOG ids> · Priority: <P0..P3> · Base: <origin/main sha>

## Objective
<one paragraph>

## Files
<exact paths>

## Acceptance
<measurement gates, kill switches, expected numbers>

## Validation battery
<unit / regression / fuzz / oracle commands that must pass>

## Do not
<constraints, with DECISIONS.md pointers where a naive version failed>
```
