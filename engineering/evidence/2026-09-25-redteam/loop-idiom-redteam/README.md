# Latest-main loop-idiom red team: correctness, generated code, and limits

**Base:** `ms178/lccc main` at `7ddb770f56027e1d9e71e2bf60645bdf5c9297b2` (25 September 2026). **Candidate:** rebased branch commit `3313ef9caef5bdf910226d94cd9a69a86606c561`, followed only by documented test/host-gate and report edits. This is a report of measured results, not a claim that every P0 optimization is finished. Code speed matters, but wrong results invalidate a timing arm. The older `0229885c` measurements in adjacent directories are historical; all current-main comparison numbers below use the separately built `7ddb770f` binary.

## 1. Preservation and environment

Before each rebase, in-flight work was preserved. The first checkpoint is `arena/snapshot-before-upstream-merge-2d8f9d1a` plus `/home/user/ms178-pre-rebase-inflight.patch`. Before rebasing again onto `7ddb770f`, the loop fix and tests were committed, while `/home/user/ms178-inflight-before-second-upstream-rebase.patch` and `/home/user/ms178-evidence-before-second-upstream-rebase.tar` captured the working diff and untracked evidence. No uncommitted work was discarded. The 8 GiB `/swapfile` stayed active throughout builds/tests/benchmarks; Rust builds used two or fewer jobs, a constrained allocator, and the existing fastbuild profile. The previous-main and latest-main binaries were each preserved separately.

The latest-main compiler SHA-256 is `ae661ea32d2469e34f396270e9013ae51483ec52d08d7818b863fe34bbe66d24`; the **benchmark-measured candidate** SHA-256 is `7b324bff70bd1b0d54b98edf6598fddbfaafe55c486470434f863fb039985973`. Neither binary is part of the portable patch: rebuild both from their named commits for target-hardware A/B. The measured revision and exact compiler binaries/commands are recorded in `latest-tip-corpus/results.json`.

## 2. Counterexamples and the accepted fixes

1. **Forward-overlap smear (P0 miscompile):** a counted loop copying `dst[i] = src[i]` with `dst=src+1` repeatedly reads its own prior store. Current main emits unconditional `memmove` for distinct parameter names; that library call preserves the original source bytes and is **not equivalent** to the scalar loop. The 8,192 deterministic randomized/288 global-copy test cases in `tests/regression/loop_idiom_overlap_runtime.c` return scalar digest `4238867d` on GCC and this patch. Current main returns the wrong digest `27a5e1d3` and exits 9. Separate first-principles parameter reproducers also showed the old `01010203...` bytes versus correct smeared `01010101...` bytes. See `latest-tip-correctness.txt`.
2. **Aliased global names, including separate translation units:** names are not object identities when one is a GNU/ELF symbol alias. Gather *both* ends of `IrModule.aliases` and explicit asm-label redirects. An unknown extern declaration cannot prove global disjointness: even when the alias is declared in another translation unit. Only distinct strong **private** definitions, with no alias/weak/common/unknown top-level asm, may prove two global roots disjoint. `tests/regression/loop_idiom_extern_alias/{use,defs}.c` returns 1 on current main and 0 with this patch. This is an ELF alias extension: GCC `-O2` also assumes the two invisible extern names are distinct and returns 1; GCC `-O0` and the original scalar loop return 0. Do **not** cite GCC `-O2` as the oracle for that cross-TU extension. The single-TU cases are GCC-differential.
3. **Bound load hoisted across an aliased store:** the header reloads an unsigned bound; writing through a `char *` parameter can change it mid-loop. Old `memmove` with a hoisted count prints `1114114`, while the original loop exits early at `2`. GCC and the patch print `2`: `tests/regression/loop_idiom_bound_alias.c`. A bound load is hoistable only when its root is **provably** disjoint from both the load and store roots, never merely a different name or SSA value.
4. **Sound fast path without sacrificing LZ4:** for uncertain roots, form unsigned `delta = (uintptr_t)dst - (uintptr_t)src` and take `memmove` only when `delta > (size_t)(n-1)`; otherwise execute the original scalar loop. Forward overlap has `0 < delta < n` and therefore always takes the scalar path. `dst <= src` is safe for memmove; a wrapped subtraction may conservatively choose scalar. With `n=0`, `n-1` is the unsigned maximum, so even null pointers take the zero-trip scalar path, never a libc call. For *proven-disjoint* copies with a nullable parameter, a separate `n>0` guard permits fast `memcpy` while keeping zero-trip null semantics. Neither rule is a benchmark-name special case or an unchecked pointer-order comparison.
5. **SSA and fail-closed matching:** version the preheader and keep the original loop, merge IV and bump-pointer direct live-outs at the exit, and extend existing exit phis for the new fast predecessor. Reject unsupported bound-load, operand-availability, additional-phi, and multi-predecessor-exit shapes *before mutation*. A per-pass header set prevents matching and versioning the retained fallback again. Terminator uses are now checked/rewritten via the canonical walker instead of Return-only handling; loop-defined copies that do not dominate the new edge cannot masquerade as live-out phis. Tests exercise pointer/IV returns and IR/SSA verification.

