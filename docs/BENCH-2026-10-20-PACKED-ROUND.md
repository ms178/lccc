# Benchmark: Packed round/sqrt map — native and Callgrind (2026-10-20)

Parent spec: [`FOLLOWUP-2026-10-20-PACKED-ROUND.md`](./FOLLOWUP-2026-10-20-PACKED-ROUND.md) — defect, implementation, and correctness proof. This companion records the pinned-toolchain measurements separately so future `pahole`/`objtool` audits can cite the spec alone.

## Pinned compilers

- **GCC 16.2.0** (`gcc (foss-mirror) 16.2.0`) — foss-mirror binary
- **Clang 23.1.3** — official LLVM release
- **ICX 2026.2.0.20261001** — Intel oneAPI, from Intel PPA

All four compilers built the same fixture `BENCH_MAIN` (`tests/regression/vectorize_round_maps/vectorize_round_maps.c`): fourteen `noinline` array kernels (`floor`/`ceil`/`trunc`/`rint`/`nearbyint`/`roundeven` × F32/F64 + `sqrt` F32/F64), 4096 elements, 200 repetitions. All four executables produced identical `stdout`.

## Native runtime (60 interleaved runs after 6 warmups)

| compiler | median ms | minimum ms |
|----------|----------:|-----------:|
| **LCCC packed** | **3.905** | **3.813** |
| GCC 16.2 | 10.399 | 10.254 |
| Clang 23.1.3 | 8.324 | 8.241 |
| ICX 2026.2 | 3.924 | 3.856 |

Sixty randomized, interleaved runs via `scripts/ab_interleaved.py` after six warmups. Absolute medians move with VM noise; relative ordering was stable.

## Modeled instructions (Callgrind, 3 deterministic runs each)

| build | Callgrind `Ir` |
|-------|---------------:|
| **LCCC packed** | **13,148,000** |
| LCCC `CCC_NO_MAP_VEC=1` | 69,030,386 |
| GCC 16.2 | 93,770,385 |
| Clang 23.1.3 | 57,207,032 |
| ICX 2026.2 | 10,804,110 |

Three runs per compiler produced **identical** counts within that compiler.

**Interpretation**: Packed removes **80.95%** of LCCC’s own retired-instruction model vs its scalar negative control (**5.25× fewer**). ICX executes fewer modeled instructions; LCCC’s compact loop was marginally faster in this native sample — reported as measured, not as universal cycles.

## Metric-trap audit (Compiler Explorer)

For standalone `floor_f64` (`scripts/codegen_oracle.py`):

- GCC 16.2: **10** static insns — scalar, 1× work per iteration
- LCCC / Clang: **47** static insns — vector setup + full remainder paths, 4× work per hot iteration (F64x4)
- ICX: **24** static insns

**Lesson**: Static function size ranks GCC “best” while it does 4× less useful work. This is why **runtime + Callgrind** above, not static counts, are authoritative. LCCC and ICX have identical 2 memory ops in the packed path; LCCC’s hot body is `vroundpd mem,%ymm0` → `vmovupd %ymm0,mem` → `addq $32` → `jl`.

## After rebase — signed-guard fix

Initial revalidation on the rebased tree:

- LCCC packed: **13,147,955 Ir**
- LCCC scalar: **69,030,355 Ir** (5.25×)

Red-team found the byte-IV fusion declined dynamic signed `i64` bounds for multi-byte elements, keeping a redundant `addq $1` element IV in every hot iteration.

Preheader guard now: `n ∈ [0, i64::MAX/elem_size]` → `byte_limit = n/8 * 8*elem_size` via byte IV; else `0` packed trips → exact scalar mirror. Cost: 2–4 insns once in preheader, **−1 `addq` every hot iteration**.

After fix:

- LCCC packed: **11,020,010 Ir** (−16.18%, **6.26×** fewer than scalar)
- ICX: 10,804,117 Ir — parity
- Native (50 LCCC/ICX rounds): **4.004 ms vs 4.081 ms** median on this VM

## Machine-level confirmation

- **GDB 16.3** disassembly of linked `BENCH_MAIN`: `vroundpd (base,index),%ymm0`, `vmovupd %ymm0,(dst,index)`, loop control — no hidden stack traffic.
- **Linux `objtool` stack validator**: pass; warnings are expected kernel-ABI `%rbp` as scratch + `omit-frame-pointer` call, not malformed flow.
- **Pahole 1.30**: correctly reports C object has no DWARF/BTF type section — no layout claim made.

Raw logs and assembly dumps are archived with the PR; reproduce with `cargo build --profile fastbuild -j2 && CCC=target/fastbuild/lccc bash tests/regression/check_vectorize_round_maps.sh`.
