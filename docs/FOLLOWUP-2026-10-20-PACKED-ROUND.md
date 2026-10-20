# Packed round/sqrt map follow-up (2026-10-20)

## Defect and implementation

Elementwise `floor`, `ceil`, `trunc`, `rint`, `nearbyint`, and `roundeven`
loops reached x86 code generation as scalar `RoundScalarF32/F64` operations.
The IR and both x86 assemblers already supported packed ROUNDPS/ROUNDPD, but
the map expression grammar had no rounding node. Consequently the hot glibc
round-family workload remained scalar.

The map vectorizer now preserves the scalar immediate as
`MapExpr::Round(mode, child)`, preflights SSE4.1 legality before mutation,
emits a register-valued `VecRound` family at 128 or 256 bits, and reconstructs
the original scalar intrinsic in every remainder lane. Direct stream loads are
folded into the legal memory source of packed round and square root. The x86
vector-defer model classifies round and sqrt SSA producers; its existing
position-zero cache-aware rule covers their unary consumption. Deliberately
not labelling them as two-operand binary consumers avoids granting an unsound
position-one cache contract to immediate/index operands. This eliminates dead
vector stack homes. A simple packed body is exactly two instructions:

```asm
vroundpd $mode, (base,index), %ymm0
vmovupd %ymm0, (destination,index)
```

The SSE2 target rejects packed round before mutation; `-mno-avx` selects the
128-bit SSE4.1 forms. i686 retains its exact x87 implementation and is covered
for every operation, tail length, signed zero, infinity, NaN payload, and all
four ambient rounding modes.

## Reproduction

```sh
cargo build --profile fastbuild -j2
CCC=target/fastbuild/lccc bash tests/regression/check_vectorize_round_maps.sh
```

The mutation-sensitive gate covers twelve rounding combinations (six modes ×
F32/F64), F32/F64 sqrt, AVX2, SSE4.1-without-AVX, SSE2 refusal, the map kill
switch, integrated assembly, x86-64 runtime tails 0..39, and the complete i686
runtime leg. Independent add→round and add→sqrt kernels cover the register
source arm for both element widths, so a working memory fold cannot conceal a
broken producer/consumer chain. The gate rejects reintroduced vector stack
traffic and missed memory-source folds.

## Oracle and performance evidence

Pinned compilers:

- GCC 16.2.0 (`gcc (foss-mirror) 16.2.0`)
- Clang 23.1.3, official LLVM release
- ICX 2026.2.0.20261001

The benchmark is the fixture's `BENCH_MAIN`: fourteen noinline array kernels,
4096 elements, 200 repetitions. All four executables produced identical
output. Sixty randomized, interleaved native runs after six warmups measured:

| compiler | median ms | minimum ms |
|---|---:|---:|
| LCCC | 3.905 | 3.813 |
| GCC 16.2 | 10.399 | 10.254 |
| Clang 23.1.3 | 8.324 | 8.241 |
| ICX 2026.2 | 3.924 | 3.856 |

Three deterministic Callgrind runs per executable produced identical counts
within each compiler:

| build | Callgrind Ir |
|---|---:|
| LCCC packed | 13,148,000 |
| LCCC `CCC_NO_MAP_VEC=1` | 69,030,386 |
| GCC 16.2 | 93,770,385 |
| Clang 23.1.3 | 57,207,032 |
| ICX 2026.2 | 10,804,110 |

Thus the transform removes 80.95% of LCCC's retired-instruction-model count
versus its scalar negative control (5.25× fewer). ICX executes fewer modeled
instructions; LCCC's compact loop was marginally faster in this native sample.
This is reported as measured rather than as a universal cycle claim.

## Red-team follow-up after rebase

A fresh Compiler Explorer run through `scripts/codegen_oracle.py` found an
important metric trap. For standalone `floor_f64`, GCC 16.2 has only 10 static
instructions because it remains scalar; LCCC and Clang each have 47 after
counting vector setup and their complete remainder paths, while ICX has 24.
Static function size therefore ranks GCC “best” while it performs four times
less useful work per hot iteration. Inspection of the saved assembly confirms
that LCCC and ICX have the same two memory operations in the packed data path;
LCCC's hot body is packed round, packed store, and loop control. This is why the
runtime and Callgrind evidence above—not the static score alone—is authoritative.

Initial revalidation on the rebased tree produced 13,147,955 Callgrind Ir for
packed LCCC and 69,030,355 for `CCC_NO_MAP_VEC=1` (5.25× fewer). Red-team
inspection then found that the existing byte-IV fusion declined dynamic signed
64-bit bounds for multi-byte elements. A preheader range guard now maps negative
or byte-scaling-overflowing limits to zero packed trips and leaves the exact
scalar mirror in charge; safe limits use the byte IV for both addressing and
loop control. This removes the redundant element-IV increment from every hot
vector trip. Final Callgrind fell to 11,020,010 Ir, a further 16.18% reduction
and 6.26× fewer than the scalar control, while ICX 2026.2 measured 10,804,117.
Fifty randomized native LCCC/ICX rounds after warmup measured 4.004 ms versus
4.081 ms median on this VM. Absolute native medians move with VM conditions;
the guarded semantics, assembly reduction, and modeled-instruction reduction
do not.

The audit also rejected one tempting but incorrect metadata choice: unary
round/sqrt are not members of the two-operand-binary consumer family. Their
vector result must be classified as an SSA producer, while consumption of their
source is already safe through the generic position-zero rule. Keeping the
binary classification would broaden the position-one deferral contract to an
immediate or address index without a corresponding backend guarantee.

GDB 16.3 disassembly of the linked benchmark independently confirmed the hot
body at machine addresses: memory-source `vroundpd`, `vmovupd`, then loop
control, with no hidden stack traffic. Linux objtool's stack validator completed
successfully on the generated object; its warnings are the expected kernel-ABI
frame-pointer assumptions (`%rbp` used as a user-space scratch register and an
omit-frame-pointer call), not malformed instruction flow. Pahole 1.30 correctly
reported that this C object has no DWARF/BTF type section; it therefore supplies
no layout evidence for this scalar-array fixture and is not used to manufacture
a performance claim.
