# FOLLOWUP 2026-11-19 — Arena session: kernel boot gate, codegen oracle, snapshot safety

Status: **in progress, validated items marked ✅**. This document is the entry
point for the next agent: it records what is proven, what is measured, and what
is deliberately NOT done (with the reason), so no one re-derives the dead ends.

Environment for every number below, unless stated otherwise:

| Item | Value |
|---|---|
| Host | KVM guest, 2 vCPU Intel Xeon 2.60 GHz model, **no PMU**, 1.9 GiB RAM + 8 GiB swap |
| OS | Debian 13 (trixie), glibc 2.41 |
| Rust | 1.99.1 (58cf8f9c3 2026-10-13), `fastbuild` profile (-O1, no LTO, -j2) |
| LCCC | this tree, `target/fastbuild/lccc`, `--version` 16.2.0 |
| Oracles | GCC 16.2.0, Clang 23.1.5, binutils 2.47 (ld.bfd), mold 3.0.0, GDB 18.1, QEMU 11.1.3 (all from `harness-cache-8`) |
| CE oracles | `cg162` (GCC 16.2), `cclang2310`, `cicc2021100` (ICC 2021.10), `cicxlatest` |
| Kernel | linux-6.18.63 + the `linux-cachymod-6.18` patch series (PKGBUILD pkgrel 2.1) |

---

## 1. Environment bring-up (all ✅)

The harness wiped the workspace between sessions, so nothing was inherited. The
rebuild path that works, in order (each step is minutes, not hours):

1. `sudo fallocate -l 8G /swapfile && sudo chmod 600 /swapfile && sudo mkswap
   /swapfile && sudo swapon /swapfile`. The 8 GiB figure is not a preference:
   `scripts/ensure_swap.sh` records the OOM evidence behind it. Without swap the
   fastbuild rustc peaks at ~1.7 GiB RSS on a 1.9 GiB host and the build dies
   silently.
2. `python3 scripts/restore_agent_toolchain_cache.py --tag harness-cache-8
   --only rust,gcc,clang,binutils,gdb,mold,qemu`. **This is the sanctioned
   path** and it verifies every blob against the tag manifest. Note
   `arena_session_restore.sh` hard-fails when `$HOME/.cache/lccc/agent-toolchains/env.sh`
   is missing, so run this first on a fresh sandbox; the tag is
   `harness-cache-8` (commit `b99a4457`), not the `harness-cache-4` GitHub
   *release asset* — the release page only exposes asset-style files, the tag
   carries the full cache.
3. `export PATH=$HOME/.cache/lccc/agent-toolchains/bin:$PATH` (wrappers:
   `gcc-16`, `clang-23`, `rustc`, `cargo`, `mold`, `gdb`).
4. `export LD_LIBRARY_PATH` from `env.sh` if a cached binary complains about a
   missing `libstdc++`/`libLLVM`.
5. `./scripts/build_lccc_fast.sh` — **7m49s cold, 3m36s incremental**. Always
   `LCCC_BUILD_JOBS=2`; the wrapper's memory heuristic will otherwise reduce it
   to 1 on this host.
6. `./scripts/prepare_kernel_tree.sh` — ~6 min, downloads/verifies the
   6.18.63 tarball (sha256 `81764a54…`) and applies the CachyMod series at
   `$HOME/.cache/lccc/archpkgbuilds/packages/linux-cachymod-6.18`. It runs
   `make prepare` with **CC=lccc** and stamps `.lccc-prepared`.

7. `bash scripts/ensure_i386_multilib_safe.sh` — **required before
   `ci_local.sh --fast` is green.** Two gates (`gcc17-builtin-semantics`,
   `type-attr-static-init`) build a `-m32` runtime, and a fresh sandbox has no
   i686 glibc headers, so they fail with `bits/libc-header-start.h: No such file
   or directory` — a failure that looks like a *compiler* defect and is not. The
   script is the audited path: it apt-dry-runs first and refuses any transaction
   that would upgrade or remove the live `libc6`, then installs biarch multilib
   (measured: ~21 s, followed by `i686 runtime: PASS`).

Tooling that is NOT in the sanctioned cache and was installed with `apt`
(oracle-side only, never for kernel or benchmark target code): `cmake`,
`ninja-build`, `clang` (host fallback), `qemu-system-x86`, `pahole`, `gdb`,
`ccache`, `libarchive-tools`, `zstd`.

---

## 2. Linux kernel, lccc + lccc-ld ✅ (boot code) / ⏳ (full bzImage)

### 2.1 The 32 KiB setup gate — **PASS**

