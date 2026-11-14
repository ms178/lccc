# 2026-11-13 — code-generation oracle baseline (static survey + paired runtime)

**Status:** measurement record. Nothing in this session's baseline changed the
generated code; the companion commit corrects two stale in-tree `TODO` claims
(audit CD-01/CD-02, `backlog.md` §15) and adds this record. Every number here
is a **screening** measurement on the Arena research host, not a bare-metal
claim: the host is a 2-vCPU virtual machine (Xeon at 2.60 GHz, hypervisor
detected) with **no PMU** (`perf` absent), so the wall-clock part is paired,
randomized, CPU-pinned and reported with bootstrap confidence intervals
exactly because the environment is noisy — never because the numbers are
"probably right".

Repository state: `4c3c08173` (`main`), LCCC built with `scripts/build_lccc_fast.sh`
(fastbuild profile, `-O1`, `-j2`). Oracles: Compiler Explorer pinned channels
(`cg162` = GCC 16.2, `cclang2310` = Clang 23.1.0, `cicc2021100` = ICC 2021.10.0,
`cicxlatest` = ICX latest) for the static survey, and locally installed
GCC 16.2 / Clang 23.1.5 for the paired runtime matrix.

## 1. Static survey — where the instruction gap is

`scripts/codegen_oracle.py --rank` over all 87 kernels in
`tests/benchmark/programs`, flags `-O2 -march=x86-64-v3`:

```
scripts/codegen_oracle.py --rank --local target/fastbuild/lccc \
    --oracles gcc,clang,icc,icx -j 4 \
    --json rank.json --markdown codegen-rank-x86-64-v3.md \
    tests/benchmark/programs/*.c
```

Result: **2389 instructions of aggregate gap** across 101 compared functions —
**72 behind, 9 tied, 20 ahead**. The full ranking is in
[`codegen-rank-x86-64-v3.md`](codegen-rank-x86-64-v3.md); 12 of the 72 "behind"
rows compare functions with *different call counts* and are explicitly marked
`Comparable: no` by the tool (it refuses to let a callee-body difference read as
a codegen regression).

**How to read this file, learned the hard way this session.** Three of the
largest rows are not codegen defects at all, and chasing them wasted time before
the mechanism was established:

| Row | Apparent gap | Actual mechanism |
|---|---|---|
| `libm_round_family:trunc` | 15 insns vs ICX 1 | ICX's single instruction is an artifact of that kernel's own `trunc` definition under ICX; LCCC emits **13** instructions and GCC **18** — LCCC is the *best* of the three here. |
| `libm_round_family:round_family_pass` | 82 vs GCC 27 | LCCC unrolls the 4096-element loop 4×; GCC does not. Per element the two are equivalent (one `vroundsd` per builtin in both). Instruction count is not runtime. |
| `expat_siphash24:sip24_final` | 158 vs GCC 81 | LCCC *does* emit a jump table; each of the 8 arms is reached through a `movq %rsi,%rN; jmp` trampoline (16 wasted instructions). Real, but a phi-copy/block-shape issue, not switch lowering. See CD-20. |

The lesson is mechanical: **a rank row is a question, never a verdict.** Open the
two assembly bodies and the callee set before acting (the tool's own
`ORACLE-METRIC-1` note says the same).

## 2. Paired runtime — the same kernels, measured

`tests/benchmark/run_benchmarks.py` with 11 rounds, 2 excluded warm-ups,
`--compilers lccc,gcc,clang` (GCC 16.2, Clang 23.1.5), taskset-pinned:

```
tests/benchmark/run_benchmarks.py --only zlib_ng_adler32,gzip_crc32,\
zlib_ng_compare256,expat_utf8_content,expat_xml_scan,zlib_ng_slide_hash,\
gzip_longest_match,sqlite_varint,linux_find_bit,glibc_strstr \
  --compilers lccc,gcc,clang --gcc gcc-16 --clang clang-23 --reps 11 \
  --json bench.json --markdown bench-lccc-gcc-clang.md
```

Headline (full table in [`bench-lccc-gcc-clang.md`](bench-lccc-gcc-clang.md)):

