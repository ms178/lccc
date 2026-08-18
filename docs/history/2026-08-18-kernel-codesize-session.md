# 2026-08-18 — Kernel boot-code correctness + size session

**Base:** `origin/main` @ `1abc5f1` (post-PR #107)
**Branch:** `arena/kernel-codesize` (snapshot: `/home/user/ms178-1.patch`, regenerate with `git format-patch origin/main --stdout`)
**Mission:** compile+boot the user's patched linux-cachymod-6.18.44 with lccc; kill the 32K setup.bin blocker. **Linker work is delegated to another agent — do not touch `src/backend/*/linker` or `lccc-ld`.**

## Environment protocol (harness wipes EVERYTHING between sessions)

1. `sudo fallocate -l 10G /swapfile && sudo chmod 600 /swapfile && sudo mkswap /swapfile && sudo swapon /swapfile` — **mandatory first step**.
2. rustup (stable, minimal), then `cargo build --profile fastbuild --locked -j2`.
3. `apt-get install bison flex bc cpio libelf-dev kmod qemu-system-x86 busybox-static gcc-multilib libc6-dev-i386`.
4. Kernel tree: linux-6.18.44 + all 26 patches from `ms178/archpkgbuilds/packages/linux-cachymod-6.18` in PKGBUILD order (0000-rt … 1020-r8169). All apply cleanly with `patch -Np1`.
5. `make defconfig` (HOSTCC=gcc), then `scripts/config -e PAGE_POOL` is NOT enough — the r8169 godlike patch needs `select PAGE_POOL` in `drivers/net/ethernet/realtek/Kconfig` under `config R8169` (missing Kconfig dep in the patch; fix applied locally, should go upstream into `1020-r8169-rtl8125-multi-queue-godlike.patch`).
6. Workspace snapshots CAP at ~10k files: the kernel tree and `.git` do NOT survive. Only `/home/user/ms178-1.patch` and small files survive. **Commit + format-patch after every validated step.**

## Fixed this session (each commit validated: 780/780 unit, 50/50 correctness, regparm ABI suite at -O0/-O1/-O2/-Os)

1. **`-mregparm=3` was a caller/callee ABI split miscompile** (blocking ALL realmode code): callee read register params from the caller's stack. Fixed end-to-end: gcc_regparm_mode classification core (variadic=all-stack, i64 GP pairs, overflow kills, FP-scalars skip GP, aggregates ceil(size/4)), two-phase prologue capture (memory stores first, then parallel-move register captures with xchg cycle break), sret-in-%eax (no `ret $4` under regparm), indirect calls through value homes (never stage target in %eax).
2. **Unsound ESP-offset never-read-store peephole**: compared raw N(%esp) offsets across ESP adjustments; now normalizes to frame-anchored coordinates via a linear sp_down walk (calls/irregular writes degrade conservatively).
3. **Blanket inline-asm callee-saved clobber** replaced by per-block scratch-demand proof against the %ecx/%edx pool prefix (arch/x86/boot's `"=rm"` segment reads cost 3 push/pop pairs each before).
4. **GlobalAddr+Load/Store absolute fold** for non-PIC i686 (`movl sym,%eax` instead of `movl $sym,%edx; movl (%edx),%eax`); immediate stores fold to `movl $imm,sym`.
5. **-mpreferred-stack-boundary=2/3 honored** in i686 frame rounding.
6. **never_materialized fold preview**: folded GlobalAddr values excluded from regalloc AND slot assignment (dead addresses were stealing the clean caller-saved register and 8-byte slots).
7. **Peephole: zero-idiom fusion** (`xorl %eax,%eax; movl %eax,R` → `xorl R,R`) and **copy-then-test collapse** (`movl S,%eax; testl S,%eax` → `testl S,S`).

## Measurements (realmode corpus = 19 arch/x86/boot C files, REALMODE_CFLAGS, text bytes)

| checkpoint | lccc | gcc 14.2 | ratio |
|---|---|---|---|
| session start (after ABI fix) | 42175 | 12999 | 3.24 |
| + asm-demand scan | 40737 | | 3.13 |
| + global fold | 38738 | | 2.98 |
| + boundary + never-mat + peepholes | 38259 | | 2.94 |
| + direct-to-dest ALU | 38083 | | 2.92 |
| + slot RMW collapse | **37784** | | **2.90** |

Setup.bin limit is 32768 total (includes ~2.5K of .S objects + tables); C text budget is roughly ≤26K → **need ~1.55× more shrink** (38259 → ~25000).

## Next levers (ranked, with evidence)

1. **Accumulator-centric ISel** (biggest; ~30-40% of remaining gap): codegen computes everything via %eax then moves to the allocated register (`movl %ebx,%eax; incl %eax; movl %eax,%ebx` instead of `incl %ebx`). Needs BinOp/Load results emitted directly into dest_reg when one exists. Look at `emit_binop` in i686/codegen/alu.rs; the x86-64 backend has partial support (`emit_float_binop_into_reg` pattern).
2. **Redundant GEP recomputation**: `movl %ebx,%ecx; addl $40,%ecx` emitted twice for the same address in regs.c (no GVN/CSE at the addressing level; gep_fold rejects multi-use GEPs). Extend gep_fold to multi-use when base is register-resident, or CSE identical GEPs per block.
3. **push/pop churn**: 100+ pushl/popl quads across corpus; many functions use 4 callee-saved where GCC uses 0-1. Partly follows from #1 (fewer register-resident values needed if ops go direct-to-memory/dest).
4. **32-bit int ops through wide paths**: cpucheck/printf still 3.3-4×; look at i128_ops paths being taken for 32-bit code.
5. **`movl %eax,%edx; movl $16,%eax; call intcall` pattern** (36×): argument staging order for regparm reverses; loading %edx first then %eax would avoid the shuffle. See emit_call_reg_args ordering.

## Kernel build status

- vmlinux links (GNU ld; lccc-ld delegated to linker agent), all C objects compile with lccc-x86.
- One vmlinux-level blocker found+fixed: missing PAGE_POOL Kconfig select (patch bug, see above).
- bzImage: all C objects compile; setup.elf gate still fails (`ld: Setup too big!`); setup .o total = 45210 bytes vs 32768 limit (12.4K over). vmlinux itself links after the PAGE_POOL Kconfig fix.
- (old note) bzImage build in progress at session end (setup.elf assert `_end <= 0x8000` is the gate; current C-text estimate says STILL TOO BIG — expect "Setup too big!" until levers 1-2 land).
- Boot test plan: qemu-system-x86_64 -kernel bzImage -initrd busybox initramfs -append "console=ttyS0 rdinit=/init" -nographic; busybox-static is installed.

## Oracle notes

- godbolt.py works for GCC/Clang/ICX comparisons (scripts/godbolt.py, one compiler id per call). For local GCC 14.2 comparisons use the REALMODE_CFLAGS recipe in this doc (`/tmp/rmflags.sh` pattern).
- The regparm probe corpus (p1..p6, probe2-5) in this session's history is the authoritative GCC-semantics reference for i386 regparm; the conformance test is `tests/regression/regparm3_abi_conformance.c`.
