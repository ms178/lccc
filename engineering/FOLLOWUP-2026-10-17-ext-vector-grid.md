# Follow-up: ext_vector_type ABI grid — first measured run against Clang 23.1.3

**Status: open (ABI-EXTVEC-VA-1, ABI-EXTVEC-I386-O2-1 and ABI-EXTVEC-LD-1 fixed; the Clang oracle class, ABI-GNUVEC-I386-1 and ABI-EXTVEC-LD-I386-1 open).** Measured on 2026-10-17 by the hosted "Frontend and ABI
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
  `ldx3 ident kfour` at -O2 fail clang→clang.

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
  fails 18 of them.
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

**Residual: ABI-EXTVEC-LD-I386-1** (6 rows, i386 `ldx3` only): `ident`/
`kfour` cross legs at -O0 and `hash2` cross legs at -O2 disagree with Clang
(lccc agrees with itself; the -O2 `ident`/`kfour` legs are Clang-broken,
class `oracle`). On i386 Linux every vector argument and return is
`getDirect()` for Clang, so `<3 x x86_fp80>` is what LLVM's i386
legalization makes of it (scalarized `f80` slots and an sret-demoted
return), not the 64-byte image with a 4-byte-aligned slot lccc uses after
the hidden pointer. Not measurable here without Clang; it needs the
hosted run's assembly to pin the slot layout before changing the i686
carrier.

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

## Retiring an entry

Fix the defect, run the grid locally (`scripts/ci_local.sh` gate
`ext-vector-abi-grid`, which needs the pinned oracle:
`scripts/ci_install_clang_oracle.sh`), and delete the lines the gate reports
as "expected failure(s) did not occur" in the same commit. Never add a line
for a new failure without a tracking id and a reproduction here.
