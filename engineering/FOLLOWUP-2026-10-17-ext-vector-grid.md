# Follow-up: ext_vector_type ABI grid — first measured run against Clang 23.1.3

**Status: every lccc-class entry fixed (ABI-EXTVEC-VA-1, ABI-EXTVEC-I386-O2-1, ABI-EXTVEC-LD-1, ABI-EXTVEC-LD-I386-1); open: the Clang oracle class (ORACLE-CLANG-2313-1/-2) and ABI-GNUVEC-I386-1 (GNU `vector_size` on i386, latched by its own gate).** Measured on 2026-10-17 by the hosted "Frontend and ABI
contracts" job (ubuntu-24.04, lccc fastbuild, Clang 23.1.3 from
`scripts/ci_install_clang_oracle.sh`, GCC 16 as the linker driver).
`tests/regression/check_ext_vector_abi_grid.sh` ran 848 legs over 53 shapes
× {-m64, -m32} × {-O0, -O2} × four provider/caller pairings; 202 legs failed
(249 individual checks); a second run failed a different subset of 27
x86-64 `va_arg` legs, which is itself a finding (see ABI-EXTVEC-VA-1). The gate had never run with its intended oracle
before: the runner's clang-18 was not Clang 23.1.3, so the oracle check
skipped it.

The failing checks are pinned, one line each, in
`tests/regression/ext_vector_abi/grid_expectations.txt`; the gate is red on
any failure not in that file and on any listed failure that stops happening.
This note explains the classes and how to retire them.

## Class `oracle` — Clang 23.1.3 disagrees with itself (153 checks)

For these (bits, opt, shape, check) tuples the **clang provider → clang
caller** leg fails, so there is no reference convention for lccc to meet:

- **i386 `va_arg` of ext vectors** (`hashva`, -m32, -O0 and -O2, 32 shapes):
  `bx2 bx3 bx4 bx8 bx16 bx32 f32x2 f32x5 f32x8 f32x16 f64x3 f64x4 f64x8 ldx1
  ldx2 ldx3 u8x2 u8x3 u8x4 u8x5 u8x7 u8x32 u16x2 u16x3 u16x12 u16x16 u32x5
  u32x8 u32x16 u64x3 u64x4 u64x8`. Shapes whose storage is exactly 16 bytes,
  or 8 bytes with an integer element, pass; sub-8-byte, padded-to-8 and
  wider-than-16 shapes do not. Clang's i386 variadic caller and its
  `va_arg` lowering use different slot sizes/alignments for these.
- **`long double` vectors on x86-64**: `ldx1 hashva` (-O0, -O2) and
  `ldx3 ident kfour` at -O2 fail clang→clang. `ldx1 hashva` also fails
  `clang:lccc` (Clang's `va_arg` reads the one-lane vector from the XMM
  register save area although its own caller — and lccc's — passes it in
  memory; `lccc:clang` passes).
- **ORACLE-CLANG-2313-2 — `ldx3` at -O2, both targets** (second hosted
  measurement, after the 10-byte lane stride landed): `hash2`/`hashva`
  fail `clang:lccc` and `lccc:clang` at -O2 only, while every -O0 cross
  leg and both lccc ↔ lccc legs pass. lccc's layout is
  optimisation-level invariant (lccc -O0 ↔ lccc -O2 links pass locally in
  all four pairings; the callee reads `x` at `entry+8` and `y` at
  `entry+72` at both levels) and agrees with Clang -O0, so Clang -O0 and
  Clang -O2 must disagree with each other about the lanes of
  `<3 x x86_fp80>`. The gate now proves that without lccc in the picture:
  the `clangO0:clang` leg (Clang -O0 provider, Clang -O2 caller) runs for
  every -O2 shape and is listed as failing `hash2`/`hashva` for `ldx3` on
  x86-64 and `hash2` on i386. (Likely LLVM mechanism: the -O2 pipeline
  scalarizes a `load <3 x x86_fp80>` + `extractelement` through a GEP
  whose index scales by the element's 16-byte *alloc* size, while the
  vector load/store legalization packs lanes at the 10-byte *store*
  size; `<2 x x86_fp80>` does not take that path, which is why `ldx2`
  -O2 passes.) If the self-consistency leg ever passes, the rows are
  "expected failure did not occur" and the lccc side must be re-triaged.

These are Clang defects (or, for `va_arg` of a vector, an area the psABI
leaves to the implementation). They are listed so the gate does not pretend
they are lccc bugs, and so a future oracle bump that fixes them shows up as
"expected failure did not occur" and gets re-measured deliberately. The
pinned leg is in `tests/regression/lib/clang_oracle.sh`.

## Class `lccc` / `flaky` — lccc defects (56 deterministic + 72 flaky checks at first measurement; VA-1 since fixed)

Clang agrees with itself on these; lccc does not meet it.

### ABI-EXTVEC-VA-1 — x86-64 MEMORY-class vector arguments wider than 16 bytes (72 flaky checks) — **FIXED**