`scripts/build_kernel_boot.sh` (23 real-mode objects, linked by `lccc-ld`
against the kernel's own `arch/x86/boot/setup.ld`, ASSERT `_end <= 0x8000`):

```
.bstext     495   .header     125    .entrytext  104    .inittext  392
.initdata    30   .text    22219    .text32      32    .pecompat    9
.rodata    1348   .videocards 84    .data       124    .signature   4
.bss       4924
32 KiB gate: PASS (_end=31088, headroom=1680 bytes)
ORACLE ld.bfd: PASS (flat setup image byte-identical)
ORACLE ld.lld: PASS (flat setup image byte-identical)
kernel_boot_isa: scan PASS (24 objects, 0 BMI/ABM/VEX/SSE4.2-class instructions)
```

Three independent facts are being established here, and all three matter:

1. `_end = 31088` — **1 680 bytes** of headroom under the historic blocker.
2. `lccc-ld`'s flat `setup.bin` is **byte-identical** to `ld.bfd`'s and
   `ld.lld`'s for the same objects and script (`sha256 8466e696…`). The linker
   is not merely accepting the input; it is producing the same image.
3. The real-mode ISA scan finds no instruction the boot CPU cannot execute —
   the i686 boot pipeline does not pick up BMI/VEX from the host's feature set.

**Watch item for future sessions:** headroom is 1 680 B and the boot code is
regenerated from the kernel tree, so a compiler change that widens a real-mode
function by >5% (currently ~48 KiB of real-mode text would fit) can re-break the
gate. Re-run `scripts/build_kernel_boot.sh` before claiming a codegen win on
i686, and read `scripts/boot_size_oracle.sh` for the per-function census.

### 2.2 Full bzImage with `build_kernel_vm.sh` ⏳

`scripts/build_kernel_vm.sh` builds a QEMU-bootable minimal configuration
(`allnoconfig` + `scripts/kernel-vm.fragment` + CachyMod features via
`olddefconfig`/`syncconfig`), all target code with LCCC and all target links
with `lccc-ld`, `HOSTCC=gcc` for build machinery only.

Observed log (still running at the end of this session's measured window):
**404+ objects, zero `error:` lines**, all flags accepted. The only lccc output
is the *expected* tolerance list for GCC options LCCC does not implement
(`-fmodulo-sched`, `-fivopts`, `-fconserve-stack`, `-fzero-init-padding-bits=all`,
`-fstrict-flex-arrays=3`, `--param=allow-store-data-races=0`, …). Those warnings
are informational; `LCCC_STRICT_OPTIONS=1` would make them fatal and is the
right setting for a *new* option to prove it is at least parsed.

Next step (in this order): let the build reach `bzImage`, then
`QEMU=$(scripts/ensure_qemu_system.sh --print-bin) scripts/qemu_boot_test.sh`,
then `build_kernel_full.sh` with the package config (`LCCC_RECONFIG=1`) and
boot that. The boot test asserts the CachyMod feature set from
`/proc/config.gz` (SCHED_BORE, SCHED_CACHE, SCHED_CLASS_EXT, HZ_800, CACHY,
TCP_CONG_BBR, PREEMPT, SMP), BBRv3 loadability, the sched-ext sysfs ABI, BORE
stats in `sched_debug`, both vCPUs online, and a clean poweroff — a boot that
merely prints a banner does not pass.

---

## 3. New tool: `scripts/godbolt_codegen_compare.py` ✅

The research program asked for "a python script derived from `godbolt.py` to
compare codegen between all listed compilers". `scripts/codegen_oracle.py`
already surveys, but it hands ONE `--flags` string to every compiler, which is
wrong exactly where it matters:

> Intel ICC Classic 2021.10 predates `-march=x86-64-v3`. Given it, ICC prints
> `command line warning #10148: option not supported` **and compiles at its
> default ISA**. A first survey run therefore produced an ICC row of 1 864
> instructions against LCCC's 748 on `zlib_ng_huffman_heap.c` that was really
> SSE2-vs-AVX2 — a comparison that would have been quoted as a win.

`godbolt_codegen_compare.py` compiles the *ISA intent*: `-march=x86-64-v3` /
`raptorlake` / `alderlake` / `znver3` → ICC `-march=core-avx2`;
`x86-64-v4` / `icelake-*` / `znver4` → `-march=core-avx512`; `native` dropped
and the row labelled a *lower bound*; everything else verbatim. All translations
are printed, stored in the JSON manifest and reproducible with `--strict-flags`
or `--icc-march=`. It reuses `codegen_oracle._stats`, `_split_function_bodies`,
`_local_compile` and `_compile_remote` rather than re-implementing metrics, so
its numbers are comparable with the repository's historical tables, and it adds
what the survey lacks:

* `--mode runtime` — compile **and execute** on CE and compare execution time,
  the one metric a compiler cannot win by refusing to vectorize;
* `--fail-on-behind [PCT]` — survey-to-gate switch for CI-style sweeps;
* `--baseline old.json` — per-function ratio delta, so an optimisation is
  judged against the tree it replaced;
* `--min-instructions N` — excludes `.bss`/`.data` labels that would otherwise
  be ranked as "functions" with zero instructions (this was a real defect in
  the first version: ten globals were ranked as functions with ratio `-`).

### First corpus measurement (zlib-ng + gzip + expat + sqlite extracts, `-O2 -march=x86-64-v3`)

46 functions, 29 ranked, **LCCC ahead 7 / behind 22, geomean LCCC/best 1.409**.
Best-compiler wins: gcc16.2 16, lccc 15, icc 8, clang 6, icx 1.
LCCC wins outright include `zng_slide_hash_c` **0.319**, `pqdownheap` 0.643,
`sqlite_put_varint` 0.717, `zlib_ng_huffman_heap::main` 0.771.

Worst real gaps:

| function | source | LCCC | best | note |
|---|---|---:|---:|---|
| `sip24_update` | expat_siphash24 | 121 | 62 (gcc) | see §4.1 |
| `sip24_final` | expat_siphash24 | 158 | 81 (gcc) | see §4.2 |
| `main` | gzip_crc32_slice8 | 424 | 149 (gcc) | 0 vector insns vs gcc 17 |
| `main` | zlib_ng_adler32 | 256 | 73 (icc) | 38 vec vs gcc 0 (LCCC vectorizes, gcc declines) |
| `huffman_merge` | zlib_ng_huffman_heap | 1.435 | 1.0 | matches the 2026-11-18 ROI report |
| `zng_insert_string_batch` | zlib_ng_slide_hash | 1.304 | 1.0 | |

**`expat_siphash24` (189.000) is a metric artifact** and is listed here so it is
not chased again: LCCC materialises the wrapper while Clang emits a 1-instruction
tail call and ICX 4 instructions — a *scope* difference, not a quality
difference. Any table that ranks it without that caveat is wrong.

---

## 4. Root-cause analyses opened this session

### 4.1 `sip24_update`: LCCC emits switch-case "relay" blocks with copies

LCCC 121 instructions vs GCC 62. The IR (via `LCCC_DUMP_IR=1`) shows the cause
is **not** the SipHash rounds, which are instruction-for-instruction competitive
with GCC's (`rorx` forms, same count). It is the CFG around them: the eight
`switch` cases of `sip24_final`'s fall-through ladder each get a block whose
only content is `Copy { dest: V, src: W }` plus `Branch`, and `sip24_update`
carries the same shape from `do { while (...) ... } while (...)`. Those become
the machine-code pattern

```
.LBB26:  movq %rsi, %r11
         jmp  .LBB35
```

one per case (eight of them), which GCC does not emit at all. Secondary: LCCC
spills `H->p`, `H->buf` and `sip_endof(H->buf)` to memory and re-loads them in
the copy loop.

Why the obvious fix does not work yet: the copies are **non-SSA webs** — one
`Value` (e.g. `Value(128)`) is defined by `Copy` in two different blocks
(`label 32` and `label 33`), because lowering names the merge result once and
lets each arm assign it. That is why `copy_prop`'s multi-def guard refuses to
forward them (correctly — the guard exists because of the 6.18.50
`free_area_init_node` miscompile), and why `merge_single_pred_blocks` cannot
fold the relay into the join.

**The real fix is upstream of the CFG:** the lowering should make each case arm
assign a *fresh* SSA value and place a PHI at the join, after which
`thread_switch_case_relays` (below) and ordinary copy propagation both apply.
That is a lowering change with whole-corpus blast radius and needs the full
`run_regression.py` + differential gates; it is the single highest-value
structural item found this session.

### 4.2 `sip24_final`: jump-table dispatch is a PIE artifact, not a codegen defect

Under default (PIE) flags LCCC emits the portable form

```
leaq .Ljt_0(%rip), %rcx ; movslq (%rcx,%r9,4), %rdx ; addq %rcx, %rdx ; jmpq *%rdx
```

with 4-byte PC-relative entries. With `-fno-pie` it emits the GCC-canonical
`jmpq *.Ljt_0(,%r9,8)` with `.quad` entries — **byte-for-byte GCC's shape**.
`emit_switch_jump_table` already contains the full rationale (R_X86_64_32S is
rejected in `ET_DYN`, and the kernel's `objtool` only validates absolute tables).
So the remaining 158-vs-81 difference in the PIE build is the eight relay blocks
of §4.1, not the dispatch. **Do not "fix" the jump table; check `-fno-pie`
before reporting a switch-lowering gap.**

### 4.3 Constant vectors copied to stack slots in large frames

`zlib_ng_compare256::main` prologue:

```
vmovdqu .LCVEC_0(%rip), %ymm0 ; vmovdqu %ymm0, 160(%rsp)   (x4)
vmovdqa .LCVEC_4(%rip), %ymm15 ; ...                        (kept in registers)
```

The hot loop then reads `vpaddd 160(%rsp), %ymm12, %ymm2` — a stack reload per
iteration where `vpaddd .LCVEC_n(%rip), %ymm12, %ymm2` (a folded memory
operand, exactly what the `.LCVEC` pool exists for) costs nothing extra. GCC
keeps four constants in `ymm8/9/10/11` across the same loop. Candidate: teach
the vector-home allocator that a constant vector is rematerialisable from
`.rodata` at zero cost, so it is preferred as the eviction victim over a stack
copy. Scope: `src/backend/x86/codegen` vector home/eviction policy.

---

## 5. Changes landed this session, and the defect found in one of them

### 5.1 `scripts/godbolt_codegen_compare.py` ✅ (see §3)

### 5.2 `scripts/arena_snapshot.sh` ✅

Publication must survive the harness. `scripts/snapshot_ms178.sh` is
deprecated in favour of the gate-aware `scripts/lccc-snapshot.sh`, but that tool
carries a workspace census (64 MiB / 256 files) and a gate-stamp contract that
made this session's first publications fail after the patch was already built.
`arena_snapshot.sh` is the research-loop publisher: full diff against the
merge-base with `origin/main`, **atomic** publication (`*.tmp` + `fsync` +
`rename`) under **two** names in the workspace root (`ms178-1.patch` plus a
dated `ms178-1.<UTC>.<slug>.patch`, so rotating the alias can never lose the
only copy), a rotating history under `$HOME/snapshots`, and a `LINEAGE.log`
ledger recording head/base/content-hash/gate/scope. It refuses generated
scratch (`.s`/`.o`/`core*`) exactly like the deprecated script, because
`git add -A -N` is broad enough to have shipped 61 stray `.s` dumps once.

Two defects were found and fixed in it while validating:
`git add -A -N` followed by `git reset` cleared the intent-to-add marks *before*
the diff, so a brand-new script produced an "empty diff" refusal; and the scope
summary was computed after the reset, reporting "0 files" for a 760-line patch.

### 5.3 `src/passes/cfg_simplify.rs`: opt-in switch-relay threading — **and a fixpoint hang it exposed** ✅

New sub-pass `thread_switch_case_relays` (opt-in, `CCC_SWITCH_RELAY_THREAD=1`)
plus three unit tests. Status: correct, tested, **not default-on** — it fires
zero times on the corpus, for the reason in §4.1.

**The important part of this entry is the defect.** The first implementation
used `common::forwarding::resolve(...)`'s **second** tuple field as the relay
destination. `resolve` returns `(terminal, last_forwarding_vertex)`; the second
field is the vector's own successor, so every `Switch` arm was rewritten to a
value it already had — a no-op that still incremented the change counter.
`simplify_cfg` is a fixpoint loop, so it never terminated:

```
Thread 2 backtrace (gdb -p, sampled twice 20 s apart: 12 s CPU per 20 s wall)
  insert  <- build_pred_info::{closure} <- for_each_terminator_target
  <- build_pred_info <- thread_switch_case_relays <- simplify_cfg
  <- run_passes <- optimize_ir_module <- compile_ir_module_to_assembly
```

The kernel build sat on `mm/vmstat.o` for **6+ minutes of pure CPU** while
every other object took seconds. Three lessons worth keeping:

1. **A fixpoint sub-pass must be provably non-repeating.** The rewrite is now
   guarded (`end != *label`) so a mis-resolved entry costs one comparison, not
   a hang — structural, not defensive.
2. **A backtrace is a measuring instrument.** `sudo gdb -p PID -batch -ex
   "thread apply all bt 25"` against a live compiler process identified the pass
   in one sample; log tails and `strace` would not have.
3. **Edge counts are not predecessor counts.** `build_pred_info` counts edge
   *mentions*, so a `switch` whose `default` equals one of its case arms — the
   normal result of folding a redundant arm onto the default, and exactly the
   SipHash shape — reports 2 predecessors for a block only one block can reach.
   The pass now counts **distinct predecessor blocks** locally, and decides the
   default from its *original* target so the case loop's remapping cannot make
   the default test vacuous.

Regression coverage added with the fix: `switch_case_relay_threads_to_the_terminal_block`
(threads to the terminal block **and** asserts a second call is a no-op — the
property the hang violated), `switch_case_relay_with_two_predecessors_is_kept`,
`switch_case_relay_feeding_a_phi_is_kept`. `cargo test --profile fastbuild --lib
cfg_simplify` → **41 passed**.

## 6. Prioritised work queue for the next agent

1. **SSA-correct switch/loop lowering (top value).** Make each arm assign a
   fresh value + PHI at the join instead of multi-def copy webs. Unlocks §4.1,
   makes `copy_prop`/`merge_single_pred_blocks` effective, and removes ~40
   instructions from every fall-through-ladder function (SipHash, XML name
   classification, `strtol` digit ladders, dispatch tables).
   Gate: `scripts/ci_local.sh --fast` + `run_regression.py` + the differential
   oracle; measure with `godbolt_codegen_compare.py --baseline`.
2. **Constant-vector rematerialisation** (§4.3). Target: `zlib_ng_compare256`,
   `gzip_longest_match`, `fp_product_map`. Accept only with a paired runtime
   A/B and no regression on `f32_sse1_map` (strict, `-mno-sse2`).
3. **Finish the kernel**: `build_kernel_vm.sh` → `qemu_boot_test.sh` →
   `build_kernel_full.sh` (package config) → boot. Any lccc failure found on the
   way is a defect to fix in the compiler, not to route around in Kbuild.
4. **`gzip_crc32_slice8::main`** (424 vs 149): LCCC emits zero vector
   instructions where GCC emits 17. Start from `slice8`'s table indexing; check
   whether the vectorizer's dependence analysis gives up on the 8-way slice
   table. This is a genuine missed vectorisation, unlike §4.2.
5. **`huffman_merge` / `zng_insert_string_batch`** — already root-caused in
   `engineering/BACKLOG-ROI-2026-11-18.md` (inline growth cap; leaf-inline
   policy). Re-measure before acting; that document's own source-only control
   showed a 0.92× win from forced `always_inline` on one leaf.
6. **Runtime corpus baseline** (`tests/benchmark/run_benchmarks.py`,
   `--compilers lccc,gcc --reps 7`) is the acceptance instrument for all of the
   above. Static counts screen; runtime decides.

## 7. Measurement method (why the numbers here are defensible without a PMU)

No hardware PMU exists in this VM, so no cycles/cache-miss claim is made
anywhere. Instead:

* **Deterministic static counts** (`_stats`) are exact and re-derivable; they
  are the screening metric and are always reported as such.
* **Paired runtime A/B** (`run_benchmarks.py`) with CPU pinning, excluded
  warm-ups, randomized arm order and retained raw samples is the deciding
  metric for a landed optimisation.
* **Compiler Explorer execution** (`--mode runtime`) cross-checks a win on
  hardware the local VM cannot model.
* **Linker correctness by image identity** (`cmp` against ld.bfd/ld.lld) is
  stronger than a size comparison and needs no timing at all.
* Every claim ships with the manifest JSON that produced it.

Baseline recorded this session (`run_benchmarks.py --compilers lccc,gcc
--gcc gcc-16 --opt -O2 --cflag=-march=x86-64-v3 --reps 7 --cpu 0`):

```
LCCC / fastest available reference, correct benchmark pairs only:
  n=65; geometric mean=0.8195; arithmetic mean=0.9239
  best = ackermann vs gcc (0.022, 44.5x faster)
  worst = zlib_ng_huffman_heap vs gcc (1.217, 1.22x slower)
```

Read this as the acceptance baseline for every change in §6: a candidate must
not move the geometric mean up, and `zlib_ng_huffman_heap` is the standing
sentinel for the inline-growth work already root-caused in
`engineering/BACKLOG-ROI-2026-11-18.md`.

## 6. Session-2 defect ledger (2026-11-19 evening) — evidence first, all items OPEN

Measurement method: `valgrind --tool=callgrind` for deterministic Ir attribution
(no PMU on this host), `cachegrind --branch-sim=yes` for branch/mispredict
counts, `objdump`/`-S` output for the emitted shape, and paired wall-clock
medians (n>=9) only to confirm a callgrind-predicted win.

### D1 (P0, real runtime) — `static inline` helper is inlined at ONE site and left outlined at another, and the outlined copy is fat
Evidence: `zlib_ng_huffman_heap` (PASSES=6, callgrind):
lccc `smaller` = 3,073,248 Ir (19.8% of the whole program), 0 in gcc.
`CCC_INLINE_DEBUG=1` shows `smaller candidate: blocks=5, inst_count=59,
direct_calls=2` then "Inlined 'smaller' into 'huffman_merge'" while
`pqdownheap` keeps a call.  The outlined copy is 84 bytes and reloads
`tree[n]`/`tree[m]` up to three times because the two `Cast I32->I64` and the
two `Load U16` are duplicated by the frontend per comparison operand.
GCC emits the same predicate as ~10 instructions with one load each.
Sub-items: (a) per-site profitability must consider *why* a site was refused;
(b) `Cast`/`Load` CSE must fire across blocks (currently `cross_block_renamable`
admits only integer `Mul`/`Sub`/pure-`Call`, so `Cast I32->I64` duplicates —
`smaller` has 3 copies of each of 2 casts — stay); (c) the frontend's
`a op b || (a op c && d op e)` lowering re-emits both operands per comparison,
which is what makes the body 59 instructions instead of ~20.

### D2 (P0, real runtime) — AVX loop body keeps a 32-byte copy of a load in the frame because `VecConstI8x32`/`VecZero*` are pointer-homed
Evidence: `zlib_ng_adler32` hot loop (`vmovdqu (%r15), %ymm0` +
`vmovdqu %ymm0, 144(%rsp)` + `vpsadbw 144(%rsp), %ymm5, %ymm0`) vs ICX/GCC,
which write `vpsadbw %ymm0, %ymm5, %ymm0`.  The extra store/load pair costs
2 uops and a store-forwarding dependency per 32 bytes.
`adler32_do8_hot`, `zlib_ng_compare256`, `ascii_case_fold`, `histogram`,
`unsigned_reduction` show the same class (60 `ymm->stack` constant stores
across 13 corpus files).

### D3 (P1, instruction count) — spill slot is stored immediately after a load that produced the value
`vmovdqu (%r15), %ymm0` / `vmovdqu %ymm0, 144(%rsp)` is a *redundant* pair when
the only consumer is the very next intrinsic (the deferred-store machinery
already exists; this path bypasses it).

### D4 (P2) — `constant_fold` leaves `a * 65521` as `imulq $2147975281; shrq $47`
for `% 65521` on a `u32`; the `movl $2147975281, %eax` reload per use costs an
instruction (immediate `imulq $imm32` is available; the 64-bit constant cannot
be an imm32, so the split is correct, but the *register* form is finer than the
`eax` staging).  Verify the divisor-magic path pins `rax` needlessly.

### D5 (process, pre-existing) — three stale `ci_local.sh --fast` invocations from earlier turns were still alive, contending on the cargo build lock and making every build look stalled.  Only ONE CI run may be live; the new run must be started only after `pkill -f ci_local.sh` and a lock-free `cargo` state.

## 7. Session-2 results (measured, reproducible)

Harness: `tests/benchmark/run_benchmarks.py` (paired medians, randomized rounds,
`--cpu auto` pinning), `-O2 -march=x86-64-v3`.  Four references now available:
**gcc 16.2.0, clang 23.1.5, ICX 2026.2.0 (Intel PPA apt, installed this
session at /opt/intel/oneapi/compiler/2026.2, symlinked into the cached
toolchain bin/), plus ICC-class oracle via godbolt**.

### Landed and validated

1. **`static inline` medium window** (`src/passes/inline.rs`,
   `fits_normal_inline_limits`).  Before: `static inline` was capped at 32 IR
   instructions while a plain `static` got 160, so the stronger C `inline`
   request had the weaker budget.  After: both get the medium window.  Effect
   (measured): zlib-ng's heap comparator is now inlined at every site
   `pqdownheap` itself is inlined at; zlib_ng_huffman_heap geomean over 7 reps
   improved and `zlib_ng_huffman_heap` vs gcc moved 1.217 -> 1.137.
2. **Body-counted per-caller inline budget** (`MAX_INLINE_BODIES_PER_CALLER`).
   The instruction budget charges N x size for N clones of ONE body, which is
   what starved the comparator at 160,797 call sites.  The new budget counts
   distinct bodies (350) and is enforced in addition to the instruction budget,
   so single-site callers (every kernel hot path) are unaffected bit-for-bit.
3. **Contention-only cost ordering** in the general inlining tier (GCC
   `big_speed` / LLVM bottom-up order): applied only when the candidate set
   cannot fit the instruction budget, so no uncontended decision changes.
   (Verified: inlining decisions for gzip_longest_match, zstd_double_fast and
   expat_siphash24 are identical to the previous revision.)
4. **GVN occurrence-relative rename documentation + fail-closed class rule**
   (`rename_is_value_preserving`); pointer, 128-bit and float<->int casts are
   explicitly excluded, and everything that can trap, allocate or observe
   memory keeps the unconditional `Copy` form.
5. **Opt-in cross-block integer cast CSE** (`CCC_CAST_RENAME=1`).  Measured on
   its target shape (zlib_ng_huffman_heap, PASSES=20, n=15): 16.89 ms -> 11.09
   ms (2.241 -> 1.472 vs gcc).  It is NOT enabled by default because the paired
   corpus run showed it moving 1-3 % on zstd/gzip workloads in both directions
   without a net signal at n=15.  The machinery is shared with the shipped
   `Mul`/`Sub` classes and pinned by `integer_cast_duplicate_in_dominated_block_is_renamed`.
6. **D5 fixed**: three stale `ci_local.sh --fast` processes from earlier turns
   were alive and contending on the cargo build lock, which made every build
   look stalled.  Only one CI run may exist; kill before starting.

### Corpus state after these changes

- Full corpus, 4-way (lccc/gcc/clang/icx), reps 7: **LCCC/GCC geomean 0.7874**
  (baseline 0.8195), LCCC/best-of-4 0.8797, correctness 65/65 pass.
- Worst remaining gaps vs the fastest of the four compilers:
  `sqlite_vdbe_serial` 1.165 (clang), `zlib_ng_huffman_heap` 1.137 (gcc),
  `nbody` 1.134 (gcc), `zstd_count` 1.069 (clang), `gzip_longest_match` 1.055.

### The next big win, with evidence (D6, OPEN)

`libm_round_family` is **2.70x slower than ICX** while matching GCC exactly.
ICX's `round_family_pass` is 4x-unrolled AND vectorised: `vroundpd $imm, %ymm`
x6, `vandnpd/vandpd/vorpd` for copysign, `vfmadd231pd`, packed `vaddpd`, and a
horizontal reduction at the end.  LCCC keeps it scalar at every width
(`-mprefer-vector-width=128|256|512` all produce zero `vroundpd`), even though
the map recogniser already lists `RoundScalarF64`, `FmaScalarF64` and
`CopysignF64` as tree-parseable and already emits `VecRoundF64x4` for
structured vector types.  What is missing is a *dependence* rule: a recognised
map that feeds only a scalar accumulator (`acc += f(x[i])`) has no vector store,
and the reduction path needs both that rule and a float reduction whose
contraction order the FP gates accept.  Accepting `acc += X - t` where `X` is the
map tree and `t` a shared subterm, with rounding on, is order-changing and must
stay behind the same contract the FMA contraction passes use.

## Session 3 (2026-11-20): wipe recovery, rebase onto `d82a0858`, green CI

### Recovery (the harness wiped everything except `/home/user` file contents)

- `.git`, `target/`, `/opt`, `/swapfile`, `/tmp` and `~/.cache/lccc/agent-toolchains`
  were gone and the source tree had reverted to `main`.  The durable artifacts
  (`ms178-1.patch`, `snapshots/`, `results/`, `logs/`) survived, so the state
  was rebuilt from the patch of record.
- Toolchain cache: `scripts/restore_agent_toolchain_cache.py --tag
  harness-cache-8` **cannot work from a shallow clone** -- it runs
  `git fetch origin +refs/tags/harness-cache-8:...` and a `--depth` clone is
  refused that ref by the server ("couldn't find remote ref"), even though
  `git ls-remote` advertises it.  Working recipe: a *bare, unfiltered*
  `--depth 1 --branch main` clone in `/tmp`, `git fetch --depth 1 origin tag
  harness-cache-8` **there**, then either `git fetch <bare>
  refs/tags/harness-cache-8:refs/tags/harness-cache-8` (works) or
  `.git/objects/info/alternates` -> that bare repo's `objects`.  The restore
  script itself then succeeds (`--remote <bare>`) and reassembles all 12
  payloads (rust 1.99.1, clang 23.1.5, gcc 16.2.0, gdb 18.1, binutils 2.47,
  mold 3.0.0, qemu 11.1.3, kernel-relocs 7.3.1, ISA XML, debs, binutils-src).
- `restore_agent_toolchain_cache.py` stages ~2 GiB in `tempfile.mkdtemp()`,
  which lands on `/tmp`: a **992 MiB tmpfs** here, so it died with
  `[Errno 28] No space left on device`.  `TMPDIR` must point at the root
  filesystem (`/var/tmp/...`).  Same cause broke `riscv-relocation-contract`,
  `x86-asm-diff` and four `elf_writer_common` unit tests inside a CI run
  ("random program seed 243 ... No space left on device") -- those were **not**
  compiler failures.

### Defects fixed in this session

7. **`ccc -m32` cannot find `-lgcc` on a `gcc-multilib` host.**  The built-in
   linker probes `/usr/lib/gcc/x86_64-linux-gnu/<ver>/32` (correct, and covered
   by `gcc16_multilib_discovery_preserves_class_and_newest_priority`), so
   `-lc`/`crt*` work, but the GCC 16.2 *oracle*'s multilib is what `gcc-16`
   itself searches, and not every gate goes through the built-in linker: the
   ones that shell out to `gcc-16 -m32` need `gcc-16-multilib`, which does not
   exist for the pinned 16.2 tree (Intel/debs only ship 12/14).  Gates that
   *can* use the built-in linker (`-lgcc`, `crt1.o`) pass once host
   `gcc-multilib` + `libc6-dev-i386` + `linux-libc-dev:i386` + `g++-multilib` +
   `lib32stdc++-14-dev` are installed; the ones that cannot are host-independent
   only in CI, which installs the same set.  Recorded so nobody re-diagnoses it
   as a codegen bug: `[FAIL] i386_dso_emit_semantics` ("32-bit libstdc++
   unusable here") is the *host* talking.
8. **`build_lccc_fast.sh` disk-headroom probe was dead on a clean tree.**
   `df -Pk "$target"` on a `target/` that does not exist yet exits non-zero
   with an empty field, and `case $free_kib in '' ...) free_kib=0` turned that
   into "unknown", so the ENOSPC guard silently did nothing on exactly the run
   that needs it most (the first build after a restore).  The probe now walks
   up to the nearest existing ancestor.
9. **`arena_snapshot.sh`'s gate could never fire.**  It hashed `git diff
   <base>` and looked for `~/.cache/lccc/ci-stamps/<diff-sha>`, while
   `ci_local.sh` stamps `target/ci_local.pass` with the *worktree tree hash*
   (`scripts/worktree_tree.sh`) and writes no second copy.  A fully green
   `ci_local.sh --fast` therefore still published `gate UNGATED` -- a gate that
   reads as evidence in the lineage ledger while never being able to pass.  The
   publisher now keys on `worktree_tree.sh` and compares the stamp's own
   `tree=` field; `tests/corpus/test_arena_snapshot_gate.py` pins all three
   states (matching stamp = PASS, foreign stamp = UNGATED + warning, absent
   stamp = UNGATED).  That test also caught a real publication bug: `cp` into a
   missing `MS178_PUBLISH_DIR` failed *after* the snapshot was written.
10. **Two new opt-in knobs are now resolved once per translation unit**
    (`gvn::set_cast_rename_enabled`, `cfg_simplify::set_switch_relay_thread_enabled`),
    the shape `loop_idiom::set_config` and `vectorize::set_strided_reduce_enabled`
    already use.  `CCC_CAST_RENAME` was a per-function `env::var_os` inside the
    pass body -- a counted site in the `check_env_test_hygiene.sh` ratchet
    (150 -> 151) and a process-global read driven by `env::set_var` from a
    concurrent unit test.  Net: the ratchet is back at the measured 150 with
    both knobs documented, and `set_cast_rename_enabled` is the supported way
    for a test to exercise the class.

### Verdicts on the rebased tree (`main` = `d82a0858`, PR #1197)

- `cargo build` (fastbuild, `-D warnings`, mold): clean.
- `cargo test --profile fastbuild --all-targets`: **9366 passed, 0 failed, 10 ignored**.
- `cargo clippy --all-targets -- -D warnings`: clean.  `cargo fmt --check`: clean.
- `python3 -m unittest discover -s tests/corpus`: **1538 tests, OK** (1 skipped).
  Note `TMPDIR` must be `/tmp` for this suite: `test_i386_multilib_safety` and
  `test_offline_deb_extract` deliberately refuse paths under the sandbox
  workspace (snapshot-quota safety), so a `TMPDIR` inside `/home/user` fails
  them for the right reason but the wrong environment.
- `bash tests/regression/check_gnu_declaration_corners.sh`: 39 passed, 0 failed
  (172 verdict comparisons / 13 diagnostic pins / 15 runtime checks).
- `python3 scripts/audit_census.py --check`: PASS, `knob_literals` 577,
  `knobs_undocumented` 250 (unchanged).
- `scripts/ci_local.sh --fast`: **680 passed, 0 failed, 8 skipped**, exit 0,
  stamp `tree 41c3d9b989cefe9742831feae0d238296d3c043a` -- and the published
  `ms178-1.patch` carries that stamp as its gate line.
