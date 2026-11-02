# Historical post-merge red-team audit and codegen measurements

**Historical snapshot: 25 September 2026, NOT the current `main`.** This work was based on `ms178/lccc` at `9667a59f321e5b5806af447b58e50be96c60ac3e` (the merge of PR #617). In-flight work was committed as the rebased checkpoint `d80c0c74` *before* that rebase; earlier measurements on `5624367b` remain in the parent evidence directory. The historical code fix (`3e471685`) stopped global dead-store elimination for linker-visible aliases. Its source and regression are `src/passes/dead_statics.rs` and `tests/regression/dead_static_aliased_global.c`. None of the compiler hashes or ratios below should be presented as measurements of a later `main`; see the latest-tip report in `../loop-idiom-redteam/README.md` for that.

## 1. Critical correctness finding: stores observable through a linker alias

```c
static int hidden;
extern int published __attribute__((alias("hidden")));
void update(int v) { hidden = v; }
/* Another translation unit may read published. */
```

**Counterexample to the old closed-translation-unit assumption:** the `globaldse` from PR #617 searched for readers/escapes of `GlobalAddr("hidden")`. An alias reader uses `GlobalAddr("published")`: a different symbol for the same address. A reader in another translation unit has no node in the current IR at all. On the unmodified historical HEAD, GCC printed `123 456|0` but LCCC printed `0 0|1` with `-O2` (`prepatch-alias-failure.txt`). GCC, Clang and the preserved previous LCCC printed `123`; the ELF symbols `hidden` and `published` had the same address.

**Fix:** collect both ends of every `IrModule.aliases` entry once and exclude those names from global-DSE candidates. This is a semantic escape check, not a benchmark/function exception, heuristic knob, or blanket disablement. Rust tests cover strong and weak aliases and subsequent static elimination; the GCC-differential C test covers both strengths. `repro/alias_writer.c` plus `repro/alias_reader.c` exercises genuine separate translation units. All 20 combinations of five optimization levels and four compilers (previous LCCC, fixed LCCC, GCC, Clang 19) returned `123` with status 0 (`cross-tu-alias-differential.json`).

Correctness takes precedence over speed: a fast wrong result is not a valid timing arm. A diagnostic experiment with raw `inline asm` naming `hidden` without a C operand (`repro/inline_asm_hardcoded_symbol.c`) failed to link with GCC and Clang at `-O2` as well. It does not justify blindly applying an additional transformation to all asm templates; see `inline-asm-raw-symbol-diagnostic.json`.

## 2. Historical validation of the alias fix

| Gate | Historical result | Evidence |
|---|---|---|
| Fastbuild, Rust 1.98.1, `-D warnings` | Pass; compiler SHA-256 `39180a95cb8dedb6fb2b3b1a5b9150ad851fda65a13cfacdc73ac2f19388d5c6` | Historical `target/fastbuild/lccc` |
| `cargo fmt --all -- --check`; `git diff --check` | Pass | Commands |
| Rust library tests, two threads | 3,432 passed, 0 failed, 7 already ignored | `cargo test --profile fastbuild --lib -- --test-threads=2` |
| Full C regression with IR verification and GCC/A-B comparison | 760 pass, 0 fail, 8 unavailable GCC comparisons; no A/B disagreement | `full-regression-summary.txt` |
| i686 | Installed 32-bit libc/multilib and `qemu-i386`; all 16 filtered i686 cases passed in that **historical** environment; full corpus rerun | Test logs; boot-size gate skipped without a kernel tree |
| 39 registered workloads, strict output gate | All 39 LCCC/GCC/Clang 19 stdout/status pairs matched | `corpus-correctness.json` |
| Forced GLA spill and intra-block gaps, IR and allocator verifiers | 39/39 GCC-matching outputs; mechanism check **not shipped** | `gla-gap-corpus-correctness.json` |
| FP Select/union-punned copysign, 8,192 inputs including signed zeros, infinities, NaN payloads | All 35 compiler/optimization/pass-disable combinations matched | `repro/fp_select_copysign.c`, `fp-select-copysign-differential.json` |

The first complete suite failed six i686 *builds* because 32-bit libc headers were missing; this was not a compiler validation success. Only after installing `libc6-dev-i386`, `gcc-multilib`, and `qemu-user` was that suite rerun to completion. Its `SKIP=8` partially counted GCC comparisons in addition to passing LCCC programs, not eight magically passing comparisons. A missing prepared kernel tree independently skipped the boot-size gate.

## 3. New baseline at that time, not a current-main measurement

`workloads-vm.json`/`.md` measured 39 correct LCCC/GCC pairs on the same source/flags/input/host with `-O2 -march=x86-64-v3`: two discarded warm-ups and nine paired rounds per workload in randomized compiler order, pinned to CPU 0. Raw samples and bootstrap intervals are in JSON. Ratio is LCCC/GCC; **greater than 1 means slower**. The previous column was a separate historical run from `../head-39workloads-vm.json`, *not* an interleaved before/after A/B.

| Workload | Before PR #617 | Historical post-fix compiler | Finding |
|---|---:|---:|---|
| `spectral_norm` | 2.344 | 2.374 | Largest gap *in that run* |
| `nbody` | 1.416 | 1.420 | No defensible change |
| `hash_table` | 1.302 | 1.264 | Separate VM runs do not demonstrate a merge speedup |
| `expat_xml_scan` | 1.246 | 1.252 | No progress |
| `sha256_transform` | 1.185 | 1.189 | No progress |
| `chacha20_block` | 0.859 | 0.847 | LCCC faster than local GCC in that run |

**Code identity rather than spurious timing precision:** 37 of 39 pre/post LCCC benchmark ELFs were byte-identical. `linux_find_bit` changed relocation/data addresses but not hot-function mnemonics or length. `libm_round_family` changed code substantially: global-DSE removed stores to the never-read `out` array (`libm-assembly/`). Its *directly interleaved* before/after test (41 rounds, three warm-ups) measured a mere 0.20% median improvement, sign-test `p=0.0609`: **not a statistically established speedup**. `linux_find_bit`: 0.05%, `p=0.2115`, median/min pointing in different directions: noise. Raw samples: `pre-vs-post-code-identity.json`, `libm-pre-vs-post-paired.json`, `linux-find-bit-pre-vs-post-paired.json`.

The measured compiler's SHA-256 was `e634c905d1cd277afa058746e846bc2dd1f9368c1360de338e6ba9679935b449`; the final compiler was rebuilt after an implementation-only alias-set precomputation (hash above). Repeating the same 39 commands proved all *loaded ELF sections* identical in name, address, size and bytes (`final-compiler-output-identity.json`). Whole ELF files differed in non-loaded metadata; no whole-file identity claim is necessary.

A refreshed Compiler Explorer ranking (`oracle-current-rank.json`/`.md`) compared 14 sources against GCC 16.2, Clang 23.1, ICC, ICX, and local LCCC with `-O2 -march=x86-64-v3`. Static totals remained LCCC 2,871 vs GCC 2,324 instructions. A same-named function's instruction-count gap is **not runtime evidence** when compilers inline or specialize different work. For `spectral_norm`, assembly instead showed LCCC building four integer denominators scalarly before a 4-wide `vdivpd`, while GCC constructed them in vectors. The map vectorizer correctly rejected additional non-IV header phis; that legality check was not weakened without a recurrence/live-out proof.

## 4. Piecewise GPR locations and next-use: prototyped, rejected

The existing global location allocator already models Register↔Stack with capture stores, reloads under new SSA names, CFG edge repair and Belady-style next-use planning. Shipped code excludes spill gaps and intra-block gaps. On the then-current compiler, forcing `CCC_GLA_SPILL_GAPS=1` over 51 C sources changed 20 functions: **7,527→7,630 instructions (+103), 661→761 stack references (+100, +15.1%)** (`gla-crossblock-only-census.json`). Adding `CCC_GLA_ALLOW_INTRA=1` changed nothing else in that corpus (`gla-spill-gap-census.json`). Even the best-looking static case, `sqlite_varint` (−5 instructions, −3 stack references), was **1.60% slower** in extended paired execution (`PASSES=240`, 31 rounds, three warm-ups, consistent median/min, `p=0.00008`). `expat_xml_scan` with `PASSES=320` was **4.51% slower**, `p=0.00002`. Outputs/statuses agreed and arm binaries differed (`gla-gap-*-paired.json`).

**Decision:** do not ship a global or function-name switch or infer a major allocator rewrite from a negative experiment. Preallocation pressure is not feedback from actual assigned registers and slots; extra captures/reloads can disrupt well-folded memory operands or callee-save tradeoffs. A defensible future general prototype would use *post-allocation* slot traffic, loop costs and verified next-use transitions, then rerun target-hardware A/B. Existing enabled rematerialization was left unchanged; no broad runtime or Raptor Lake win is claimed here.

## 5. Hardware boundary and repeatable target test

The development host identified itself as virtualized `Intel(R) Xeon(R) Processor @ 2.60GHz` with AVX-512, **not** the i7-14700KF. `perf stat -e cycles,instructions` reported both hardware events as `<not supported>`. VM wall time is paired local screening only; queried uops.info/Raptor Lake models are not measurements on the target CPU. `run-on-i7-14700kf.sh` refuses to run unless it detects a real i7-14700KF (local refusal tested: exit 3), keeps `-march=x86-64-v3 -mtune=raptorlake` identical for both compilers, pins execution to a chosen CPU (`CPU=...`; the operator must select a P-core) and saves raw samples, compiler hashes, assembly and a PMU probe. `BASE_LCCC=/path/to/unpatched/compiler` adds a third randomized paired arm. **Target-hardware measurements were not available**: no hardware-specific speedup is ratified here.
