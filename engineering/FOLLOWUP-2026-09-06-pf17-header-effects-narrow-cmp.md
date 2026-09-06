# PF-17 follow-up: header-effect safety and narrow comparison assembly

**Date:** 2026-09-06

**Base:** upstream `origin/main` `5d4a5462b520c9407f2898f156680663e902dbcc`

**Working commit before this unit:** `bfb0ef1c09ee273c257a8711ff4e986bda7c4f17`

**Scope:** make the loop-rotation opt-in path correct for a repeated header
side effect and make rotated I8/I16 comparisons assemble and execute.  This is
a correctness repair; loop rotation remains opt-in.

## Result

Two independent PF-17 blockers are fixed.

1. `loop_rotate` now refuses a candidate when *any* non-`Phi` instruction in
   the header is outside the closure cloned to the latch.  This is deliberately
   conservative: arbitrary header instructions have observable order and may
   not be silently dropped from later guard evaluations.
2. The classic x86 comparison emitter now names the width-matched `%rcx`
   sub-register after staging a narrow operand in `%rcx`: `%cl` for I8/U8,
   `%cx` for I16/U16, `%ecx` for I32/U32, and `%rcx` otherwise.  The repair
   covers both normal compare emission and compare-replay emission.

The complete regression corpus is now clean with rotation explicitly enabled:
**666 pass, 0 fail, 13 skip-compare, 0 skip-run, 679 total**.  The same result
holds with the normal global environment (the two intended opt-in test
sidecars still exercise rotation).

## Root-cause analysis

### Header work that was not cloned

The pre-existing transform retained the header as the one-shot zero-trip guard
and copied only the transitive SSA closure feeding its `CondBranch` condition
to the latch.  That is valid only when the header has no independently
observable work.  For this source shape:

```c
for (int i = 0; (trace[i] = 7 + i * 13), i < n; ++i)
    sum += data[i] ^ trace[i];
```

`i < n` needs only the comparison, not the store.  The old transformation ran
the store at the initial header guard, then used a cloned comparison at the
latch.  `trace[1]` through `trace[n]` stayed stale instead of being written at
every later guard evaluation.  This was visible in the prior red-team result
for `k_header_store` (`header=-3795 -38` versus GCC `header=-3795 12`).

The right short-term fix is a rejection, not an attempt to clone the store:
there is no general proof in this pass that every remaining header instruction
is pure, reorderable, or safe to replay after the body.  New **Guard F** checks
exactly that every non-`Phi` header instruction appears in the cloned closure.
`Phi`s remain exempt because the existing transformation reconstructs their
recurrence at the latch.  A trace of the focused test shows Guard F declining
the header before the transform mutates IR.  The first reported unclosed item
is an address-conversion `Cast`; the header also contains the deliberately
unclosed `Store`.  Either is sufficient to make partial cloning unsound.

This restriction leaves ordinary pure canonical guards eligible.  Debug runs
still report successful rotations for `loop_rotate_default_enable`,
`loop_rotate_stale_phi_pred`, and `loop_rotate_while_dec`.

### Narrow compare: not a MachInst defect and not merely a peephole defect

The I8/I16 failures initially appeared after rotation as:

```asm
cmpb %rsi, %r8b
cmpw %rsi, %r8w
```

GAS rejects those operands as different widths.  Bisection ruled out the
MachInst route: `CCC_NO_MACHINST=1`, `CCC_MI_DISABLE_KINDS=cmp`, and
`CCC_MI_DISABLE_KINDS=cast` all retained the failure.  Disabling peephole phase
2 made the final spelling look different, but its pre-peephole form was
already invalid:

```asm
movq %rsi, %rcx
cmpb %rcx, %al
```

The phase-2 `narrow_copy_fold` pass only propagated the full register name
back through the move (`%rcx` to `%rsi`) and thereby exposed the same primary
backend defect more clearly.

`emit_int_cmp_insn_typed` and `emit_int_cmp_replay_insn` correctly choose
`cmpb`/`cmpw`, but their scratch fallback formerly chose `%ecx` only for
I32/U32 and `%rcx` for every other type.  `%rcx` is the staging register, not
a valid byte/word operand spelling.  `cmp_secondary_reg` centralizes the
width mapping and both routes consume it.  Full-width staging remains correct:
a `movq` preserves the low byte and low word, and `cmpb`/`cmpw` read exactly
those bits.  I32/I64 code generation is unchanged.

With the repair, the rotated narrow regression emits valid equivalent forms:

```asm
cmpb %sil, %r8b
cmpw %si, %r8w
```

The text peephole may coalesce the `%rcx` relay back to the original ABI
register, but it preserves the selected sub-register width.  An independent
`gcc -c` assembly gate passed; `lccc -S` success alone is not considered a
validation result.

## Tests and evidence

| Gate | Result |
|---|---|
| `./scripts/build_lccc_o1_j2.sh` | pass; Rust 1.98.1, Cargo jobs=2, Rust opt-level=1, warnings denied |
| `cargo fmt --all -- --check` | pass |
| `RUSTFLAGS='-D warnings' cargo test --profile fastbuild --all-targets --locked -j 2` | 1,997 pass, 0 fail, 6 ignored (2,003 total) |
| `RUSTFLAGS='-D warnings' cargo clippy --profile fastbuild --all-targets --locked -j 2 -- -D warnings` | pass, no diagnostics |
| focused `loop_rotate_header_side_effect` regression, SSA validation | 1 pass / 0 fail; LCCC and GCC both print `4 1249 1249 1` |
| focused `vectorize_narrow_iv_reduction` regression with rotation opt-in | 1 pass / 0 fail; compiles, links, runs, and stdout-matches GCC |
| independent GAS gate on generated narrow assembly | pass (`gcc -c`) |
| full regression corpus, `CCC_LOOP_ROTATE=1 CCC_VALIDATE_SSA=1`, two workers | 666 pass / 0 fail / 13 skip-compare / 679 total in 58 s |
| full regression corpus, normal global environment, SSA validation | 666 pass / 0 fail / 13 skip-compare / 679 total in 55 s |
| `LCCC=target/release/lccc bash scripts/check_benchmark_outputs.sh` | 156 pass / 0 fail / 0 skip |

Persisted diagnostic evidence outside the worktree is under
`/home/user/artifacts/perf-2026-09-06/pf17/`, including the fixed narrow
assembly, Guard-F debug trace, and both full-corpus JSON/log pairs.

## Performance interpretation and default-enable decision

This unit makes no claim of a runtime speedup.  The narrow repair changes only
the register spelling in a single comparison; it preserves the one-comparison
shape and turns an otherwise unassemblable program into executable code.
Guard F may decline some unsafe rotation candidates, deliberately giving up a
potential branch-layout improvement where correctness cannot be proven.  The
pass is still opt-in, so normal production code generation has no new default
rotation exposure.

A default-on decision remains blocked on broader differential fuzzing and
representative performance measurements.  The clean 679-test opt-in corpus is
a necessary correctness milestone, not sufficient evidence to enable a
transformation globally or to claim parity with GCC 16.2, Clang 23.1, or
ICX/ICC on the extended performance corpus.

## Follow-up

1. Keep the new focused side-effect and narrow-IV tests opt-in permanently;
   they prevent both a silent semantic loss and a `-S`-only false positive.
2. Before considering default-on rotation, run the extended/fuzz corpus and
   code-generation/performance screens with a fresh compiler binary, not a
   restored target directory.
3. Any future expansion that supports header effects must model ordered replay
   explicitly.  It must not weaken Guard F by treating non-closure header work
   as dead merely because it does not feed the condition.
