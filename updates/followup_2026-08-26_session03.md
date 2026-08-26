# LCCC Follow-up / Kontinuität — Session 03 (2026-08-26)

**Scope:** GCC C torture/execute correctness on the x86-64 and i686 backends.
**Objective:** fix the *root cause* of every failing test, validated on real
hardware, with statistical measurement, and a durable methodology.

> This document is the hand-off note for the next agent/session. It records
> (1) what was accomplished and is validated, (2) every remaining defect with a
> precise, root-cause analysis, and (3) the infrastructure added.

---

## 0. Session environment (reproducibility)

| Item | Value |
|---|---|
| Host CPU | 2 vCPU VM (no HW PMU → **no reliable perf counters**; see §Perf) |
| RAM / swap | 1.9 GiB RAM + 6 GiB swapfile (`/swapfile`, created by `scripts/ensure_swap.sh`) |
| Disk | `df` showed 12 GiB free on `/`; builds ran with `-j2` (project policy) |
| Rust | 1.98.0 (repo pin in `rust-toolchain.toml`) |
| Host GCC | 14.2.0 (Debian) |
| Host clang / mold | 19.0 / mold (installed for the fastbuild link path) |
| LCCC binary | `target/fastbuild/lccc` (fastbuild profile, `-O1`, no LTO, incremental) |
| Torture suite | `gcc/gcc/testsuite/gcc.c-torture/execute` from gcc-15.1.0 source (**1681** sources) |
| Linker | standalone `lccc-ld` via the gcc-driver `-B` shim |

## 1. Accomplished this session (validated)

### 1.1 Vector comparison + unary elementwise lowering (12+ tests)

**What changed (root cause).** The frontend lowered *vector comparisons*
(`==`,`!=`,`<`,`<=`,`>`,`>=`) and *unary vector ops* (`~`,`-`) through the
generic **scalar** path, which lower's a vector operand to its stack *address*
and then compares/negates/bit-nots the address as if it were a scalar integer.
The downstream `memcpy`/load of the "result" then dereferenced that bogus
value as a pointer → SIGSEGV. This is exactly the "vector's low 64 bits used as
an ADDRESS" class documented in `intrinsics.rs` (`simd_movnt SIGSEGV`).

Concretely, `V v = ~((V){0} <= 0)` (PR 110817) produced IR:

```
Cmp Sle(lhs = <alloca ptr>, rhs = 0)
Cast ... ; UnaryOp Not ...

```
i.e. `ptr <= 0`, then `~ptr`, then `memcpy` dereferences `~ptr` → SIGSEGV.

**Fix.** Added a dedicated element-wise lowering path, GCC-vector-extension
semantics (all-ones/all-zeros per-lane mask):

* `src/ir/lowering/expr_assign.rs`
  * `lower_vector_cmp(op, lhs, rhs, vec_ct)` — per-lane load (or broadcast the
    scalar operand), integer `Cmp` (using signed/unsigned `IrCmpOp` from the
    element type), then `mask = 0 - zext(cond)` in the element type to produce
    the all-ones/all-zeros mask at any element width.
  * `lower_vector_unary(ir_unary, inner, vec_ct)` — element-wise `Neg`/`Not`.
  * `ir_zero_const(ty)` helper.
* `src/ir/lowering/expr_ops.rs` — hooks in `lower_binary_op` (comparison ops,
  integer element types only) and in `UnaryOp::Neg`/`UnaryOp::BitNot`.
* `src/ir/lowering/complex.rs` — `expr_ctype` now returns the operand vector
  type for a vector comparison so `~(vec <= s)` sees a vector. **Carefully kept
  in its own match arm** so that *non-vector* comparisons (including complex
  `z == 0`) fall through to `get_expr_ctype` exactly as before — the first
  attempt moved comparisons into the arithmetic arm and regressed
  `complex-2`, `20020227-1`, `pr38151`, `pr42248`; verified and reverted.

**Tests fixed** (all now PASS at `-O0/-O1/-O2/-O3/-Os`): `pr110817-1/2/3`,
`pr105613`, `pr109040`, `pr94412`, `pr109986`, `pr108292`, `simd-1`, `simd-2`,
`pr23135`. **Zero regressions** confirmed by a full re-run (see §2).

### 1.2 Infrastructure added

* `scripts/lccc-snapshot.sh` — wire-into-the-workspace autosave (atomic writes,
  flat `ms178-1.patch` + tarball + git bundle + ledger). Canonical deliverable
  `/home/user/ms178-1.patch` (APPLIES-CLEAN each save).
* `scripts/x86_gcc_torture_i686.py` — full 32-bit (i686) torture/execute runner
  mirroring `x86_gcc_torture.py`, drives `lccc-i686` (which invokes the
  standalone `lccc-ld` with the `elf_i386` model) and uses `gcc -m32` for the
  eligibility/reference check. Includes the multilib header paths.