**Nondeterministic.** Two hosted runs of the same binaries failed different
subsets of the same legs: `hashva` on -m64 for `u8x32 u16x12 u16x16 u32x5
u32x8 u32x16 u64x3 u64x4 u64x8 f32x5 f32x8 f32x16 f64x3 f64x4 f64x8` (and
`ldx1 ldx2 ldx3`), at -O0 and -O2, on the legs `clang:lccc` and
`lccc:lccc`, never on `lccc:clang`.

**Correction to the first write-up.** The grid's leg name is
`<provider>:<caller>` (`check_ext_vector_abi_grid.sh` links
`p-$prov.o c-$call.o`), so `clang:lccc` is a Clang callee under an **lccc
caller**, and `lccc:clang` — the leg that never failed — is lccc's `va_arg`
under a Clang caller. The first write-up read the pair the other way round
and sent the reader to the callee; the defect was in the **caller**.

**Root cause.** lccc modelled a vector type's alignment as the C11
`_Alignof` figure (16 bytes: GCC's `min_align_of_type` caps non-user-aligned
types at `BIGGEST_ALIGNMENT`) and used that one number everywhere. GCC and
Clang give the type `TYPE_ALIGN == size` ("always naturally align vectors",
`layout_type`), uncapped on x86, and only the C11 operator reports the
capped figure. A MEMORY-class argument of such a type sits at a multiple of
its **full** alignment ("the ABI for passing parameters with 32-byte
alignment has changed in GCC 4.6" — the caller realigns `%rsp` so the
outgoing area satisfies it). Every callee — Clang, GCC, lccc's own prologue
and `va_arg` — rounds the slot address up to 32 (or 64); lccc's caller wrote
the vector at a 16-aligned slot, so on the stack residue 16 mod 32 the
callee read 16 bytes that were never written: stack garbage, hence the
run-to-run variation. The same model error made `struct { int z; v32 v; }`
48 bytes with `v` at 16 (GCC: 64 bytes, `v` at 32), gave 32/64-byte vector
objects 16-byte alignment, and made `_Alignof(T32[2])` and `_Alignof(x)`
(object, not type) 16 instead of 32.

**Fix.** `src/common/types.rs` now distinguishes the GCC/Clang alignment of
a vector type (its size) from the C11 `_Alignof(type)` figure (capped at
16, 32 under `-mavx`, 64 under `-mavx512f`); struct layout, object
alignment, `__alignof__`, `_Alignof(expr)` and `_Alignof(array)` use the
former; the x86-64 caller's `struct_arg_aligns` rounds the slot of a
MEMORY-class vector argument to the full alignment and realigns the outgoing
area. The same change carries `aligned(N)` on a typedef as the type's exact
alignment (it may lower it, GCC semantics) and makes `_Alignof(var)` of a
typedef-aligned variable report the typedef's figure.

**Verification** (all against GCC 16; no Clang oracle in the fixing
environment, so the Clang-side grid rows below are retired on the strength
of the identical Clang convention — `byval ... align 32` — and the
measured `lccc:clang`-never-fails pattern):

- `tests/regression/check_gnu_vector_align_abi.sh` (CI gate): 39 layout and
  alignof constants compared verbatim across `-m64`, `-mavx`, `-mavx2`,
  `-mavx512f`, `-m32`, `-m32 -mavx`; x86-64 runtime cross matrix (va_arg,
  named-after-six-GP, first-stack-argument; 32- and 64-byte vectors; -O0
  and -O2; four pairings; both stack residues) — 46/46; the pre-fix compiler
  fails 18 of them. Hosted note: the gate's first hosted run failed on the
  runner GCC's default `-fcf-protection` `.note.gnu.property` (extra `.long`
  lines, not ABI data); the extraction is now scoped to the probe's `r`
  initialiser (DECISIONS 2026-10-18 addendum).
- Corpus tests `tests/regression/gnu_vector_overalign_layout.c` and
  `tests/regression/typedef_aligned_lowers_alignment.c` (run_regression.py,
  GCC reference output).
- The 60 non-`long double` `flaky` rows are retired from
  `grid_expectations.txt`. The 12 `ldx*` `hashva` rows were re-attributed
  to ABI-EXTVEC-LD-1 (`ldx1` is 16 bytes and cannot have been this defect)
  and are retired with it below.

**Residual, tracked separately: ABI-GNUVEC-I386-1** — on i386 GCC passes
16-byte GNU vectors in `%xmm0-2` for fixed arguments and keeps 32/64-byte
vectors in memory at their full alignment with a realigned `%esp` (DRAP via
`%ecx`); lccc's i686 call sequence packs them 4-byte granular. lccc agrees
with itself; both cross legs diverge, and `check_gnu_vector_align_abi.sh`
latches exactly that verdict so it is re-measured on every run.

### ABI-EXTVEC-LD-1 — `long double` ext vectors by value, x86-64 (29 deterministic + 12 flaky) — **FIXED**

**What failed.** `ldx1` return (`ident`, `kfour`) failed **lccc → lccc**;
`ldx2` return was a hard compile error ("by value is not supported");
`ldx2`/`ldx3` as arguments (`hash2`) disagreed with Clang both ways; every
`ldx*` `hashva` leg was flaky.