- **LCCC / GCC geometric mean = 1.0029** (arithmetic 1.0060) over 10 correct
  pairs — statistically a tie, decided by individual kernels in both directions.
- **LCCC / fastest reference = 1.0537** — Clang is the fastest reference on 8 of
  10 kernels, and it is the compiler to beat on this corpus.
- LCCC wins outright on `expat_utf8_content` (**0.873**, 1.15× faster than GCC)
  and `gzip_crc32` (**0.920**, 1.09× faster than Clang).
- Worst rows: `zlib_ng_compare256` (**1.205** vs Clang) and
  `gzip_longest_match` (**1.185** vs Clang).

Coefficients of variation were 5–25 % on the short kernels even after pinning;
the report prints them per compiler and never silently drops outliers. Treat only
rows with a CI that excludes 1.0 as established, and re-measure on the
i7-14700KF before any performance claim reaches a release note.

## 3. Root causes established this session (ranked by expected payoff)

1. **Constant-trip-count loops with an early exit are not unrolled (CD-21).**
   `zlib_ng_compare256` is a 32-iteration `do/while` with a data-dependent early
   return. Clang fully unrolls it (32 straight-line blocks, 4 instructions per
   8 bytes); LCCC keeps it rolled with 6 instructions per 8 bytes *and* a
   loop-carried branch. That is the entire 1.205 ratio. This is the single
   highest-value codegen item in the census: the idiom is exactly what
   `memcmp`/`ZSTD_count`/`compare256`/`longest_match` are made of, i.e. the
   hottest inner loops in the compression workload family.
2. **FP/vector constants are re-materialised as memory operands, never hoisted
   into registers (CD-22).** Every `.LCFP_n(%rip)` operand is re-loaded at each
   use; GCC hoists the mask into an XMM register outside the loop and reuses it.
   Measured in `libm_round_family`: LCCC issues 2 constant loads per copysign per
   element (8 per unrolled iteration); GCC issues 0 in the loop. On Raptor Lake
   this is load-port pressure in exactly the FP loops that dominate the
   `fir_filter`/`fp_product_map`/`matmul` family.
3. **Phi-copy trampolines after jump-table dispatch (CD-20).** 16 of 158
   instructions in `sip24_final` are `movq %rsi,%rN; jmp` pairs.
4. **`copysign` uses two distinct mask constants where GCC uses one.** LCCC:
   `vandpd sign_mask` + `vandpd abs` + `vorpd`. GCC keeps `abs` in a register and
   uses `vandnpd` (one mask, 8 fewer rodata bytes per function, one fewer
   distinct cache line). Folded into CD-22 as its smallest instance, because the
   `vandnpd` form is only a win once the constant is hoisted; in straight-line
   code it costs an extra `vmovsd`.
5. **No CPU-specific codegen versioning is needed for any of the above** — all
   four are wins at the `-march=x86-64-v3` baseline the corpus uses.

## 4. Environment record (what makes this reproducible)

| Field | Value |
|---|---|
| CPU | `Intel(R) Xeon(R) Processor @ 2.60GHz`, 2 vCPU, hypervisor: true |
| RAM / swap | 1.9 GiB / 4 GiB (`scripts/ensure_swap.sh`, active during the run) |
| PMU | none (`perf is not installed`) — no cycle/IPC/cache evidence in this record |
| Kernel | `Linux 6.1.189+` (sandbox), Debian trixie userland |
| LCCC | `4c3c08173`, fastbuild profile, binary SHA-256 `ccbe1ebb…` |
| Oracles | CE `cg162`, `cclang2310`, `cicc2021100`, `cicxlatest`; local GCC 16.2, Clang 23.1.5 |
| Pinning | `taskset` CPU 0, auto-applied |
| Rounds | 11 measured, 2 warm-ups excluded, randomized balanced blocks |

**Not claimed:** anything about i7-14700KF behaviour, anything derived from
`perf`, and anything about whole applications — these are kernels, and
`tests/benchmark/WORKLOAD_PROVENANCE.md` governs what each one may be used to
claim.
