# LCCC Follow-up / Kontinuität — Session 05 (2026-08-27)

**Scope:** linux-cachymod **6.18.46** boot-code (32 KiB gate) bring-up on current
main; two validated i686 size optimizations; GCC reference baseline established;
complete root-cause decomposition of the remaining 2,512-byte gate overflow.

**Base:** `origin/main @ e3327e39` · **Session commit:** `dcd1d4f7`
**Patch:** `ms178-1.patch` (APPLIES-CLEAN, 20,884 bytes, snapshot `S01`)

---

## 1. Environment (differences vs. session 04 — READ before restoring)

| Item | Session 04 (Arena) | Session 05 (this sandbox) |
|---|---|---|
| RAM / swap | 1.9 GiB + 8 G swap | 4.1 GiB, **swap IMPOSSIBLE** (no CAP_SYS_ADMIN; `swapon` exits 255; no sudo) |
| Persisted root | `/home/user` | `/home/z/my-project` (workspace snapshot) + `/tmp/my-project` (PolarFS 29P); `/home/user` NOT creatable (`/home` root-owned) |
| clang/mold | installed | NOT installed, no root for apt → `build_lccc_fast.sh` auto-falls back to gcc+GNU ld (correct behaviour) |
| Extra tools | apt | extracted from .deb into `/home/z/.local-tools` (flex, bison, bc, cpio, zstd, libelf, libdw, qemu-user-static symlinks, libc6-i386) — source `/home/z/.local-tools/env.sh` (adds BISON_PKGDATADIR) |
| 32-bit exec | gcc-multilib | **native 32-bit syscalls blocked by seccomp** (`int $0x80` → SIGSYS). Static `-m32 -nostdlib` binaries exec, but any syscall dies. Runtime validation trick: encode verdict in the DEATH SIGNAL (int3=pass/SIGFPE/SIGSEGV/SIGILL) — see §4 |
| Rust | 1.98.0 | 1.98.0 (rustup `~/.cargo`) — `cargo test` needs `CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=gcc RUSTFLAGS=""` (repo `.cargo/config.toml` pins clang+mold) |

**Snapshot invocation in this sandbox:**
```bash
LCCC_REPO=/home/z/lccc-work/lccc \
LCCC_ARTIFACTS=/tmp/my-project/lccc-artifacts \
LCCC_DELIVERABLE=/home/z/my-project/ms178-1.patch \
  ./scripts/lccc-snapshot.sh "<slug>" "<desc>"
```
(`LCCC_DELIVERABLE` is new — the script's hard-coded `/home/user` path is now
overridable; this improvement is part of the patch.)

## 2. The 32 KiB boot gate on 6.18.46 — status: FAIL by 2,512 bytes (was PASS on 6.18.44)

