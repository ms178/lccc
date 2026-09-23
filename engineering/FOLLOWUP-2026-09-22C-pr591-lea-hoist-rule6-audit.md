# Follow-up: PR #591 (LEA hoist + 3-operand RORX) — critical audit review and fixes

Date: 2026-09-22 (session 61)
Base: `c7917656` (main, merges #591 `73a2ab1e` + #590 `0fbe7ef2` over `e08565c0`).
Branch: `main60` (on top of the session-60 width-fix commit `14ed72b4`).

---

## 1. Scope

An external audit of the merged PR #591 flagged five issues (F1–F5) plus
recommended fixes (T1–T8). This session:

1. re-verified every finding against the code — and, where the audit
   reasoned but could not execute, **executed** the claims;
2. fixed all confirmed issues with the most precise (not merely
   conservative) construction available in the pass's line-based design;
3. independently reviewed the rest of the PR (RORX emitter,
   `peephole_common.rs`, CI hooks, gate B-section) — no further defects.

## 2. Findings, verdicts, and evidence

### F1 (HIGH) — Rule 6's post-loop "kill" is unsound. CONFIRMED.

`hoist_loop_invariant_gpr_load`'s Rule 6 stopped its post-loop scan at the
first line where

```rust
kills_it = matches!(infos[k].kind, Other { dest_reg } if dest_reg == REG_NONE)
    || writes_family(&infos[k], text_k, dst_family)
```

but `writes_family` answers "ANY part of the family written" — which is
TRUE for lines that simultaneously **read** the family. Stopping the scan
at such a line makes the pass ignore every later mention of the
destination. Four distinct miscompile shapes, each pinned by a new unit
test in `loop_hoist_tests` (all five FAIL on the pre-fix tree — the bug is
mechanically demonstrated, not argued):

| test | shape | why it miscompiles |
|---|---|---|
| `gpr_hoist_refuses_rmw_after_loop` | `addq %rax, %r11` after the loop | RMW reads the hoisted value; result changes |
| `gpr_hoist_refuses_conditional_kill_after_loop` | `cmpl; je .skip; movq %rsi, %r11; .skip: movq %r11, %rax` | the `je` path skips the "kill" and still reads %r11 |
| `gpr_hoist_refuses_unknown_dest_read_after_loop` | `movq %r11, (%rbx)` (classified `Other { REG_NONE }`) | the unknown-dest line READS %r11 and stores the hoisted value |
| `gpr_hoist_refuses_partial_write_after_loop` | `movb $3, %r11b` then `movq %r11, %rax` | 7 high bytes still hold the hoisted value |

plus one more the audit's fix would have missed:

| `gpr_hoist_refuses_deep_break_past_kill` | loop-body `je .LBB9` with `.LBB9` landing AFTER the kill | the deep-exit path reads the destination without ever passing the kill |

### F2 (HIGH-gate) — the sha256 C1 check was vacuous. CONFIRMED, measured.

The K-load detector

```python
re.match(r'^\s*movl?\s+\(%r\w+, %r\w+d\)', l)
```

requires the index spelling to end in a literal `d` directly before `)`.
No real SIB load has that shape: against the actual `sha256_transform`
output (`movl (%rcx, %r10, 4), %ebp`) the match set was **empty**, so the
"K-table LEA hoisted out of the 64-round loop" check passed with nothing
to check. (Verified by running the regex on the real asm.)

### F3 — A2's alignment check had a `leaq` escape hatch. CONFIRMED.

A2 accepted "the nearest non-empty line above the header is `leaq`"
unconditionally — so the BAD placement (`.p2align` above the `leaq`, i.e.
the LEA inserted between the directive run and the header label) also
passed.

### F4 — no pin for the kill path. CONFIRMED.

The only pre-existing Rule-6 test used a pure READ after the loop (the
`reg_refs` veto) — the kill/`break` branch added by #591 had zero test
coverage, so an over-eager fix could silently delete the PR's flagship
optimisation (the sha256 tail hoist) without any test failing.

### F5 — label dicts overwrote on reuse. CONFIRMED.

`labels = {name: i for ...}` keeps the LAST occurrence; a backedge in an
earlier function paired with a later function's reused label, so the loop
span could be mis-measured across functions (and in the worst ordering,
not detected at all).

### Items the audit did NOT cover (independent review)

- **RORX emitter** (`emit_rotate_reg_direct`, both rorx sites in
  `emit.rs`): the direct-form gate reuses `operand_to_callee_reg`'s
  home-freshness discipline (`reg_assignments` + `home_clobbered`);
  `src == dest` is a legitimate 3-operand form (source read before the
  write); the typed 32-bit view reads the low 32 bits of a zero-extended
  home and the 32-bit write zero-extends the upper half, so the existing
  `emit_sext32_for_value` normalisation stays exact; `rorx` flag
  behaviour is unchanged (the staged path already emitted `rorx`). No
  defect.
- **`peephole_common.rs`** (new shared utility): word-boundary matching,
  SIB-aware comma splitting, `LineStore` — reviewed, clean, well-tested.
- **Gate B-section scope**: 32-bit staging goes through the generic
  `operand_to_callee_reg` (`movq`) path, so the `movq`-only staging regex
  has correct scope.
- **CI hooks** (`ci.yml`, `ci_local.sh`): gate registered in both,
  correct invocation.

## 3. The fixes

### 3.1 Rule 6 (T1, refined)

The kill predicate is now `pure_family_write(text_k, dst_family)` — the
existing text-exact predicate (mov*/lea with the destination spelled at
full 32/64-bit width and NO source in the same family, plus `xor %r, %r`
zeroing). The `Other { REG_NONE }` arm is deleted; such lines fall to the
`reg_refs`/implicit mention checks.

The scan stops at a kill only when it **dominates** every later read.
`kill_dominates(k)` is exact for the pass's line-based model:

- **(a)** no `Jmp | CondJmp | Call` in `(latch..=k]` — a conditional in
  the post-loop tail could skip the kill, and a call's clobber/argument
  effect on the family is unknown (inline asm, indirect jumps, and jump
  tables are already function-level vetoes via `loop_entry_is_unique`);
- **(b)** no jump from `(entry..=latch]` whose target label sits after
  `k` — a deep loop `break` would skip the kill.  Edges from at or before
  the hoist point are provably safe (those paths never saw the hoisted
  value), and a label after `k` with no such incoming edge (an epilogue
  block) is harmless.  Targets are resolved to the LAST label occurrence
  within the function (fail closed on an unresolved target).

When dominance cannot be proven the strict pre-#591 rule applies: any
mention after the loop vetoes, no scan stop.

**Deliberate deviation from the audit's T1.2**: the audit's literal
condition forbade `Ret` anywhere in `(latch..func_end)`. Every function
ends in a `ret`, so the kill-break would be dead code and the sha256 tail
optimisation (the reason the PR exists) would die with the bug. A `ret`
after the kill is a terminal sink, not a path that routes around it; the
family-2 128-bit-return read is still vetoed by the existing
`Ret && dst_family == 2` check.

### 3.2 Gate C1 (T3)

The K-load regex now matches the real SIB shape
`mov[lq]? (DISP)?(%base, %index[, scale])` with scales 1/2/4/8 only
(16 is unencodable, so the pattern cannot be satisfied by a malformed
line — verified against the real output: it finds all three indexed loads
in `sha.s`).  **A load-less result now FAILS** the check with the
candidate lines printed, instead of passing vacuously.

### 3.3 Gate A2 (T4)

The `leaq`-directly-above-header form is now accepted ONLY when no
`.p2align` sits above that `leaq`; a `.p2align` above the LEA means the
insertion displaced the alignment from the header label and fails the
check.

### 3.4 Gate label pairing (T7, refined)

Both A1 and C1 now pair each backedge with the label occurrence **nearest
above the jump** (rather than first- or last-wins).  A reused label name
can no longer stretch a measured loop span across functions.

## 4. Negative verification (a gate that cannot fail is useless)

Each fixed check was run against synthetic BAD inputs to prove it now
fires:

- C1: K-LEA inside the loop → reports 1 leak (was: vacuous pass);
  no SIB loads at all → reports -1 with candidates printed.
- A2: `.p2align` above `leaq` above header → reports displaced (was:
  pass).
- A1: two functions reusing `.LBB2`, K-LEA inside function 1's loop →
  reports 1 leak (old last-wins dict: 0, undetected).

## 5. Validation

- `cargo test --lib`: **3139 passed / 0 failed / 7 ignored**
  (3133 baseline + 6 new rule-6 pins).
- `tests/regression/check_lea_hoist_rorx.sh`: **7/7 PASS** with a
  non-vacuous C1.
- Full regression suite: **745 passed / 0 failed / 8 skipped,
  AB-diff 0** (the 8 skips are the standing ELF32-execution-runner and
  kernel-tree skips; environment note — the harness wipe had removed the
  32-bit dev files, restored by reinstalling `libc6-dev-i386`, which is
  what provides the traditional `/usr/include/bits` path on Debian).
- `scripts/ci_local.sh`: full CI mirror run (this session's green-CI
  gate).

## 6. Follow-up (unchanged from the 09-22B plan)

RA-2 phi-chaining (Part 2) → RA-3 pressure-aware greedy scan (dominant
residual lever per the census) → RA-4 `Cmp(and(x,y),0)` fold → RA-5
absolute-address stores in `-m16` → RA-6 copy+subreg-extend fold.
