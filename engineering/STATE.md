# Current compiler state

State refreshed against **`main`** (`ms178/lccc`) with Rust 2024 Edition / Rust 1.80–1.98.1 modernization, expanded 39-benchmark workload corpus, and verified Godbolt/Compiler Explorer competition oracles (GCC 16.2, Clang 23.1, ICC 2021.10, ICX latest).

The item catalog is [`agent/BACKLOG.md`](agent/BACKLOG.md); the active queue is [`tasks/`](tasks/README.md); the negative-results ledger is [`DECISIONS.md`](DECISIONS.md).

---

## What is production

- **Rust Modernization (Rust 1.80–1.98.1 / 2024 Edition):**
  - `std::sync::LazyLock` adopted across static caches, gates, and debug flags in passes, backend, and frontend.
  - Modern `Option::is_none_or`, `Option::is_some_and`, `Result::is_ok_and` throughout IR lowering, peepholes, passes, and backends.
  - Bounds-checked `slice::split_at_checked` in instruction decoders and encoders.
  - Clean unsigned integer `div_ceil` and `align_up` throughout stack layout and ELF writers.
- **C frontend** → SSA IR → `-O0` skip / `-O1` light / `-O2` full / `-O3` +unroll / `-Os`/`-Oz` size (`src/passes/README.md` is the authoritative tier list).
- **Linear-scan RA** in `src/backend/live_range.rs` (scan) + `src/backend/regalloc.rs` (policy). **Segment-aware interference is the scan's primary model** (RA-05a landed): `LiveRange::segs` + `segments_conflict`, coalesce-leader piece/use union, Phase-2f residual fill generalized to all targets and default-ON. Kill switches: `CCC_NO_SEGMENT_FILL`. Tier-2 hole-aware graph coloring is **production default** for eligible subsets (`CCC_NO_TIER2_GRAPH` restores scan-only).
- **MachInst Window Register Allocator:** SSA-driven MachInst window allocation with 85%+ instruction selection coverage.
- **ABI physical hints** (RA-26) retain leading ParamRefs across safe call-free x86 CFG leaves; ordered caller homes; stack-arg/mixed/calling shapes fail closed (`CCC_NO_LEAF_PARAM_GPR`, `CCC_NO_EMPTY_LOCAL_FRAME_ELISION`).
- **RA verifier**: `CCC_VERIFY_REGALLOC=1` hard-verifies segment interference, final assignments, and eviction occupancy history.
- **-O0** uses canonical stack homes on all four backends because phi elimination leaves non-SSA multi-def webs (RA-27).
- **Liveness** `src/backend/liveness.rs` — worklist backward dataflow (no `MAX_ITERATIONS` cap), fat `intervals` plus hole-aware `segments`.
- **FMA / FP contract**: `FpContract { Off, OnExpr, Fast }` threaded cli→pipeline→passes→backend. Default **Off** (GCC `gnu*` parity); `-ffp-contract=fast`/`-ffast-math` enable Fast; `-ffp-contract=on` contracts within a tagged statement root. Scalar and packed `vfmadd231*` emitters production.
- **Vectorizers**: reduction (single/multi/secondary accumulator), widening I32→I64 + masked conditional-sum, stencil (constant-tap affine), elementwise map expression trees, plain-copy; per-natural-loop PGO profitability gate. **Adler-32 rolling-checksum epic** (x86-64 AVX2, -O2+): the serial `s1 += b; s2 += s1` two-accumulator recurrence — left scalar by GCC/Clang/ICX at every level — vectorized 32 bytes/iteration via vpsadbw + vpmaddubsw(weights [32..1]) + vpmaddwd + a deferred `hsum(vs3) << 5` cross term, exact mod 2^32 for every input and length (ring-homomorphism proof); K ∈ {1,2,4,8,16,32} source unrolls, GEP and pointer-increment cursor spellings, signed/unsigned byte counters, register-homed accumulators and .rodata tables (8 vector instructions/iteration; measured 20.0 GB/s vs GCC -O3's 2.81 on 4 MB — 7.1×); fail-closed grammar (phi census, strict body whitelist, live-out discipline, no-side-effects). The byte-count epic gained the multi-accumulator decline, the limit-based IV live-out materialisation (immune to post-vectorize unroll restructuring), and the narrow-compare constant wrap (`x == 0xAA` on U8 streams). See `engineering/FOLLOWUP-2026-09-17-adler32-epic.md`. **BB-SLP v7** (basic-block, x86-64 -O2+): store-seeded pack graph with packed lane shifts (I16/I32/I64, both widths), rotate decomposition in both spellings, Not/Neg composites, the Sub(x,1) all-ones idiom, sub-word shift promotion (zext/sext-aware), the FP strict min/max fold, the 128-bit VEX memory fold (width-tagged pending tuple, commutativity-defended), FP negation via the one-instruction .rodata sign-mask composite, integer min/max folds for every relational spelling, the general cmp+blendv composite (width-exact granularity: vblendvps for dword lanes, vpblendvb for word/byte), the rule-(b) cross-block relaxation (extracts dominate every external use), mixed-run aligned-window seed fallbacks, the VEX three-operand FP compare, and the sub-word SELECT demotion (C integer promotion: i8/i16/u8/u16 selects and min/max pack at the lane width with zext/sext predicate remapping and out-of-range-constant rejection) — lane-exact under strict FP, no reassociation anywhere. See `journal/2026-09-W3.md` and `engineering/FOLLOWUP-2026-09-16B-bb-slp-v7-subword-selects.md`. **BB-SLP v8**: struct-field stream composition (GEP bases carrying index variables compose — the `a[i].f1/f2` unlock), the FIELD-DISJOINTNESS THEOREM (same-base/same-stride different-index streams with disjoint single-element windows never alias — full proof in source), per-lane byte-precision rules (c)/(d), the same-source splat (per-component field re-loads broadcast once), PackKind::Forward (chained seeds reuse one vector through its extracts — the j-velocity seed forwards the i-velocity seed's (dx,dy)), the PACKED FMA CONTRACTION with rounding parity (Add/Sub(acc, Mul(x, uniform-s)) → one vfmadd/vfnmadd — the tri-config differential demands the same single rounding the scalar gap-fused contraction takes; complete register-alias discipline over the three dying operands; VLFOLD accumulator fold), the scalar gap-FMA Sub extension (`p[i].v -= a*b*c` with the address chain in the gap → GCC's vmulsd+vfnmadd231sd), vmovddup broadcast fast paths, cross-seed splat CSE, register-sourced lane extracts — nbody advance: the pair body is GCC's exact shape, 0.38→0.24s measured (gcc 0.209s; fully scalar before). See `engineering/FOLLOWUP-2026-09-17B-bb-slp-v8-struct-fields-fma.md`.
- **Loop rotation** `src/passes/loop_rotate.rs` is **opt-in** (`CCC_LOOP_ROTATE=1`): correctness-clean for canonical counted loops.
- **DSE** `src/passes/dse.rs` (same-block, closed-alloca escape analysis, byte-range kills; `CCC_NO_DSE`); backedge PRE; GVN per-object epochs for disjoint non-escaping allocas + `restrict` params; GlobalAddr CSE with oracle-derived placement.
- **Aggregates**: AVX2 64-byte assignment = 2 YMM pairs + `vzeroupper`; 32/48-byte copies stay XMM. SysV all-SSE 16-byte struct returns use xmm0/xmm1.
- **Multi-arch**: x86-64, i686 (natural 4-byte slots, m16 boot pipeline, 32 KiB boot gate PASS), AArch64 (CASP, MOVW `:abs_g*:`, `.org`, PREL64, G1/G2/SABS reloc repair), RISC-V (va_arg struct{long double} end-to-end padding). Assembler + ELF linker in-tree for all four.

---

## Benchmark Corpus & Performance (39 Workloads)

The benchmark corpus contains **39 deterministic workloads** (33 historical + 6 new workload extracts: `chacha20_block`, `sha256_transform`, `linux_rbtree`, `zstd_count`, `lz4_compress`, `glibc_strstr`).

- **Aggregate LCCC / GCC Geometric Mean:** **`0.8598`** (LCCC faster overall than GCC 14.2 across the 39 benchmarks).
- **Aggregate LCCC / Fastest Ref Geometric Mean:** **`0.8936`**.
- **Correctness:** **39 / 39 (100%)** bit-identical to reference compilers.
- 2026-09-08 re-measurement after OP-05c/OP-05d (`--reps 3`, so indicative
  rather than certified — re-run at `--reps 7+` on a quiesced machine before
  quoting): LCCC/GCC geometric mean **`0.7597`**, correctness still 39/39.

### Elementwise map lowering — hot-loop density (2026-09-08)

Loop kernels must be ranked by **steady-state instructions per input byte**
(`scripts/hot_loop_metric.py`), never by `codegen_oracle.py`'s static
whole-function `insns` column: a compiler that refuses to vectorize emits
one tight scalar loop and "wins" that column while doing 32× less work per
instruction.  `/home/user/work/casefold.c`, `-O3 -march=x86-64-v3`:

| kernel | LCCC | GCC 16.2 | Clang 23.1 | ICC | ICX |
|---|---|---|---|---|---|
| `fold_lower` | **0.2812** (9 / 32 B) | 1.2500 | 0.2422 (4× unrolled) | 2.0000 | 0.6250 |
| `fold_upper` | **0.2812** | 1.2500 | 0.2422 (4× unrolled) | 1.3750 | 0.6250 |
| `clamp_bytes` | **0.1875** (6 / 32 B) | 0.1875 | 0.1172 (4× unrolled) | 0.4375 | 0.3750 |
| `classify_alpha` | **0.2500** (8 / 32 B, was scalar) | 0.3125 | 0.2109 (4× unrolled) | 4.2500 | 0.5625 |
| `short` clamp (16-bit) | **0.2188** (7 / 32 B) | — | — | — | — |
| `int` clamp (32-bit) | **0.2188** (7 / 32 B) | — | — | — | — |

LCCC beats every non-unrolled competitor on every kernel: 2.2× vs ICX and
4.4× vs GCC on the case folds, 2.3× vs ICX and 1.25× vs GCC on the
classifier, 2.0× vs ICX and level with GCC on the clamp.  Clang's remaining
lead everywhere is 4× unrolling of identical per-element work — the single
open item (follow-up doc TODO 3.1), projected to flip the case fold and the
classifier once done.

**Measure loop kernels with `scripts/hot_loop_metric.py`, and run
`scripts/ci_local.sh` before every push** — it mirrors all sixteen gates of
the `test`, `bench` and `clippy` jobs.  Two CI failures this program were
caused by validating against a subset: an AVX2 `!=` mask miscompile that only
the differential oracle saw, and a +24% instruction-count regression that
only the codegen-quality gate (in `bench.yml`, not `ci.yml`) saw.

The `isalpha` classifier now lowers to three packed instructions
(`vpor` c|32, `vpaddb` bias, `vpcmpgtb`) via the single-bit window union plus
the range fusion; it used to be a scalar loop.

---

## Competition Oracle Matrix (Compiler Explorer)

| Benchmark / Function | LCCC Insns | GCC 16.2 | Clang 23.1 | Intel ICC | Intel ICX (latest) | Status |
|---|---:|---:|---:|---:|---:|---|
| `glibc_strstr` (`two_way_short_needle`) | **75** | 150 | 89 | 94 | 155 | **Best in the world (1.00×)** |
| `zstd_count` | **67** | 77 | 37 | 69 | 68 | **Beats GCC 16.2, ICC, ICX** |
| `sha256_transform` | **231** | 154 | 126 | 171 | 1069 | **Beats ICX by 4.6×** |
| `chacha20_block` | 592 | 232 | 176 | 1098 | **74** | Open gap: AVX2 QR vectorization |

---

## Priority Research Backlog (Next High-Impact Targets)

1. **ChaCha20 Quarter-Round SIMD / Unrolling:** ICX generates 74 instructions via AVX2 vector quarter-rounds; LCCC generates 592 instructions due to scalar quarter-round spill pressure.
2. **LZ4 Match Loop & Store Forwarding:** LZ4 compression loop needs better unaligned word load forwarding and fast hash table index registers.
3. **Loop Rotation Default-Enable (PF-17):** Hardening remaining 15 edge-case loop shapes to enable loop rotation by default (eliminating the extra entry jump in hot loops).
4. **MachInst Instruction Selection Coverage Expansion:** Expand MachInst window allocator to 95%+ of instruction forms.

## Kernel-mode status (linux-cachymod-6.18.52, lccc + lccc-ld only)

- **M1 vmlinux link PASS** — 21.5 MB vmlinux, pure lccc-ld, 0 dynamic
  relocations, vdso phdr parity with GNU (regression
  `script_vdso_declared_note_phdrs`).
- **M2 bzImage PASS** — `m16` boot pipeline, i386 real-mode setup,
  zstd compressed image (5.0 MB), reproducible sha256.
- **M3 boot** — kernel boots to `run_init_process` with zero phantom
  NULs in the serial log (MachInst sub-word arriving-reload fix,
  `arriving_reload()` in `machinst_alloc.rs`, regression
  `machinst_subword_reload_sign.c`).
- **Defect (d) ROOT-CAUSED + FIXED (2026-09-17)** — every execve failed
  with -E2BIG because `pte_mkwrite` (by-value pte_t + inlined helper
  chain) returned leftover stack as the PTE: aggregate_sroa's
  copy-buffer collapse deleted the initializing Memcpy of a buffer a
  forwarded load had just been re-pointed at (`forward_target_roots`
  guard; regression `sroa_fwd_load_vs_buffer_collapse.c`). Kernel
  rebuild/boot validation of the fix still pending.
- **Defect (e) FIXED (2026-09-18, commit c256c917)** — the
  `aes-ctr-avx-x86_64.S` failure was the tip of FOUR assembler
  defects, three of them silent object corruption: (1) VAES
  (vaesenc/vaesenclast/vaesdec/vaesdeclast) had no EVEX forms
  (0F38.W0 DC..DF) for zmm/ymm16-31 operands; (2) same gap for
  vpsrldq/vpslldq (0F.73 /3, /7); (3) `:vararg` macro parameters
  bound a single argument instead of the whole remainder — `.irp i,
  \vecs` inside `.macro _xor_data vecs:vararg` emitted 1 of 8
  iterations, so the assembled AES-CTR object was missing 7/8 of its
  rounds with no diagnostic (both macro engines: asm_preprocess +
  x86 parser); (4) `.octa` silently emitted nothing (unknown-directive
  fallthrough), zeroing `.Lbswap_mask`. Post-fix, lccc's kernel object
  matches GNU as byte-for-byte in .rodata and instruction stream
  (264/264 AES rounds), modulo the intentional commutative-vpxor
  VEX.2 shortening and NOP padding style. Regression coverage:
  `tests/asm-diff/vaes.casefile` (6 groups incl. reject parity:
  VAES has no opmask/broadcast forms) and
  `tests/asm-diff/macro_vararg.casefile` (4 groups: kernel pattern,
  forwarding, empty/single invocation).
- **Defect (b) OPEN** — CPU1 hotplug bring-up times out (`maxcpus=1`
  boots fine).
- Kernel harness scripts (`build_kernel_vm.sh`, `prepare_kernel_tree.sh`
  with whole-tree extraction audit, `qemu_boot_test.sh`) are unreviewed
  deltas in the session patch, not upstream.