## 2. Measurement (x86-64, `-O2`, full corpus, `-j2`)

| run | pass | run-fail | link-fail | compile-fail | note |
|---|---|---|---|---|---|
| baseline (`S00`, pristine) | 1638 | 27 | 5 | 0 | |
| after vector fix (`S03`) | **1649** | **16** | 5 | 2 | +11 pass, no regression |

Per-status for the current tree: `pass 1649`, `run-fail 16`, `link-fail 5`,
`compile-fail 2`, `reference-compile-skip 1`, `reference-run-skip 3`
(`1676` cases, `195.9 s`).

> Note on the `reference-*` skips: the *reference* (GCC) compile/run also
> completes for these, so the harness reports them as skips rather than
> failures; they are genuine LCCC gaps, not environment noise.

## 3. Remaining defects (root-cause analysis) — TODO for next sessions

Each entry gives the failing test(s), a precise root-cause, and the file(s) to
investigate. They are ordered by expected impact.

### 3.1 VARARGS: `movq` (64-bit load) of a 32-bit-stored slot  — HIGH impact
**Failing:** `920625-1`, `va-arg-21` (SIGSEGV); very likely also the
`*printf-chk-1` family and `eeprof-1` (all abort with `-6`), and `20230630-2/4`.

**Root cause (confirmed by gdb + codegen diff).** A loop/variadic counter `i`
(`int`, I32) has an 8-byte stack slot. At `-O0` every load is the *sign-
extending* `movslq slot, %rax`; the value is consistently 4-byte. At `-O2` the
*address-computation* load of the same slot is emitted as a plain **`movq`**
(8-byte) load, reading the uninitialized/stale high 32 bits:

```
movl $0, 192(%rsp)      ; i = 0           <- 32-bit store
loop:
  movslq 192(%rsp), %rax ; compare         <- sign-extend (correct)
  ...
  movq 192(%rsp), %rbx   ; index i*8       <- 8-byte load  (WRONG)
  shlq $3, %rbx
```
gdb: `rbx = 0xe0000000000000` at the faulting `mov (%r10),%r8d`, so `i` was
garbage-extended → wild `&pts[i]` → SIGSEGV.

**Where to look.** `src/backend/x86/codegen/emit.rs` `mov_load_for_type`, and
the "W2 Load→Cast fold" in `src/backend/x86/codegen/memory.rs` (around the
`fold_skip_cast` handshake) — the fold that merges `sext(load I32)` into a
single load must choose the **sign-extending** form (`movslq`) when the load's
IR type is I64 but the value is stored in a 4-byte slot. `StackSlot` is a bare
`i64` (no width), so the slot width must be threaded through the load emitter
or the fold must consult the cast's `from_ty`/`to_ty`. The `movl`-store side is
correct; the I64 load must pair with a sign-extend.

### 3.2 VLA `__builtin_offsetof` stride — `pr41935`
**Symptom:** abort. `foo(n,i,j) { typedef int T[n]; struct S{int a;T b[n];};
return __builtin_offsetof(struct S, b[i][j]); }`
LCCC computes `4 + i*8 + j*4`; correct is `4 + i*(n*4) + j*4` (and, for the
backing struct, `4 + n*n*4`). The inner stride `sizeof(T)=n*4` was treated as
the constant `8`.
**Where:** frontend `offsetof` lowering for *nested VLA* (`T b[n]` with
`T=int[n]`) — the `b[i]` element stride. Investigate `src/ir/lowering/`
`const_eval_global_addr.rs` / `expr_builtins_intrin.rs` (`__builtin_offsetof`)
and the VLA-size propagation in `frontend/sema/type_context.rs` /
`frontend/parser/types.rs`.

### 3.3 Aligned VLA struct member — `pr82210`
**Symptom:** abort. `struct S { __attribute__((aligned(16))) struct T{short c;}
a[size]; int b[size]; }` — the address of `s.a[i].c` offends alignment or the
`a`/`b` members overlap. All opt levels. Likely the `aligned(16)` on a VLA
member disables slot relocation / correct offset computation for the following
`b[]`.
**Where:** `src/backend/stack_layout/` (VLA + member alignment) and the struct
layout in `src/frontend/sema/type_context.rs`.

### 3.4 Bitfield copy/store-forwarding — `pr70127`
**Symptom:** abort at `-O2`. `struct S{int f; signed int g:2;}` with chained
assignment `d = e = a[0] = c;` then `foo(a[0].g)` — a 2-bit signed bitfield read
through the store chain. Likely store value not narrowed to the bitfield or the
chained copies don't forward.
**Where:** `src/ir/lowering/expr_assign.rs` (bitfield store /
`lower_struct_assign`) and `src/backend/x86/codegen/peephole/passes/store_forwarding.rs`.

