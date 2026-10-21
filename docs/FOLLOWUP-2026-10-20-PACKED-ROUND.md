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

## Oracle and performance evidence — see companion benchmark record

For fully pinned measurements, see [`BENCH-2026-10-20-PACKED-ROUND.md`](./BENCH-2026-10-20-PACKED-ROUND.md) — native runtime (60 interleaved runs), Callgrind (3× deterministic), and the Compiler Explorer metric-trap audit. Summary: packed removes **80.95%** of LCCC’s scalar `Ir` (**5.25×**), **6.26×** after the signed-guard fix (`11,020,010` vs `69,030,386`), at parity with ICX `10,804,117`; median native `3.905 ms` vs GCC `10.399 ms` on the `BENCH_MAIN` fixture.

## Red-team follow-up after rebase

- **Metric-trap finding**: `floor_f64` static sizes (GCC `10`, LCCC/Clang `47`, ICX `24`) rank scalar “best” despite 4× less work per hot iteration — see bench doc for why runtime/Callgrind are authoritative.
- **Signed-guard fix**: Byte-IV fusion now handles dynamic signed `i64` bounds (`0…i64::MAX/elem_size` → byte IV, else `0` packed trips → scalar mirror), removing the redundant `addq $1` element IV from every hot iteration (−16.18% Ir, −1 uop per vector trip).
- **Metadata choice rejected**: Unary `round`/`sqrt` remain **SSA producers** only — not two-operand binary consumers. Position-zero already covers their source; widening position-one would grant an unsound contract to immediates/indices.

Machine-level confirmation (details in companion doc): **GDB 16.3** shows `vroundpd mem,%ymm0` → `vmovupd`, **objtool** passes (only expected `%rbp`/`omit-frame-pointer` warnings), **pahole 1.30** correctly N/A for this C object.
