# Session 47 — re-base onto PR #184/#185 + dead-store elimination revival (2026-08-22)

Base: `ms178/lccc` main `7811dc1` (merges PR #184 — ARM peephole battery,
scalar fmsub/fnmsub contraction, five codegen correctness fixes — and PR #185,
the session-46 MachInst narrowing-cast fix). Build: `fastbuild`, Rust opt-level
1, no LTO, two jobs, 4 GiB swap. Host: constrained VM without PMU.

## Fix: `eliminate_never_read_stores` was dead for no-FP functions with pushes

### Root cause (traced line by line)

`peephole/passes/dead_code.rs::eliminate_never_read_stores` detects function
prologues in two forms:

1. **Form 1 (frame pointer):** `pushq %rbp` + `movq %rsp,%rbp` + `subq $N,%rsp`
2. **Form 2 (no frame pointer):** `subq $N,%rsp` preceded by a `.cfi` directive
   within a fixed 3-line window

Two independent defects made form 2 unreachable for the common six-push no-FP
prologue (lccc allocates `%rbp` as an ordinary callee-saved register in no-FP
mode, so the push chain is `%rbx %r12 %r13 %r14 %r15 %rbp`):

- **Form-1 failure path skipped the prologue marker.** When `pushq %rbp` was
  NOT followed by `movq %rsp,%rbp` (the normal no-FP case), the loop jumped to
  `j + 1` — one line PAST the `subq $N,%rsp` — so the subq line was never
  examined by form 2.
- **Form-2's 3-line directive window missed.** Even when the subq was reached,
  the `.cfi_startproc` directive sat 7 lines back (behind the six pushes) and
  the fixed `i - 3` window missed it.

Net effect: every function with a callee-saved push chain silently skipped
dead-store elimination. The gzip CRC kernel kept a never-read
`movq %rax, 8(%rsp)` in its inner loop (the loaded table value's slot home
was stored but never loaded again).

### The fix

- Form-1's failure path now falls through (`i += 1`) so the `subq` gets its
  own form-2 examination.
- Form-2's directive scan now walks back up to 16 lines, treating `Push`
  lines (callee-saved saves) and nops as transparent, and stops at the first
  other line — a directive containing `cfi_startproc`/`cfi_def_cfa_offset`
  confirms the prologue; any other terminator fails closed (an interior
  `subq` cannot match a previous function's directive).

Both changes only widen the pass's reach to the prologue shapes it was
already designed for (the form-2 path with its full safety nets — escape
pre-scan, rsp-shift bail, read-range collection — was simply unreachable).

### Measured effect

gzip CRC kernel TU: `main` 118 → 112 instructions (the dead store plus the
frame-slot bookkeeping disappears from the hot check loop). Runtime on the
1048576-iteration loop unchanged (that loop was already clean); the win is
the pass being alive for the entire class of no-FP functions.

### Regression coverage

New unit test `test_never_read_store_eliminated_in_six_push_nofp_function`
pins the exact six-push no-FP shape through the full `peephole_optimize`.

## Follow-up work carried over (root causes now precisely documented)

1. **RA-03 Adler DO8** — `main` spills v540 (`sum1`, loop-weighted uses 1000)
   and friends because Phase 1's six callee-saved registers are claimed by
   six earlier-arriving hot values and mode-3 eviction refuses all victims
   (every next use is inside the incoming's span). Eviction-policy A/B
   (modes 2/4/5) is all worse (60/60/88 stack refs vs mode-3's 60); the real
   fix is RA-06 live-range splitting / next-use allocation, not another
   eviction knob.
2. **RA-01/IS-22 CRC** — the remaining 112-vs-50 gap vs GCC is (a) the PIE
   per-iteration `leaq sym(%rip), %rcx` base remat (lccc defaults to PIE;
   `sym(,%idx,scale)` is illegal there, GCC-style absolute SIB only works
   non-PIE) — needs an OP-34 relaxation allowing preheader register homes
   for indexed GlobalAddr bases — and (b) accumulator relay noise
   (`movq %rax,%rsi; movl %esi,%ecx`), which is the IS-08 load-cast-fold
   extension. Both require the gzip veto gate.
3. **Sema const-assignment (C11 6.5.16.2)** — `Declaration::is_const()`
   exists, but the parser folds all declarator qualifiers into one flag
   (`const int *p` vs `int *const p` are indistinguishable), so a correct
   modifiable-lvalue diagnostic needs per-pointer-level qualifier tracking —
   the same infrastructure as FE-02 (TBAA/restrict). Do not ship a naive
   `is_const` check: it would wrongly reject legal `const int *p; p = q;`.
4. **`movl %eax,%eax` artifact** — after `movslq (mem),%rax` the `movl
   %eax,%eax` is a REQUIRED zero-extension for U32 semantics; the right
   elimination is emitting the load at the destination width directly
   (load-cast fold, IS-08), not deleting the movl.

## Validation battery (all on the final tree)

| Gate | Result |
|------|--------|
| `cargo test --lib` | **1075 pass**, 6 ignored (incl. new NRS unit test) |
| Regression corpus | **382 pass / 0 fail / 7 documented skips** (389 total) |
| intrinsics (t128/t256/t512) | **3/3** |
| gzip 1.14 (pinned SHA `01a7b881…`) | **30/30** + doc roundtrip |
| zlib-ng 2.3.3 pinned gate | **ctest=8, roundtrip=0** |
| expat 2.7.1 (lccc-built) | **2/2 ctest** |
| phi CFG fuzz 0:600 × 3 levels | **1800/1800** |
| differential fuzz 0:500 × 3 levels | **1500/1500** |
| alias fuzz m32 0:540 × O2/Os | **1080/1080** |
| checkers (sema/bit-test/redundant-test/rmw-copyprop) | all pass |

No PMU evidence: VM screening only, per project policy.