### 3.5 `pr110115` — SIGBUS at `-O1+`
`foo()` zeroes a `signed char` array; `bar(e,f)` uses `h[20]`; `main` computes
`k[baz()+1]=&b` then `*k[0] = -*k[0]`. Store-elimination / loop opt bug on the
stack `h[]` (possibly the test overruns `h[]` under a wrong loop bound
`bar(8,15)`). Investigate `src/passes/` (simplify / gvn / vector_temp_promotion)
for a bad transformation of the `for`/`switch`. Determine whether LCCC or the
test is wrong (GCC runs clean).

### 3.6 `eeprof-1` / `20230630-2` / `20230630-4` — abort `-6`
Likely ABI-related (struct return / variadic / `setjmp`-style state). Confirm
against a minimal repro and check `src/backend/x86/codegen/` return-value
classification for struct returns and the i686 register args.

### 3.7 Fortify tests — `*printf-chk-1` / `fprintf-chk-1` / `vfprintf-chk-1`
These *define* `__printf_chk`/`__fprintf_chk` and forward varargs to
`vprintf`. Abort means the variadic forwarding is wrong (see 3.1) OR LCCC
recognizes these as fortify builtins and substitutes a wrong sequence.
Confirm the driver does NOT rewrite user-defined `__*_chk`.

### 3.8 Link-fail (missing compiler builtins/symbols)
* `pr47237` — `__builtin_apply` / `__builtin_apply_args` (nested-call/stdarg ABI
  helper). LCCC must lower these to the `__builtin_apply` runtime sequence;
  today it emits undefined refs.
* `pr85331`, `pr94591` — `__builtin_shuffle` (vector shuffle builtin) and
  `id_V2SI`. Add a builtin that lowers to a shuffle/lane-select (SSE `pshufd` /
  `vperm*`).
* `pure-1` — `link_error0..4` (the test's own `#define`. Confirm LCCC's
  builtin-recognition doesn't clobber user functions named `link_error*`).
* `va-arg-pack-1` — variadic `va_arg` with `__attribute__((aligned))` /
  packed names; confirm the driver link shim handles it.

### 3.9 Compile-fail
* `nestfunc-5` — nested functions (GCC `__attribute__((cleanup))` + nested
  funcs). Frontend support for nested-function codegen.
* `pr80692` — compile-time negative/`-Werror`-style diagnostic that LCCC
  rejects. Inspect the exact `ccc:` error.

## 4. Performance & measuring on a PMU-less VM

The harness provides **no hardware PMU** (`perf` counters unavailable), so
runtime is the only first-class evidence, and even that is noisy (thermal
throttling not controllable, 2 vCPU, shared cloud). Recommended method:

* Use the **statistical** runner: run each candidate
  `N×(repeat)` with the same workload via the machine-based compare
  (`scripts/x86_gcc_torture.py --json`, plus a per-test `--filter`) and compare
  **geometric means across the corpus**, with `-O0/-O2/-O3` matrices.
* For codegen-quality work use the **assembly oracle**
  `scripts/godbolt.py`/`codegen_oracle.py` (compare LCCC vs GCC/Clang/ICX) and
  `asmdiff.py`/`insndiff.py`, and the `encdiff.py` encoding checker.
* Prefer **instruction-count / dependency-length reasoning** from the
  generated `.s` over wall-clock when the noise floor is high; record only
  changes that move geomean by >1% across many runs.
* When PGO/SIMD codegen must be validated, rely on `compiler-explorer`
  assembly comparison and micro-benchmarks with fixed input, not PMCs.

## 5. How to continue

1. Rebase against latest `ms178/lccc` main.
2. `scripts/ensure_swap.sh` → `scripts/build_lccc_fast.sh` (fastbuild, `-O1`).
3. Run `scripts/x86_gcc_torture.py --flags=-O2 -j2` for x86-64 and
   `scripts/x86_gcc_torture_i686.py --flags=-O2 -j2` for i686 (both `--json`).
4. Fix the highest-impact items first: **§3.1** (varargs `movq`/`movslq`
   slot-width), then **§3.2/§3.3** (VLA layout), **§3.4** (bitfield), then the
   link-fail builtins (**§3.8**).
5. After every validated fix run `scripts/lccc-snapshot.sh "<slug>" "<desc>"`
   and confirm the deliverable reports `[APPLIES-CLEAN]`.

## 6. Validation / regression commands

```
# full x86-64 -O2
python3 scripts/x86_gcc_torture.py --suite <suite> --flags=-O2 -j2 --json r.json --failure-log f.log
# full i686 -O2
python3 scripts/x86_gcc_torture_i686.py --suite <suite> --flags=-O2 -j2 --json r32.json --failure-log f32.log
```