The earlier global-DSE alias fix from the rebased checkpoint also remains in the patch. The host-only i686 ISA-parity script now skips **only** runtime execution when the i686 ELF interpreter is absent; assembly, veto, and macro assertions still run. On an interpreter-equipped host the real exit-code assertion remains mandatory.

## 3. Verification performed

| Gate | Result / scope |
|---|---|
| `CCC_VERIFY_IR=1 CCC_VALIDATE_SSA=1` complete regression corpus on latest-tip candidate | **797 passed, 0 failed, 13 GCC comparisons unavailable, 9 i686 executions/builds host-skipped; 819 total**, `latest-tip-regression-ssa.json`. Host lacks 32-bit libc headers/interpreter; these are not silently counted as passing executions. |
| `scripts/ci_local.sh --fast` plus its three slow gates run separately | **74 pass, 0 fail, 3 intentionally skipped in fast run**. All three skipped gates were run separately against the same compiler/source: full SSA regression (above), benchmark outputs **204 pass / 0 fail / 0 skip** at all levels, and peephole whitespace invariance **PASS** (236 newly generated assembly files, 28 assembled object pairs). `cargo test --all-targets` included **3,491 Rust library tests passed, 7 ignored**; Clippy and Rust formatting both passed. This is a composition of gate runs, not a claim that a single unmodified full CI command ran. |
| Latest-tip 39-workload paired strict runner, `-O2 -march=x86-64-v3 -mtune=raptorlake` | **39/39 candidate/current-main/GCC outputs and exit statuses match**, nine paired randomized rounds, two discarded warm-ups, no outlier deletion, CPU pinned by runner. Raw samples/commands and disassembly: `latest-tip-corpus/results.json`, `latest-tip-corpus/results.md`, `latest-tip-corpus/artifacts/`. |
| All levels on randomized overlap suite | GCC and LCCC `-O0`, `-O1`, `-O2`, `-O3`, `-Os` all produce `4238867d` with IR/SSA checks on LCCC. At `-O2` the compiler logs actual guarded rewrites for both indexed and bump forms. |
| Decision and ELF-alias gates | `tests/regression/check_loop_idiom.sh` pins private-global memcpy, guarded param memmove **with scalar path**, kill switch, near misses and cross-TU runtime smear; all pass. Bound-alias and null/zero-trip C regressions are part of the normal corpus. |
| Godbolt/Compiler Explorer scripts | `scripts/godbolt.py audit` confirms current pinned GCC 16.2, Clang 23.1, ICC and moving ICX; `scripts/codegen_oracle.py --rank` reruns seven affected/P0-relevant sources against the final codegen and saved historical ranking. Outputs: `latest-tip-oracle/audit.log`, `latest-tip-oracle/rank.json`/`.md`, `latest-tip-oracle/raw/`. Static gaps alone are not runtime results. |
| 32-bit guard codegen | `-m32 -S` with IR/SSA verification emits an unsigned 32-bit distance/length guard and retains scalar load/store; runtime i686 is not available on this host. A requested AArch64 `-S` attempt unexpectedly emitted x86-64 text under the local cross setup: **not** claimed as AArch64 validation. |

The current VM identifies as a **virtualized Intel Xeon 2.60 GHz**, not an i7-14700KF. The hardware PMU (`perf`) is unavailable. The i7-14700KF remains the required performance target; the measurements here are same-host VM screens only. The existing `../post-merge/run-on-i7-14700kf.sh` refuses the wrong CPU and records hashes, raw randomized rounds, output checks, and a PMU probe. Set `BASE_LCCC` to a compiler *built from `7ddb770f`*, choose an isolated P-core via `CPU=...`, then run on the actual target. No Raptor Lake speedup is asserted without that run.