**The conventions, from Clang's source** (`clang/lib/CodeGen/Targets/X86.cpp`,
`X86_64ABIInfo`). The earlier wording in this note — "memory via hidden
pointer for more [than one lane]" — was wrong:

| shape | argument | return |
|---|---|---|
| `ldx1` (16 B) | `classify` → X87+X87UP → **MEMORY**, 16-aligned | `%st(0)` — GCC's `struct { long double }` |
| `ldx2` (32 B) | 256 bits without AVX → MEMORY → `byval align 32` | **`%st(0)/%st(1)`** — GCC's `_Complex long double` |
| `ldx3` (64 B) | MEMORY, `byval align 64` | hidden pointer (sret), `%rax` = pointer |

The return column is not what `classify` says (MEMORY for two and three
lanes): a vector is not an aggregate for Clang's ABI code, so
`getIndirectReturnResult` hands a MEMORY-class vector return to LLVM
**direct** (`ABIArgInfo::getDirect`), and LLVM scalarizes `<N x x86_fp80>`
onto the x87 return registers — two of them exist, so `ldx2` comes back as
the pair and `ldx3` is sret-demoted, which is the hidden pointer lccc
already used. (`-mavx` changes the `ldx2` *argument* to the same
scalarization, two 16-aligned `f80` slots; lccc has no AVX ABI level, see
AUD-VABI-2.)

**Root causes.**

1. `ldx1`: `get_function_return_type` (func_lowering.rs) collapsed every
   x86-64 vector return to `I64` in the function **definition**, after the
   signature/call path had already picked `F128` for the X87 convention.
   lccc's own caller popped `%st(0)`; the callee had converted the lane to a
   double in `%rax` — a self-inconsistent ABI, hence lccc → lccc red.
2. `ldx2`: there was no two-`f80` return carrier on x86-64 at all; the
   type-level `x86_64_ext_vector_unsupported` list rejected it.
3. `ldx2`/`ldx3` arguments were MEMORY at 8/16-byte slot granularity; the
   32/64-byte slot rule arrived with ABI-EXTVEC-VA-1 and these shapes are
   now on it (same caller/callee code path as `u64x4`/`f64x8`).
4. **Lane stride** (found by the first hosted re-measurement, which still
   failed every multi-lane check in both directions while `ldx1` passed):
   lccc laid `long double` lanes 16 bytes apart. Clang lowers the type to
   LLVM's `<N x x86_fp80>`, whose elements are packed at their 80-bit
   *size* — lane `i` is at byte `10*i` — while the C object keeps the AST
   size `N * 16` rounded up to a power of two. That is exactly why a Clang
   `ld3` global is a 32-byte ELF object (3 × 10, allocated as 32) inside a
   64-byte C object, which `check_ext_vector_layout.sh` had recorded as an
   "oracle limitation". `ExtVectorLayout` now uses `X87_LANE_STRIDE = 10`
   (object size unchanged: 16/32/64; i686 too, 12 × N rounded). Because an
   IR `F128` store writes 16 bytes, a lane store must write the 80-bit image
   only (`store_x87_lane`: scratch + 10-byte copy; `LValueKind::X87VectorLane`
   for `v[i] op= x`); lane loads stay direct (the six bytes read past a lane
   are inside the object). Global initializers write 10-byte lanes.

**Fix.** `SysvPassing::X87Pair` (`sysv_classify::classify_return`, two
`long double` lanes; `arg_classes` → `[Memory]`), carried as
`AggregateReturn::x87_pair` / `FuncSig::ret_x87_pair`; the callee returns
lane 0 as the IR `F128` return and lane 1 through `SetReturnF128Second`,
the caller reads `GetReturnF128Second` and stores both lanes into a
32-byte, 32-aligned temporary (`stmt_return.rs`, `expr_calls.rs`); the
definition-side return type follows the signature for `ret_x87_aggregate ||
ret_x87_pair`. `ldx2` is removed from `x86_64_ext_vector_unsupported`; the
grid's `ret_unsupported` keeps it rejected on **i386 only** (Clang i386
also returns it in `FP0/FP1`; lccc's i686 backend has the pair primitive
but the ext-vector return carrier does not use it yet — tracked as part of
ABI-EXTVEC-LD-I386-1 below).

**Verification** (all on the fastbuild binary, this revision):

- `abi_grid.c` `ldx1`/`ldx2`/`ldx3`, lccc ↔ lccc, {-O0,-O2}², all checks
  including `hashva`: 12/12 legs ok; `hashva` deterministic over 40 runs
  with shifted initial stacks per shape and opt (0 failures), so the
  `flaky` rows are retired.
- GCC 16.2 as the register-convention oracle (Clang 23.1.3 is not
  installable offline; GCC implements the *same* x87 conventions for the
  struct / complex spellings): `check_gnu_vector_align_abi.sh` Part 4 links
  lccc's `ldx1`/`ldx2` functions against GCC's `struct { long double }` and
  `_Complex long double` callers and callees, {-O0,-O2}², 4/4 pass
  (previously: `ldx1` ident/kfour wrong value, `ldx2` did not compile).
  Only returns of the two-lane shape are compared with the complex form:
  Clang passes the argument `byval align 32`, GCC passes a complex at 16.
