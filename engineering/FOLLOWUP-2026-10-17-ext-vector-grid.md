# Follow-up: ext_vector_type ABI grid — first measured run against Clang 23.1.3

**Status: open.** Measured on 2026-10-17 by the hosted "Frontend and ABI
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

## Class `lccc` / `flaky` — open lccc defects (56 deterministic + 72 flaky checks)

Clang agrees with itself on these; lccc does not meet it.

### ABI-EXTVEC-VA-1 — x86-64 `va_arg` of ext vectors wider than 16 bytes (72 flaky checks)

**Nondeterministic.** Two hosted runs of the same binaries failed different
subsets of the same legs: `hashva` on -m64 for `u8x32 u16x12 u16x16 u32x5
u32x8 u32x16 u64x3 u64x4 u64x8 f32x5 f32x8 f32x16 f64x3 f64x4 f64x8` (and
`ldx1 ldx2 ldx3`), at -O0 and -O2, whenever **lccc compiled the callee**
(clang→lccc and lccc→lccc). The value read varies between runs, so lccc's
`va_arg` reads a slot the caller never wrote — stack garbage — rather than a
consistently wrong slot. The direction matters:

- lccc caller → Clang `va_arg` (lccc:clang) never fails `hashva` for these
  shapes: lccc's variadic **caller** places the vector where the SysV ABI
  says (class MEMORY, natural alignment, in the overflow area).
- lccc `va_arg` fails against both callers: the **callee** half is wrong.

These rows are class `flaky` in `grid_expectations.txt`: the gate counts
them but cannot judge them. The i386 rows for the same shapes are class
`oracle` (Clang disagrees with itself there), so there is no i386 reference.

Reproduce (x86-64; run several times, the verdict varies):

    lccc     -m64 -O0 -DPROVIDER -DELEM=unsigned -DLANES=8 -c tests/regression/ext_vector_abi/abi_grid.c -o p.o   # lccc callee (the failing half)
    clang-23 -m64 -O0            -DELEM=unsigned -DLANES=8 -c tests/regression/ext_vector_abi/abi_grid.c -o c.o
    gcc -m64 -no-pie p.o c.o -o t && for i in 1 2 3 4 5; do ./t; done   # "FAIL hashva" on some runs

Where to look: the `va_arg` lowering for `CType::ExtVector` (and `Vector`)
of class MEMORY. The SysV rule: round `overflow_arg_area` up to the type's
alignment when it exceeds 8 (16 for 32-byte vectors; 32 is NOT required by
the psABI, which caps at 16 — Clang follows the cap), read the object there,
then advance by the size rounded up to 8. A read from the register save
area, or an advance by the unpadded lane size instead of the storage size,
both produce exactly this stack-dependent garbage. Compare with the GNU
`__attribute__((vector_size(32)))` path, which the SysV gate covers.

### ABI-EXTVEC-LD-1 — `long double` ext vectors by value, x86-64 (29 deterministic)

`ldx2`/`ldx3` as arguments (`hash2`) disagree with Clang in both directions
at -O0 and -O2; `ldx1` return (`ident`, `kfour`) fails **lccc → lccc** —
lccc's own caller and callee disagree on a one-lane `long double` vector
return. Clang passes/returns these as consecutive x87 `long double`s
(memory for arguments; `%st(0)` for a one-lane return, memory via hidden
pointer for more). Reproduce with `-DELEM="long double" -DLANES=1`, any
leg containing lccc.

### ABI-EXTVEC-I386-O2-1 — i386 -O2 only: 32/64-byte by-value vectors (27)

`hash2` fails for `u8x32 u16x16 u32x8 u32x16 u64x4 u64x8 f32x8 f32x16 f64x4
f64x8` on -m32 at **-O2 only** (the same legs pass at -O0), in every leg
that contains lccc, including lccc → lccc. The classification is therefore
right and an optimisation is wrong: suspect the i686 call-argument staging
or vector copy elimination for by-value arguments larger than one XMM
image. Reproduce with `-m32 -O2 -DELEM="unsigned long long" -DLANES=4`;
bisect with `CCC_NO_*` knobs on the lccc provider leg first (callee side)
and then the caller leg.

## Retiring an entry

Fix the defect, run the grid locally (`scripts/ci_local.sh` gate
`ext-vector-abi-grid`, which needs the pinned oracle:
`scripts/ci_install_clang_oracle.sh`), and delete the lines the gate reports
as "expected failure(s) did not occur" in the same commit. Never add a line
for a new failure without a tracking id and a reproduction here.
