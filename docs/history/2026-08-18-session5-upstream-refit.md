# 2026-08-18 (session 5) — Upstream refit + zext/lea peepholes

**Base:** `origin/main` @ `a4b98b5` (post-PR #116)
**Branch:** `arena/audit-refit` — snapshot `/home/user/ms178-1.patch`
**Context:** PRs #113–#116 merged large agent deliveries (SLP distances, dot-FMA,
multi-accumulator XMM, i686 setup-path tightening, lccc-ld ELF32 script links).
The S08/S17 content was merged in PRE-audit form.

## Commits this session

1. **`a2eb40f` — Re-apply the 5 lost audit fixes + fix a 6th (new) defect**
   - Fixes 1–5 are the session-4 audit set (FMA contract gating both backends,
     GCC-parity contraction defaults/stickiness, mul-add fusion skipped-def
     soundness, i686 cmp-immediate width normalization, ymm2→ymm1 fallback).
     All five reproduced on the new main; verified by reproducer before and
     after.
   - Fix 6 (NEW, introduced by #116): the merged "capture regparm incoming
     EAX/EDX/ECX" loop re-captured params already saved by the phase-1/2
     capture pass, interleaved with %eax-staging stack-param copies →
     `d_i(double, int)` read b as garbage at -O1+ (regparm conformance
     bit 5). The loop now skips `param_pre_stored` entries.
   - **Process note for future sessions: upstream merges of agent patches may
     take the PRE-audit version. ALWAYS re-run the defect reproducers after a
     re-base** (they are all in this doc's Validation section and in
     tests/regression/).

2. **`1151756` — i686 redundant zero-extension elimination** (port of x86-64
   `redundant_ext.rs`): per-family byte/16-bit range tracking, deletes
   same-family re-extensions, rewrites cross-family byte re-extensions to
   movl, andl-immediate range proof, fail-closed at barriers and implicit
   writers. −534 bytes on the realmode corpus. 4 unit tests.

3. **`789...` — leal+move fusion**: Pattern 2b extended to
   `leal M(%r), %eax; movl %eax, %REG` → `leal M(%r), %REG` (refused when
   %eax is base/index). The regparm intcall address-argument shape. −313 bytes.

## Realmode corpus trajectory (arch/x86/boot C files, REALMODE_CFLAGS, GCC = 12999)

| checkpoint | bytes | ratio |
|---|---|---|
| session-4 exit | 36500 | 2.80 |
| main @ a4b98b5 (upstream #113–116) | 35338 | 2.71 |
| + audit refit (no size effect, correctness only) | 35338 | 2.71 |
| + i686 redundant-zext | 34804 | 2.67 |
| + leal fusion | **34491** | **2.65** |

x86-64 -O2 benchmark corpus: 25364 (GCC 25813, ratio .983) — byte-stable
through the i686-local passes, improved by the audit's contract default.

## Worst remaining objects (post-session)

video-bios 3.7×, string 3.5×, cmdline 3.4×, cpuflags 3.2×. Top remaining
bigrams: `movzbl %al,%eax; movl %eax,SLOT` (spill of freshly extended byte —
needs the store to use the byte register or the slot classification to
recognize byte slots), `movl 8(%esp),%eax; cmpl 0(%esp),%eax` (two-slot
compare → fold one side as memory operand; the x86-64 memory_fold pass has
this, i686's fold_memory_operands only folds loads INTO alu, not cmp lhs).

## Next levers (ranked)

1. **i686 cmp memory-operand folding**: `movl SLOT,%eax; cmpl SLOT2,%eax`
   → `movl SLOT,%eax` already exists; the missing fold is when BOTH sides
   are slots (compare needs one reg: keep lhs load, fold rhs — already done —
   so the residual is lhs-load elimination when %eax already holds SLOT via
   the reg_cache. Look at reg_cache usage in emit_int_cmp).
2. **Byte stores from byte registers**: `movzbl %al,%eax; movl %eax,SLOT;`
   where all consumers are byte loads — store `%al` with movb and mark the
   slot byte-width (needs slot width tracking, medium effort).
3. **Inline cost at -Os**: duplicated unrolled bodies (adler32 len_16→len_64
   case from session 4; also hits boot video code).
4. **setup.elf end-to-end**: with lccc-ld now speaking ELF32 script links
   (#116), the full `make bzImage` gate is worth re-running with
   LD=lccc-ld — see if "Setup too big!" distance shrank proportionally
   (session-3 measure: 45210 total vs 32768 limit; C-text is now ~4.3K
   smaller than then).

## Environment protocol (unchanged; harness wipes everything)

swap 10G → rustup stable minimal → `cargo build --profile fastbuild --locked -j2`
→ apt bison flex bc libelf-dev cpio kmod qemu-system-x86 busybox-static
gcc-multilib libc6-dev-i386 → kernel 6.18.44 + 26 archpkgbuilds patches
(1020 has the PAGE_POOL fix upstream now) → defconfig+prepare with HOSTCC=gcc.
Validation reproducers: tests/regression/regparm3_abi_conformance.c (all -O),
tests/benchmark/programs/nbody.c (all -O, want -0.169075164),
volatile uchar post-inc matrix, contract flag matrix vs gcc.
