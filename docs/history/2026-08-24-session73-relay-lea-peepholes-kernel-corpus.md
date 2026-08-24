# Session 73 — kernel-corpus oracle harness, move-relay and windowed lea peepholes

Date: 2026-08-24
Base: `d35353c` (latest `ms178/lccc` main; PRs #219, #220, #221 included).
Build: `scripts/build_lccc_fast.sh` (fastbuild profile, Rust 1.98.0, `-j2`,
8 GiB swap via `scripts/ensure_swap.sh`).

## Rebase audit of the earlier session-66/67/68 patch series

The x86 `%rsp`-write peephole fix (`push`/`pop`/`leave`/`enter` in
`eliminate_redundant_leaq::written_families`) and the VLA-aggregate varargs
by-reference ABI (caller classification + pointer-returning `va_arg` +
`typeof(vla_local)` size resolution) are **already upstream** — merged through
PR #219/#220 and documented in
`docs/history/2026-08-23-session72-alt-vla-rsp-audit.md`, together with the
regressions `tests/regression/push_rsp_leaq_dedup.c` and
`tests/regression/vla_struct_varargs.c`. Re-verified on this base:
`written_families` reports `RSP` for `push`, `reg + RSP` for `pop`, and
`RSP, RBP` for `leave`/`enter`; `lower_va_arg_struct` returns an `IrType::Ptr`
`VaArg` for runtime-sized aggregates. Nothing from that part of the series is
re-applied here — only the still-missing performance work is.

## Shipped

### Kernel corpus + fast oracle proxy

`tests/benchmark/kernel_corpus/*.c` (15 self-contained kernels: adler32 DO8,
byte sum, table CRC, strlen, max, dot, bswap, 64-byte copy, clz, ffs, swap,
FNV hash, strcmp, insertion sort, zero-byte count) and
`scripts/kernel_count.py`, which counts instructions inside the measured
function for LCCC and the system GCC at `-O2` and prints the per-kernel delta.
Repo-relative paths, `LCCC`/`GCC`/`CFLAGS` overridable from the environment.
Official scoreboard numbers still come from `scripts/godbolt.py`; this harness
exists so one peephole change can be measured in under a second.

### `src/backend/x86/codegen/peephole/passes/relay_and_lea.rs` (new, default-on)

1. **`eliminate_move_relays`** — `movl %eax, %r10d; addl %r10d, %r8d`
   becomes `addl %eax, %r8d`. The accumulator backend emits this relay for
   nearly every widened byte load.
2. **`fold_lea_into_load`** — windowed single-base `lea` fold:
   `leaq 1(%rbx), %r10; …; movzbl (%r10), %r14d` becomes
   `movzbl 1(%rbx), %r14d`. `local_patterns::fold_lea_into_memory_op` already
   folds this shape, but only when the memory operation is the *immediately*
   following instruction and only when the temporary is dead for the rest of
   the function (`fam_read_after`). Unrolled byte loops violate both: the
   consumer is 2-3 instructions away and the temporary is recycled by the next
   `lea`. This is exactly the "const-offset loads do NOT fold in unrolled
   loops" gap recorded in the earlier adler32 oracle run.

Both passes are gated by the standard skip set (`CCC_PEEPHOLE_SKIP=move_relay`,
`lea_load_window`).

#### Deadness contract (the part that has to be right)

A register may only be dropped when it is provably dead after the rewritten
use. Two independent proofs are accepted, and the union is what makes both
loop shapes foldable:

1. **Block-local write-before-read** — the first event on the family after the
   use is a full write (`mov`/`lea`-class, source not reading the family).
   Any barrier (label, jump, call, `push`/`pop`, `ret`, directive) first aborts
   the proof. This covers the unrolled adler32 body, where the temporary is
   immediately recycled.
2. **Whole-function textual uniqueness** — inside the enclosing
   `.cfi_startproc`/`.cfi_endproc` range the only lines naming the family are
   the ones being rewritten or deleted. This covers the loop-tail relay in
   `sum8`, where the copy dies behind the back edge, so no forward scan can see
   the redefinition.

Hazards closed while building proof 2 (each has a unit test):

- ABI-implicit reads are invisible to a text scan. `call`, an indirect jump and
  a tail `jmp symbol` read `%rdi %rsi %rdx %rcx %r8 %r9`, `%rax` (variadic
  vector count) and `%r10` (**static chain**, see
  `src/backend/x86/codegen/nested_fn.rs`); `ret` reads `%rax:%rdx`. Proof 2 is
  refused for those families in functions containing such a transfer. Without
  this, `movq %rax, %rdi; addq %rdi, %rsi; call bar` would have lost the
  argument set-up.
- `div`/`mul`/`cltq`/`cqto`/`shld` traffic on `%rax`/`%rcx`/`%rdx` is likewise
  invisible: any implicit-usage line aborts both proofs for families 0-2.
- Liveness is family-wide (`%ecx` write kills `%rcx`), source rewrites are
  width-exact (the use must name the copy's destination text verbatim, so
  `movl %eax,%r10d` can never feed `addq %r10,…`), only bare `(%T)` operands
  absorb a LEA (`8(%T)` would splice into `8DISP(%base)`), and the rewritten
  line must not mention the dropped register at all.
- `%rsp`/`%rbp` are excluded; pinned lines (inline asm, parameter pre-stores)
  are never touched or crossed.

## Measurements (`scripts/kernel_count.py`, LCCC `-O2` vs GCC 15 `-O2`)

Corpus total: **375 → 363** instructions against GCC's 264.

| kernel | before | after | GCC |
|---|---:|---:|---:|
| adler8 | 105 | 98 | 63 |
| sum8 | 15 | 14 | 13 |
| crc32k | 30 | 29 | 18 |
| my_strlen | 14 | 13 | 10 |
| hsh | 20 | 19 | 13 |
| scmp | 27 | 26 | 22 |
| maxv | 19 | 19 | 27 (LCCC wins) |
| dot | 16 | 16 | 17 (LCCC wins) |

Unchanged kernels are omitted. `isort` (+24) and `cntz` (+9) are the two
largest remaining gaps; both are recorded in `engineering/agent/BACKLOG.md` as
PF-06 and PF-03.

## Designed but deliberately NOT shipped

`fold_cmov_increment` (setcc/movzbl + `mov`/`add $1`/`test`/`cmovne` → `add` of
the 0/1 boolean) is a real pattern in `cntz`, but the emitted `cmovneq %r8,%rbx`
**preserves** the upper 32 bits of `%rbx` on the not-taken path while the
replacement `addl` zero-extends. The rewrite is only valid if every definition
of the cmov destination is a 32-bit (zero-extending) write, which a textual
peephole cannot establish locally. No dead or unsound pass is shipped;
the item is PF-03 in the backlog with that obstacle written down.

Likewise, the previously proposed opt-in dead-reg-move pass is not included:
its block-local scan stops at a label while the fallthrough can still read the
copy. It needs dominance-correct liveness (PF-04).

## Validation

```text
cargo test --profile fastbuild --lib          1111 passed, 0 failed
                                              (10 new relay_and_lea tests)
tests/regression/run_regression.sh            462 passed, 2 skipped, 12 failed
    The 12 failures are identical WITH and WITHOUT the new passes
    (CCC_PEEPHOLE_SKIP=move_relay,lea_load_window): aarch64 cross-gcc absent,
    i686 ELF interpreter absent, plus pre-existing FP/SIMD failures on this
    host image.
tests/correctness/run_correctness.py          50/50 vs GCC
tests/fuzz/differential_fuzz.py               seeds 0:750 x {O0,O2,O3,Os}
                                              3000 programs, 0 failures
tests/fuzz/phi_cfg_fuzz.py                    seeds 0:120 x {O0,O2,O3}
                                              360 programs, 0 failures
kernel corpus execution check                 all 15 kernels compiled by LCCC at
                                              -O0/-O1/-O2/-O3/-Os, linked against a GCC
                                              driver: output identical to the
                                              all-GCC build
```

## Follow-up (priority order)

1. RA-03 / PF-05: adler32 keeps eight byte temporaries live for the `s2`
   recurrence and spills `s1`/`s2` — 98 vs GCC 63 is almost entirely this.
2. PF-06: `isort` shift-store recognition (+24, the largest single gap).
3. PF-07: crc32 `xorl table(,%reg,4)` SIB fold.
4. PF-03 with a zero-extension proof for the cmov destination.
5. PF-04: dominance-correct liveness, which would also let both passes here
   drop the conservative barrier stop.