## 4. Latest-main performance evidence (screening, not target hardware)

For the **only changed `.text` among 39 workload executables**, `lz4_compress`, direct paired scaled runs used the same source, flags, input, host and `PASSES=12288`; each arm lasted more than 200 ms. Twenty-five interleaved/alternating rounds and three warm-ups checked outputs before timing:

| Pair, numerator/denominator | Paired median or per-arm median | Min ratio | Sign test | Interpretation |
|---|---:|---:|---:|---|
| Guarded / latest-main unsafe | **0.9946** (223.36 / 224.57 ms) | 1.0057 | `p=0.2301` | No demonstrated speed difference; median and minimum disagree. |
| Guarded / scalar (`CCC_NO_LOOP_IDIOM=1`) | **0.0929** (218.34 / 2350.90 ms) | 0.0919 | `p<0.0001` | Retains about 10.8× speed versus disabling the idiom; both arms correct on this input. |
| Guarded / GCC | **1.0072** (218.66 / 217.10 ms) | 1.0135 | `p=0.4237` | No demonstrated difference on this VM. |

The three full sample series are `lz4-7ddb-unsafe-guard-vm.json`, `lz4-7ddb-guard-scalar-vm.json`, and `lz4-7ddb-guard-gcc-vm.json`. The unsafe latest-main arm agrees on this *non-overlap benchmark input* but fails the independent correctness reproducers: it is not a valid general optimization. The earlier `0229885c` scaled-pair results remain in `lz4-*-vm.json` only as historical evidence.

Across the **latest-main** three-arm 39-workload run, candidate/current-main paired geometric mean is **1.0011** (screening noise; `.text` is **byte-identical in 38/39** workloads). No paired median regresses by more than 5%; the largest apparent difference, `fib` 1.0326, has identical executable `.text` and is not a codegen effect. LCCC/GCC paired geometric mean is **0.8007** in this VM corpus but is dominated by short recursive benchmark/startup artifacts and is not a target-CPU claim. Current main and candidate share the remaining measured gaps: `spectral_norm` ~27.85× on this VM, `mandelbrot` ~1.286×, `linux_find_bit` ~1.318×, `sha256_transform` ~1.175× GCC. Their code is unchanged in this patch. `lz4_compress` grows from 2,020 to 2,489 executable `.text` bytes; oracle static `main` moves from 250 to 314 instructions (+64). That is the cost of retaining the correct scalar fallback; the scaled measured runtime is nevertheless near parity.

## 5. P0 triage, negative findings, and remaining work

- **PF-LZ4-1:** the unsafe byte-copy rewrite is fixed with a non-smear fast path and verified workload coverage; a word-at-a-time byte-compare rewrite is **not** shipped because it needs a proof that widened loads cannot overread either range. Strong VM parity is not a substitute for i7-14700KF A/B.
- **PF-MB-1 / PF-FB-1:** Mandelbrot vectorization of extra non-IV recurrences and `linux_find_bit` branching remain open. Their generated `.text` is identical across latest-main A/B; changing either without a legal SSA/recurrence proof or a measured performance win would violate the correctness/performance criteria.
- **RA-GLA-04 and related register pressure:** the previous controlled spill-gap prototype *slowed* `sqlite_varint` by 1.60% and `expat_xml_scan` by 4.51% despite isolated static instruction improvements; see `../post-merge/README.md`. A large unverified allocator rewrite or function-name exception was therefore rejected. The existing source-less rematerialization stays in place.
- **Host/test limitations:** Rust, fuzz and Clippy gates should be rerun after any source edits; missing 32-bit runtime is a skipped execution, not a pass. There is no claimed native i7 result. These follow-ups are explicitly open, not mislabeled as completed P0 improvements.

To reproduce the current screen on a suitably provisioned host, build the latest-main and candidate compilers from the exact commits above, then run:

```sh
python3 tests/benchmark/run_benchmarks.py --compilers lccc,ccc,gcc \
  --lccc /path/to/candidate --ccc /path/to/7ddb770f-baseline \
  --opt=-O2 --cflag=-march=x86-64-v3 --cflag=-mtune=raptorlake \
  --reps 9 --warmup 2 --seed 250926 --cpu auto --strict \
  --json results.json --markdown results.md
```

The runner's arm key `ccc` is only a label for the **current-main LCCC binary**, not an Anthropic compiler. Always verify hashes and input/output before interpreting timings.