**Regression root cause (fully bisected):** commit `e12597c7` ("Make the x86
boot path GAS-faithful", landed between `6e2ee1c4` and `2d20f594`) fixed REAL
-m16 correctness defects (`.code16gcc` 32-bit call/ret operands, `rep movsl`
prefix inversion, `leal` 0x66 override, absolute-address movs disp16+R_386_16).
Those correct encodings are 1 byte larger per affected instruction, growing
`.text` by ~1.5 KiB. Session 82's PASS was measured with the OLD, INCORRECT
encodings on 6.18.44. Nobody re-ran the gate after the GAS-faithful fixes —
`_end` went 31,184 → 35,280 on 6.18.46. Kernel sources 6.18.44→6.18.46
contribute **zero** size delta for lccc (verified: lccc@`ee8b5c1c` produces the
identical `.text`=23,065 on both trees).

Bisect ledger (all on the same prepared 6.18.46 tree, `OUT` isolated):
`ee8b5c1c` PASS(.text 23,065) · `be227056` PASS(23,065) · `6e2ee1c4` PASS(23,166)
· `2fe667a7` **FAIL(24,625)** — the regression rides in the merge that carries
`e12597c7` + the `no_sse` build fix (`2d20f594` and `e12597c7` alone do not
compile: `no field no_sse on CodegenState`; `2fe667a7` is the first buildable
descendant). HEAD: FAIL(24,756) → with this session's fixes: **24,716**.

**The GCC reference (same tree, same flags, ld.bfd):**
`.text` **12,677**, `_end` **22,880** — lccc emits **+12,039 bytes (1.95×)**.
The old "gate passes with 1,600 B headroom" state was NEVER a real win: it was
incorrect encodings + still 2× GCC. The gate is an lccc codegen-quality problem,
not an encoding accident.

## 3. What this session landed (validated)

### 3.1 i686 `emit_copy_value`: immediate→slot store fold
`Copy{src: Const, dest: spilled slot}` now emits `movl $imm, slot` instead of
`movl $imm, %eax; movl %eax, slot`. Wide dests excluded (store_eax_to zero-fills
their upper word). Acc register cache stays valid (eax untouched).
Reuses `direct_store_src` for immediate formatting (made `pub(super)`).

### 3.2 i686 peephole `fuse_div_rem_pairs` (Phase 3.6, `CCC_NO_DIV_FUSION=1` to disable)
Fuses a second identical `divl/idivl` (+staging) when the first division's
%eax/%edx results provably survive to the second site: straight-line window,
no eax/edx writes, no writes to staging-read registers/slots (byte/word stores
alias by RANGE), opaque multi-writers (div/mul/xchg/string/rep/…) fail closed.
Kernel `number()` digit loop now emits ONE `divl` with both quotient and
remainder consumed directly — GCC's exact shape. Runs after the local cleanup
(so the URem `movl %edx,%eax` extraction is already folded); fusion re-runs
copy-prop/DSE to clean the vacated staging.

### 3.3 i686 peephole `fuse_sext_testb` (Phase 2/3/3b, `CCC_NO_SEXT_TEST=1`)
`movsbl %al, %eax; testl %eax, %eax` → `testb %al, %al` when the extended
register is provably dead (bounded forward proof; pops count as restores).
All flags identical (sext preserves ZF/SF; CF/OF zero both forms).  Honest
scope: on the corpus the consuming conditional branch is a barrier and the
accumulator keeps %eax live, so it currently fires only on branch-free
consumers — the 6 remaining cmdline/printf sites need CFG liveness or
narrow-test selection at CondBranch emission (see §5/§6).

### 3.4 Validation (all green)
* `cargo test --lib`: **1205 passed / 0 failed / 6 ignored**
* `scripts/run_regression_suite.sh`: **PASS=467 FAIL=0 SKIP=11, AB-diff 0**
* Runtime div/mod torture (100k random pairs, signed+unsigned, 32-bit and
  true-64-bit, -O0..-Oz, fusion on/off): PASS — via the death-signal harness
  (`-m32 -static -nostdlib`, verdict in SIGTRAP/SIGFPE/SIGSEGV/SIGILL)
* setup corpus `.text`: 24,756 → **24,716** (sext-test fusion: no corpus
  change — see §3.3 scope note; char/short torture 200k iterations PASS)

## 4. ILP32 pitfall recorded for future sessions
A "u64 miscompile" scare was a TEST bug: `unsigned long` is **32-bit on i686**
(ILP32). lccc truncates 0xDEADBEEFCAFE1234ul correctly at the load. On i686 the
64-bit types are `unsigned long long`/`long long`. The kernel's `number()` arg
`unsigned long num` is likewise 32-bit — GCC emits one 32-bit `divl` for it,
and so does lccc now. Verified via `CCC_DUMP_IR` (Load ty: U32 is CORRECT).

## 5. Remaining 2,512 bytes — the evidence-backed attack plan

Per-function excess vs GCC (top of 39 functions >30 B; total 5,371 B):

| function | gcc | lccc | excess |
|---|---:|---:|---:|
| printf.c:vsprintf | 1121 | 1890 | +769 |
| printf.c:number | 600 | 1299 | +699 |
| cpucheck.c:check_cpu | 577 | 943 | +366 |
| cmdline.c:__cmdline_find_option_bool | 204 | 558 | +354 |
| video-vesa.c:vesa_probe | 388 | 695 | +307 |
| cpuflags.c:get_cpuflags | 296 | 590 | +294 |
| cmdline.c:__cmdline_find_option | 279 | 571 | +292 |
| … | | | |

**The big number:** functions GCC inlines away entirely (rdfs8*, myisspace,
simple_strtol, isxdigit, strchr, strstr, …) total **12,186 bytes** of standalone
lccc bodies. GCC's caller sizes already absorb the inlined copies and STILL sum
to half of lccc's .text.

Root causes ranked (all observed in disassembly, see worklog for examples):
1. **RA: spilled loop-carried values.** In `__cmdline_find_option`, all 4
   callee-saved regs hold the 3 params + 1 temp; the loop state (c, state, w)
   round-trips stack every iteration (reload→use→spill, 2 stores of one load).
   GCC keeps params in memory and registers the HOT loop values. The caller-
   saved allocator (`%ecx/%%edx`, hazard whitelist in `regalloc.rs`) exists but
   cannot fire across the `rdfs8`/`myisspace` CALLS the m16 policy leaves
   behind (calls clobber eax/ecx/edx).
2. **Inlining×RA coupling.** `CCC_M16_FULL_PIPELINE=1` DOES inline rdfs8
   (`movb %fs:(%ecx),%cl` — same as GCC) but grows cmdline.o 571→713: inlined
   bodies under the current RA spill MORE than the 12-byte call they replace.
   Session 82's "inlining is a size loss" measurement remains true — until the
   RA can keep loop values in caller-saved regs across the (now call-free)
   loop bodies. **This is the #1 priority: caller-saved allocation for
   loop-carried values once calls are gone.**
3. **Byte-granular stack slots / movb immediates.** GCC stores char locals as
   `movb $48, 6(%esp)`; lccc uses 4-byte slots + 32-bit stores everywhere
   (`store_eax_to` always `movl`). A value-width map for Copy dests does not
   exist (Copy carries no IR type — result_type() is None by design).
4. Micro-peepholes still available (~150-250 B total): testb-fusion
   (`mov[sz]bl %al,%eax; testl` → `testb %al,%al`, 13 sites), 20 store-reload
   pairs, 3 redundant-zext-after-movb sites.

**Do NOT chase:** disabling VIDEO/EDD/serial in the kernel config to fit the
gate (changes the user's requested functionality); reverting the GAS-faithful
encodings (they are correctness).

## 6. Unresolved / next-session priorities

1. **P0 — i686 caller-saved RA for call-free loop regions** (see §5.2): make
   `fuse`-style windows hold `c/state/w` in %ecx/%edx once postinline (or a
   source-hint-only inliner variant) removes the helper calls. Target: −1.5 KiB
   on cmdline.c+cpucheck.c+cpuflags.c alone.
2. **P0 — rerun `build_kernel_boot.sh` as a CI gate** after every i686/x86
   commit (it was not rerun when e12597c7 landed; the gate silently broke).
   Wire into `run_regression_suite.sh` (it currently only runs `*.c` — the
   shell-test phase from session-04 P2 is still not wired).
3. **P1 — value-width map for i686 Copy/phi dests** (byte/word slots + movb
   immediate stores + movb loads for char state machines). Needs either a type
   field on Copy or a backend-side inference pass (phi_eliminate knows types).
4. **P1 — vmlinux link continuation** (session 83's B5): objtool now BUILDS in
   this sandbox (make prepare produced tools/objtool/objtool with the extracted
   libelf) — B1's `.discard.annotate_insn` empty-data assembler bug is next.
5. **P2 — godbolt oracle** (`scripts/godbolt.py`, `codegen_oracle.py`): network
   to compiler-explorer works from this sandbox; not yet driven this session.

## 7. Snapshot ledger

`S01-s05-divfusion-immstore` — base `e3327e39`, head `dcd1d4f7`, patch
APPLIES-CLEAN (4 files, +446/−2). Artifacts (full tarball, bundle, series) in
`/tmp/my-project/lccc-artifacts` (PolarFS); deliverable mirrored to
`/home/z/my-project/ms178-1.patch` and `/home/z/my-project/lccc-artifacts/`.
