# Session 86 — linux-cachymod 6.18.46: 32 KiB gate root-caused, GCC baseline established, two i686 size optimizations

Date: 2026-08-27
Upstream base at session start: `e3327e39a2a534fe9a5235afcd7833f348c371ef` (lccc main, origin HEAD)
Session commit: `dcd1d4f7` (snapshot `S01-s05-divfusion-immstore`, patch APPLIES-CLEAN)
Compiler build: `scripts/build_lccc_fast.sh` (fastbuild profile, Rust 1.98.0 `-O1`, `-j2`, gcc+GNU ld fallback — clang/mold unavailable in this sandbox)
Kernel tree: linux-6.18.46 via `prepare_kernel_tree.sh` (26/26 CachyMod patches, package config)

## Objective (user task)

Compile+link the custom Linux kernel with lccc/lccc-ld; the 32 KiB boot-code
size limit is the named blocker — build the kernel's boot code to verify; fix
all compiler/linker issues encountered; keep ms178-1.patch autosaved with all
validated optimizations.

## Headline results

1. **The 32 KiB gate FAILS on 6.18.46 by 2,512 bytes** (`_end=35,280` vs
   `0x8000`), and the regression is fully root-caused: the GAS-faithful -m16
   correctness fixes in `e12597c7` (landed post-session-83) grew `.text` by
   ~1.5 KiB. The gate had not been re-run when those fixes merged. Session 82's
   PASS used the old, incorrect (smaller) encodings on 6.18.44 — it was never
   a real margin.
2. **GCC reference established on the identical tree/flags:**
   `.text` 12,677 / `_end` 22,880. lccc: `.text` 24,716 — **1.95× GCC**. The
   gate is an lccc codegen-quality gap, precisely localized (below).
3. Two validated i686 size optimizations landed (constant→slot immediate
   store fold; div/rem pair fusion — `number()` now matches GCC's one-`divl`
   shape). `cargo test` 1205/0, regression suite 467/0, runtime div torture
   PASS at all levels. Corpus `.text` 24,756 → 24,716.

## Evidence chain (all reproducible from this tree)

* Bisect: `ee8b5c1c` PASS (23,065) → `be227056` PASS (23,065) → `6e2ee1c4`
  PASS (23,166) → `2fe667a7` **FAIL (24,625)**. `e12597c7`/`2d20f594` do not
  compile standalone (`no field no_sse on CodegenState`); `2fe667a7` is the
  first buildable descendant carrying the boot-path changes.
* Kernel 6.18.44 → 6.18.46 source delta contributes ZERO for lccc
  (`ee8b5c1c` produces byte-identical `.text`=23,065 on the 6.18.46 tree).
* Per-function excess vs GCC: 39 functions >30 B totalling +5,371 B (top:
  vsprintf +769, number +699, check_cpu +366, __cmdline_find_option_bool
  +354, vesa_probe +307, get_cpuflags +294, __cmdline_find_option +292).
* Functions GCC inlines away entirely (rdfs8*, myisspace, simple_strtol, …):
  **+12,186 B** of standalone lccc bodies.
* `CCC_M16_FULL_PIPELINE=1` inlines rdfs8 correctly (`movb %fs:(%ecx),%cl`)
  but grows cmdline.o 571 → 713: under the current RA, inlined bodies spill
  more than the ~12-byte call they replace. The m16 no-inline policy is
  therefore CORRECT for today's RA — the fix order is RA first, then inlining.

## Root-cause ranking of the remaining 12 KiB (from disassembly)

1. RA: spilled loop-carried values — all 4 callee-saved regs park params;
   loop state round-trips the stack every iteration; the caller-saved
   allocator cannot fire across the helper calls the m16 policy keeps.
2. Inliner×RA coupling (above).
3. Byte-granular slots + movb immediates for char/short locals (GCC:
   `movb $48, 6(%esp)`; lccc: 4-byte slot + `movl`).
4. Micro-peepholes (~150–250 B): testb-fusion (13 sites), store-reload
   pairs (20), redundant zext (3).

## What did NOT work / honesty notes

* The gate did not pass this session. Two micro-optimizations recovered 40
  bytes of the 2,512-byte overflow; the remainder requires the RA work
  (§ attack plan in `updates/followup_2026-08-27_session05.md`).
* A suspected u64 miscompile on i686 was an ILP32 test bug (`unsigned long`
  is 32-bit; lccc was RIGHT). Recorded to prevent future false alarms.
* Swap could not be installed (container forbids swapon without
  CAP_SYS_ADMIN; no sudo). Memory discipline: -j2 + fastbuild only.
* 32-bit runtime validation required a death-signal harness (seccomp blocks
  int $0x80): `-m32 -static -nostdlib`, verdict encoded in
  SIGTRAP/SIGFPE/SIGSEGV/SIGILL.

## Validation

* `cargo test --lib`: 1205 passed / 0 failed / 6 ignored.
* `scripts/run_regression_suite.sh`: PASS=467 FAIL=0 SKIP=11 (AB-diff 0).
* Runtime div/mod torture: 100k random pairs, signed+unsigned, 32-bit and
  true-64-bit (`unsigned long long`), all of -O0/-O1/-O2/-Os/-Oz, fusion
  on/off — PASS.
* Boot corpus: `.text` 24,756 → 24,716; flat setup.bin oracle comparison
  skipped (ld.lld absent; ld.bfd oracle path present in the script).
