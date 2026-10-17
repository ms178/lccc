# Follow-up: ext_vector_type ABI grid — first measured run against Clang 23.1.3

**Status: open (ABI-EXTVEC-VA-1 and ABI-EXTVEC-I386-O2-1 fixed; LD-1 and the Clang oracle class open).** Measured on 2026-10-17 by the hosted "Frontend and ABI
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
  `grid_expectations.txt`. The 12 `ldx*` `hashva` rows are re-attributed to
  ABI-EXTVEC-LD-1 (class unchanged): `ldx1` is 16 bytes and cannot have been
  this defect, and the `ldx` shapes fail every other by-value check under
  LD-1.

**Residual, tracked separately: ABI-GNUVEC-I386-1** — on i386 GCC passes
16-byte GNU vectors in `%xmm0-2` for fixed arguments and keeps 32/64-byte
vectors in memory at their full alignment with a realigned `%esp` (DRAP via
`%ecx`); lccc's i686 call sequence packs them 4-byte granular. lccc agrees
with itself; both cross legs diverge, and `check_gnu_vector_align_abi.sh`
latches exactly that verdict so it is re-measured on every run.

### ABI-EXTVEC-LD-1 — `long double` ext vectors by value, x86-64 (29 deterministic)

`ldx2`/`ldx3` as arguments (`hash2`) disagree with Clang in both directions
at -O0 and -O2; `ldx1` return (`ident`, `kfour`) fails **lccc → lccc** —
lccc's own caller and callee disagree on a one-lane `long double` vector
return. Clang passes/returns these as consecutive x87 `long double`s
(memory for arguments; `%st(0)` for a one-lane return, memory via hidden
pointer for more). Reproduce with `-DELEM="long double" -DLANES=1`, any
leg containing lccc.

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