- `tests/regression/ext_vector_long_double_lanes.c` (lccc-only, exit-bit
  self-check, -O0..-O3), `tests/regression/ext_vector_long_double_layout.c`
  (lane bytes at `10*i` for globals and locals, stores in every lane order,
  lane-to-lane copies, whole-vector arithmetic, -m32 too) and the unit test
  `sysv_classify::long_double_vector_tests` (one/two/three-lane
  argument and return classes).
- Hosted re-measurement: the 35 x86-64 LD-1 rows are removed from
  `grid_expectations.txt`; the Clang legs run on CI and any residual is an
  unlisted failure to triage, not a row to re-add blind.

**Found on the way — ext-vector component access was a silent miscompile.**
`v.x`, `v.zyx`, `v.lo`, `p->y = ...` on any `ext_vector_type` resolved like
a missing struct field: offset 0, type `int`, no diagnostic (`q.y = p.x` on
a `double2` compiled to `movl (%rax), %eax; movl %eax, (%rdi)`). Implemented
in the lowering (`CType::ext_vector_swizzle`, `vector_member_lanes`,
`lower_vector_component_rvalue`, `vector_component_lvalue`,
`try_lower_swizzle_assign`): `xyzw`/`rgba`/`sN…` naming sets, `lo`/`hi`/
`even`/`odd`, through pointers, on rvalues and chained, single components
as full lvalues (packed `_Bool` bits and 10-byte x87 lanes included),
multi-component plain assignment (duplicate components rejected as in
Clang), `sizeof(v.xy)`; sema diagnoses "illegal vector component name" and
"vector component access exceeds type". Not implemented, diagnosed instead
of miscompiled: compound assignment to a multi-component swizzle
(`v.xy += w`). Test: `tests/regression/ext_vector_swizzle.c`.

**ABI-EXTVEC-LD-I386-1 — FIXED** by the lane stride: the second hosted
measurement reports the i386 `ldx3` -O0 `ident`/`kfour` rows as "expected
failure did not occur" (removed); the i386 `ldx3` -O2 `hash2` rows are the
same -O2-only Clang self-inconsistency as on x86-64 and are re-attributed
to ORACLE-CLANG-2313-2. The ten i386 -O2 `lccc:lccc hashva` rows of the
32/64-byte shapes also stopped failing (removed). i386 `ldx2` by-value
return stays rejected (grid `ret_unsupported`; Clang returns it in
`FP0/FP1`, lccc's i686 ext-vector return carrier does not use the pair
primitive) — a diagnosed limitation, not a miscompile.

**Second pass over the swizzle implementation (this revision).**

- `d->xyz = s->zyx` read the source through a zero-initialised temporary
  (6 stores, 3 of them dead): a swizzle on the right now reads its lanes
  straight out of the source vector. Every source lane is loaded before
  the first store, so `v.xy = v.yx` and `rev3(&o, &o)` behave as through a
  temporary (-O2: 3 loads + 3 stores for the three-lane copy, 2 + 2 for
  the swap).
- The assignment's result vector was built after every swizzle store,
  re-reading the destination lanes (an extra **volatile** access per lane
  for a `volatile` vector, and dead work otherwise); it is now built from
  the stored values, and only when the assignment's value is used
  (`discard_assign_result`, set by `lower_expr_discard` for expression
  statements and `for` increments).
- `v.x = 1.0L` on a `long double` lane went through the x87 scratch +
  10-byte copy; a constant lane is now two immediate stores (8 + 2 bytes).
- Sema: "illegal vector component name" vs "exceeds type" are told apart
  by the spelling (`v.xr` mixes naming sets and is illegal, not out of
  range); a vector assigned from a vector of another size (`v.xy = v`,
  GNU `v2 = v4`) is "incompatible types" like GCC/Clang instead of a
  silent wrong-size copy; member access on a scalar base (`x.y` on a
  float, `p->z` on an `int *`, `w.hi.y` on a two-lane vector) is
  "request for member in something not a structure or union" instead of
  offset-0 garbage. Gate: `tests/regression/check_ext_vector_swizzle_diagnostics.sh`
  (14 fixtures, positive control runs).
- Still diagnosed rather than implemented: compound assignment to a
  multi-component swizzle (`v.xy += w`; single components take every
  operator).

**Backend debt observed, not changed here (quantified).** Every IR `F128`
load materialises a truncated `double` shadow in `%rax` (`fstpt slot; fldt
slot; subq $8,%rsp; fstpl (%rsp); popq %rax`, `emit_f128_load_finish`) that
nothing reads when the value stays x87; a non-constant x87 lane store is
`fstpt scratch` + `movq/movw` copy (3 memory operations where `fstpt
10(%rdi)` would do, because the generic `F128` store and the
load→store copy peephole (`generation.rs`, "Exact F128 load->store is a
16-byte copy") write 16 bytes). Both are properties of the x87 value model
shared by scalar `long double`, outside this grid's scope; `long double`
ext vectors are a correctness carrier, not a performance one on x86-64.

### ABI-EXTVEC-I386-O2-1 — i386 -O2 only: 32/64-byte by-value vectors (27) — **FIXED**

`hash2` failed for `u8x32 u16x16 u32x8 u32x16 u64x4 u64x8 f32x8 f32x16 f64x4
f64x8` on -m32 at **-O2 only**, in every leg that contains lccc, including
lccc → lccc.

**Root cause** (not an ABI defect; a generic IR miscompile that the i386
ext-vector carrier happened to expose). `aggregate_copy_forward` deletes a
`memcpy(tmp, obj)` whose temporary is only read, redirecting the reads to
`obj`. Its chain resolution followed a candidate's source only when that
source was *exactly* another candidate root. The i686 call lowering copies
a 32-byte vector argument into a 16-aligned `image` and then the two XMM
pieces from `image` and `image + 16`; after inlining, `piece1 = memcpy(image
+ 16)` is a candidate whose source is a **path into** the candidate
`image`. `image`'s copy was deleted, `piece1`'s reads were redirected to
`image + 16` — uninitialized stack. The callee prologue (reassembling the
vector from its pieces) has the same shape, which is why both halves failed
independently. The same hole hits plain C on x86-64: an inlined function
copying sub-objects out of a by-value struct (`p.hi = o.y`) read zeros.

**Fix.** `resolve_read` unwinds `(root, path)` through candidate sources,
prepending each hop's path, and the rewrite applies it to both Loads and
Memcpy sources (the validation already admitted both shapes).

**Verification.** `abi_grid.c` lccc ↔ lccc, 23 shapes × {-m32, -m64} ×
{-O0, -O2}² = 184 legs, 0 failures (20 before); corpus test
`tests/regression/agg_copy_forward_subobject_chain.c` (x86-64 and i686,
GCC 16 reference; the previous compiler prints `0`/garbage for `p.hi`);
pass unit test `forwards_reads_through_a_path_into_another_forwarded_temporary`.
The 27 rows are retired.

### AUD-VABI-2 — x86-64 AVX ABI level: 32/64-byte vectors in one `%ymm`/`%zmm` — **FIXED**

Measured before the fix (`check_vector_abi_wide.sh`, then a latch): under
`-mavx2` an lccc caller fed a gcc callee garbage (`add: 0 0 0 …`) and a gcc
caller crashed an lccc callee (SIGSEGV); the no-AVX legs agreed. lccc had
no SSEUP notion past 16 bytes: the struct rule stopped there and the
AVX-level classification never existed, so every 32-byte vector took the
memory convention regardless of flags.

**The convention** (psABI 3.2.3; GCC `classify_argument`/`TARGET_AVX`,
Clang `getNativeVectorSizeForAVXABI`): a vector is SSE + one SSEUP per
further eightbyte; with the AVX (AVX-512F) ABI level a 32-byte (64-byte)
vector, or a struct/union whose only member is one, is ONE `%ymm` (`%zmm`)
as a named argument and returns whole in `%ymm0` (`%zmm0`). SSE overflow
is transactional (the ninth vector goes to memory at its 32-byte alignment,
the scalar after it still takes a GPR). An **unnamed** vector of a variadic
call is MEMORY (the register save area has 16 bytes per `%xmm`) and `%al`
counts only the SSE registers actually used. `struct { v4f a, b; }`
(SSE+SSEUP+SSE+SSEUP), `struct { v8f; int }` (40 B) and `struct { v8f;
v16f }` (96 B, 64-aligned stack slot, sret) stay MEMORY.

**Implementation.** `TARGET_VECTOR_ABI_BYTES` (16/32/64, set from the
requested ISA level exactly like `BIGGEST_ALIGNMENT`: `-mavx*`, `-march=`,
`native`, cleared by `-mno-avx`); `sysv_classify` emits the wide shapes
only at that level and `vector_reg_bytes_of` recognises them; the carriers
gained a size (`StructXmmReg { size }`, `F128SseReg { size }`,
`vector_reg_move`: `movdqu`/`vmovdqu`/`vmovdqu64`). A `%ymm0`-returned
aggregate keeps the IR's sret shape — the hidden pointer is argument 0 — but
the psABI gives it no register: the callee classifies it as
`ParamClass::VectorReturnBuffer`, homes it as the address of a private
frame buffer and the return sequence loads that buffer into `%ymm0`
(without the epilogue `vzeroupper`, which would zero the upper lanes); the
caller skips it in the argument walk and stores `%ymm0` through its own
buffer after the call (`emit_call_store_vector_return`, restoring
caller-saved homes first — the restore is now idempotent per call site).
`num_fixed_args` now counts the sret slot and `emit_call` demotes unnamed
wide vectors to MEMORY; `va_arg` of a wide vector reads the overflow area
at its alignment. The ext-vector rejection (`x86_64_ext_vector_unsupported`)
admits the shapes the level makes one register (Clang: returns classify as
named), so `f32x8`/`f64x3`/… return by value under `-mavx2`.

**Verification.** `check_vector_abi_wide.sh` (no latch left): `wide.c` +
`wide_avx.c` (23 functions over the rule set above) × 4 legs × -O0/-O2 ×
{`-mavx2`, `-march=x86-64-v3`, `-mavx512f`, none, `-mno-avx`} = 80 binaries,
all equal to gcc/gcc; `-march={native,znver4,skylake-avx512,x86-64-v4}` and
`-mavx512f -mno-avx512f` agree too (manual). New Clang-oracle gate
`check_ext_vector_abi_avx.sh` (hosted CI): 10 32-byte shapes under `-mavx2`
(+4 64-byte under `-mavx512f`), lccc ↔ lccc legs run locally (0 failures).

**Debt observed (quantified) — RETIRED 2026-10-18, see the next entry.**
`v8f add8(v8f a, v8f b) { return a + b; }` at `-O2 -mavx2` was 37
instructions against GCC's 2 (`vaddps %ymm1,%ymm0,%ymm0; ret`): 32-byte
vectors were memory objects in the IR, every over-aligned alloca access
re-derived `(slot+31)&~31` at run time, parameters were captured to slots
and the return value was copied into the hidden buffer before the `%ymm0`
load. Still not covered: a per-function `__attribute__((target("avx2")))`
ABI level (GCC and Clang 23 classify per function; lccc per TU), and a
variadic call through a non-identifier callee expression (`s->fn(1, v)`)
is lowered with every argument named (VA-FPTR-EXPR).

### VEC-EXT-PERF-1 — whole-vector lowering of the GNU vector extension — **FIXED** (`add8` 37 → 2)

**Measurement.** `tests/regression/vector_ext/matrix.c` (336 functions,
generated by `gen_matrix.py`: every lane type × 16/32 bytes × arithmetic,
bitwise, shifts, compares, negate/not, ternary, compound assign, calls,
struct/union wrappers, loops, `-0.0` edge cases; 20 FMA-contraction shapes;
60 register-handoff shapes `ho_{f4,d2,f8,d4}_*`; 19 control-flow /
lifetime shapes `cf_*`) compiled at `-O2 -mavx2` by lccc and GCC 16.2
(`-mfma`), instruction counts per function (`check_vector_ext_codegen.sh`
counting rule: non-label, non-directive lines, `ret` included):

| | main `87713fa` | this series | GCC 16.2 |
|---|---|---|---|
| total instructions, the original 237 comparable functions | 12 525 | 2 533 | 1 446 |
| ratio to GCC (original 237) | 8.66× | 1.75× | 1 |
| total instructions, all 336 functions | — | 3 145 | 1 909 |
| functions worse than GCC / equal-or-better | 235 / 2 (of 237) | 119 / 118 (of 237); 156 / 180 (of 336) | — |
| `andq $-32` per-access realignments | 1 127 | 9 (`viacall` 4, `cf_wrap_phi` 3, `cf_volatile_t` 2) | 0 |
| `add_v32f` / `neg_v32f` / `eq_v32i` | 37 / 23 / 140 | 2 / 2 / 2 | 2 / 3 / 2 |
| `cond_v32f` / `sel_mask` / `sadd` | 59 / 196 / 37 | 9 / 7 / 18 | 6 / 3 / 2 |
| `fma_f8` / `fnma_f8` / `fms_d4` (scalarised on main) | — | 2 / 2 / 2 | 2 / 2 / 2 |
| `ho_f8_al2` (`t=a+b; return b*t+t`) / `mix_sab` | — | 3 / 3 | 3 / 3 |
| `cf_goto_loop` / `cf_loop_acc` (loop bodies) | — | 15 / 12 | 25 / 19 |

Runtime: every function of the matrix is executed by `driver.c` against
the GCC-built twin (612 checks, both link directions, `viacall` calls a
GCC callee from lccc code, `cf_*_call` shapes call back into the driver)
at eleven levels (`-O0 -O1 -O2`, `-O2 -mno-avx`, `-O2 -mavx`, `-O2
-mavx2`, `-O3 -mavx2 -mfma`, `-Os -mavx2`, `-O2 -mavx2 -mno-fma`, `-O2
-mavx2 -mfma -ffp-contract=off`, `-O2 -mavx512f`), 612/612 at each; lccc
compiles every level under `CCC_VERIFY_IR=1` and any verifier report
fails the gate.

**Found and fixed by the extended matrix (all pre-existing on main):**

- *SLP shared-load schedule* (`slp_vectorizer.rs`, load CSE in
  `build_pack`): a load pack shared by two consumer packs was scheduled at
  the LATER scalar group's position, after a consumer scheduled at the
  earlier group — IR use-before-def (`[ir-verify] after slp … does not
  dominate its use`), garbage at runtime for `ho_{f8,d4}_al2` at
  `-O2 -mno-avx`. The CSE now adopts the earlier group's `lane_vals` and
  anchor pointer; the no-write window check uses `is_memory_write`
  (covers `VecStore` intrinsics from a previous seed of the same block).
- *Driver* `-D'NAME(args)=body'` defined an object-like macro named
  `NAME(args)` (`define_macro`); function-like command-line macros now
  parse through the same path as `#define`.
- *Loop-carried FMA gate* was coarser than the scalar one: `s = s*a[i]
  + c` was left unfused (GCC fuses it; the differential disagreed by one
  rounding). Now the exact scalar rule: fuse unless the accumulator is the
  addend.

**What changed** (one mechanism per layer, each falls back to the scalar
lowering when its precondition fails):

1. *Lowering* (`src/ir/lowering/vector_simd.rs`): a vector binary op,
   compare, negate/not, compound assignment, `return`, `?:`, plain
   assignment and initializer whose shape has an exact packed family on
   the code-generation ISA is emitted as `VecLoad<fam>` → one packed
   intrinsic → `VecStore<fam>` into the result temporary, instead of
   per-lane scalar IR. The compare stores its mask through the MASK type's
   family so the next consumer's load pairs with it.
2. *`vec_ssa`* (`src/passes/vec_ssa.rs`, phase 11a'): promotes every alloca
   whose only accesses are whole-width same-family `VecLoad`/`VecStore`
   to SSA (store-to-load forwarding, dead store removal), and hoists a
   parameter's single `VecLoad` to the entry prefix so the prologue can
   move the incoming `%ymmN` straight into its home (`vec_param_preloaded`;
   dead-param and prologue decisions share one predicate through
   `CallAbiConfig::sysv_x86_64()` + `classify_params`).
3. *Return*: `try_lower_vector_return_simd` returns the packed value as the
   `Return` operand; `returns.rs` loads it into `%ymm0`/`%xmm0` from its
   home (no hidden-buffer round trip), marks `LCCC_RET_XMM` bit 0, and
   does not `vzeroupper` after a `%ymm0` return value.
4. *Alignment*: `downgrade_nonescaping_vector_align` now understands the
   `VecLoad*`/`VecStore*` families (unaligned moves, exact width) and the
   backend no longer re-raises a parameter alloca's alignment from
   `IrParam` after the pass proved it unobservable (the source of 1 100 of
   the 1 127 `andq $-32`s).
5. *Backend*: regalloc admits a register-valued vector `Return` and the
   width-group legal-consumer check; `vector_copy` peephole retargets a
   VEX producer whose result is only copied to the return register
   (`vmovdqa %ymm4,%ymm0` and the legacy full-width `movdqa %xmm4,%xmm0`,
   wide-reader rule kept), classifying `vp*` integer and `vcmp*` forms
   exactly.
6. *Two pre-existing miscompiles found by the matrix*: `return -a;` of a
   float-lane vector negated the alloca POINTER (SIGSEGV; `expr_ops.rs`
   `Neg` arm), and the x86 `Select` over an over-aligned alloca arm
   clobbered the flags between `test` and `cmov` (`tern_assign` at
   `-mno-avx`; `comparison.rs` saves/restores flags around the realign).

**Gate.** `tests/regression/check_vector_ext_codegen.sh` (`ci_local.sh`
gate `vector-ext-codegen`, hosted step "GNU vector-extension whole-vector
lowering"): the runtime matrix at every level the host CPU can execute,
then per-function ceilings at `-O2 -mavx2` — arithmetic ≤ 2, negate ≤ 3,
compares ≤ 2, `ident` = 1, the ternary / select / loop shapes at their
measured counts, `add_v32f` must spell `vaddps %ymm1, %ymm0, %ymm0`, and
no `andq $-32` in any register-shaped function. The scalar-residue
functions below are ratcheted at their counts so they can only go down.

**Residue, quantified (per-function counts lccc / GCC at `-O2 -mavx2`),
with the fix for each:**

- *DEBT VEX-128-INT-1* — `eq_v16i` 4 / 2, `mix_sab4` 7 / 4: the 128-bit
  integer compare and arithmetic emitters (`emit_int_cmp` with
  `avx2=false`, `emit_sse_binary_128`) use the legacy destructive SSE forms
  plus staging moves even when VEX is available (and mixing legacy SSE
  with VEX-256 state is a transition-penalty hazard). Fix: route every
  128-bit integer emitter through the three-operand VEX spelling when
  `avx_enabled`, exactly as the float twins already do.
- *Vector-by-vector shifts* — `shl_v32i` 41 / 2, `shl_v16i` 25 / 2,
  `shl_v32l` 23 / 2: only `vector << constant` is packed. Fix: new
  `VecShlvI32x{4,8}`/`VecSrlv`/`VecSrav` intrinsics (`vpsllvd`/`vpsrlvd`/
  `vpsravd`, `vpsllvq`/`vpsrlvq`; AVX2 only, no `vpsravq` before
  AVX-512) in `packed_shift`, `vec_shift_plan` and the x86 emitter.
- *64-bit lane compares* — `eq_v32l`/`lt_v32l` 41 / 2, `eq_v16l`/`lt_v16l`
  25 / 2: `packed_cmp_blendv` has no I64 family. Fix: `VecCmpI64x{2,4}`
  over `vpcmpeqq`/`vpcmpgtq` (SSE4.1/4.2 — gate on AVX2 like the others),
  `IntCmpLane::Qword` in `emit_int_cmp` with the 8-byte sign bias for the
  unsigned predicates.
- *Byte multiply* — `mul_v32c` 136 / 10, `mul_v16c` 72 / 12, `loop_v32c`
  162 / 25: x86 has no `pmullb`. Fix: GCC's expansion (`vpmovzxbw` halves,
  `vpmullw`, `vpand` with 0x00ff, `vpackuswb`) as one `VecMulI8x32`
  emitter.
- *Packed FMA residue* — `fma_chain` 4 / 3 (`(a*b+c)*d+e`: the second
  FMA's result is homed in a chain temporary and copied to `%ymm0`; the
  retarget cannot move an FMA destination — fix: let the destructive
  handoff prefer the %ymm0-resident parameter for the LAST FMA of a
  returned chain, or teach `reassociate_fma_accumulator` the packed
  forms), `fma_two` 6 / 4 (two FMAs sharing `m = a*b`: GCC re-encodes
  231/132 to keep `a`/`b` in place; lccc stages two `vmovaps` copies —
  same fix, packed re-encoding in the copy peephole).
- *Control-flow / lifetime residue* (`cf_*`, lccc / GCC): `cf_wrap_phi`
  36 / 6 and `cf_volatile_t` 21 / 8 are the struct-wrapped and
  `volatile` memory objects (both legitimately in memory; the wrapper
  still copies through the 8 `movq` capture — see struct-wrapped below);
  `cf_all_lanes` 31 / 2 (per-lane `t[i] = t[i] + i` loop: GCC folds it to
  one `vaddps` with a constant vector — needs lane-store forwarding into
  a constant pool vector); `cf_d_mix` 29 / 5 (GCC inlines the `static`
  helper and fuses; lccc calls it — inliner heuristic for small static
  vector helpers); `cf_sw` 24 / 16 and `cf_phi_call` 20 / 16 (phi merges
  of 32-byte values still go through a slot when a call or switch
  intervenes — `vec_ssa` promotes allocas, the backend phi web has no
  %ymm class yet); `cf_cond_assign` 9 / 5 (same, one arm). The loop
  shapes `cf_goto_loop` 15 / 25 and `cf_loop_acc` 12 / 19 are SMALLER
  than GCC's (GCC peels and unrolls ×2).
- *Struct-wrapped vectors* — `sadd` 18 / 2, `s4i_add` 26 / 11: the
  compound assign on a struct member is packed, but the parameter capture
  and `return a;` of the WRAPPER still copy through memory (the wrapper's
  class is `Sse+SseUp` too). Fix: let `try_lower_vector_return_simd` and
  the param hoist see through a single-member struct whose member is the
  whole vector.
- *Ternary over call arms* — `tern_se` 53 / 11, `viacall` 30 / 15: two
  sret temporaries plus the merge copy; the four surviving `andq $-32` are
  the by-value stack argument slot in `viacall`. Fix: give the conditional
  a single result home when both arms are `%ymm0` returns (merge in
  register), and let the call's vector-return buffer BE the result alloca.
- *VEX-128 FMA staging* — `ho_f4_*`/`ho_d2_*` at 128 bits carry one
  legacy `movaps`/`movdqa` staging copy where the 256-bit twins carry none
  (same root as DEBT VEX-128-INT-1: the 128-bit emitters stage through a
  legacy copy); integer `VecZero` seeds (`v32i s = {}`) are not
  register-homed (`cf_uninit` is float and is 3 / 3).
- `loop_v32f` 16 / 14: one `movslq` outside the loop (the induction
  variable is now widened to 64 bits because the packed body indexes with
  a 64-bit GEP); the loop body is 7 instructions either way.
- Not accepted by lccc at all (frontend, unchanged here): designated
  initializers on vectors, general `__builtin_shufflevector`, multi-lane
  swizzle compound assignment, `-mprefer-vector-width=`.

### PEEP-MULSS-RAX-1 — legacy-SSE `mul*`/`div*` classified as an implicit `%rax` write — **FIXED**

Found by the `-mno-avx` leg of the gate above (lccc main + any callee
crashed). `parse_dest_reg_fast` prefix-matched `mul`/`div`, so the CFG
liveness treated `mulss %xmm2,%xmm3` as a KILL of `%rax`;
`eliminate_dead_flag_writes` then deleted the `addq $63,%rax; andq
$-64,%rax` realignment of a 64-byte vector alloca whose address was
dereferenced after the multiply (`movss %xmm3,(%r10,%rax)` wrote 48 bytes
early; `fill_mul` in the new corpus test printed 1326 for 2040). Only the
VEX-less path spells the multiply `mulss`, so the default v3 baseline hid
it. Fix: exact mnemonic match (`mul{,b,w,l,q}`, `{i,}div{,b,w,l,q}`); unit
test `sse_scalar_arith_is_not_an_implicit_rax_write`, corpus test
`sse_scalar_mul_not_rax_kill.c` (`-O2 -mno-avx`, GCC reference).

## Retiring an entry

Fix the defect, run the grid locally (`scripts/ci_local.sh` gate
`ext-vector-abi-grid`, which needs the pinned oracle:
`scripts/ci_install_clang_oracle.sh`), and delete the lines the gate reports
as "expected failure(s) did not occur" in the same commit. Never add a line
for a new failure without a tracking id and a reproduction here.
