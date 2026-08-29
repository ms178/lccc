# Follow-up work document — 2026-08-28

**Task:** build and boot the user's custom kernel (`linux-cachymod-6.18`, i.e. `linux-6.18.46`
+ the 26 `archpkgbuilds/packages/linux-cachymod-6.18` patches) with **lccc + lccc-ld**,
using a default config. Hard gate: `arch/x86/boot` setup must satisfy `_end <= 0x8000`
(32768 B).

**Status at end of session**

| Requirement | Status |
|---|---|
| Compiles end-to-end with lccc + lccc-ld | **DONE** — 1015 TUs, 0 errors |
| 32 KiB setup gate | **PASS** — `_end = 0x7600` (30208 B) |
| objtool clean | **4 warnings left** (was an "empty alternative entry" flood); GCC reference = 0 |
| Boots under QEMU | **NOT YET** — root-caused to a single precise defect (below) |

---

## 1. Accomplished this session

### 1.1 Two real assembler bugs found and fixed

Both live in `src/backend/elf_writer_common.rs`, in the deferred `.skip` machinery that the
kernel's `ALTERNATIVE()` macro depends on (a `.skip` whose size is a symbolic expression
involving labels that are only known after the section layout is final).

**Bug A — deferred `.skip` fill bytes dragged preceding same-offset labels forward.**
`resolve_deferred_skips` spliced the fill bytes at `offset`, then advanced every label with
`loff >= offset`. A label defined *immediately before* the `.skip` also has `loff == offset`,
so it got pushed forward with the fill. In the kernel's
`.byte 773b-771b` idiom (the recorded source length of an ALTERNATIVE region) this collapsed
the difference to **0**, so objtool reported "empty alternative entry" and the boot-time
alternative patcher would have patched **zero bytes** — the SMAP/lfence-style alternatives
would silently never take effect. No diagnostic was emitted anywhere.

*Fix:* `deferred_skips` became `Vec<DeferredSkip>`; the new struct carries
`labels_at_offset` / `numeric_at_offset` snapshots taken (via `snapshot_labels_at`) at the
moment the skip is deferred. Both label-adjust loops now shift a label only if it is strictly
after the gap, **or** sits at the gap start and is **not** in the snapshot. Relocations,
jumps, alignment markers and deferred diffs keep the original `>=` semantics.

**Bug B — two deferred skips at the same offset were spliced in the wrong order.**
The sort was written as
`skips.sort_by(|a,b| a.sec.key().then(a.offset.cmp(&b.offset)).reverse())`.
`.reverse()` was applied to the *comparator's `Ordering`*, not to the vector, so the sort was
descending but **stable** — ties kept source order. Since each splice *inserts* at `offset`
(pushing already-inserted bytes right), source order puts the second gap's fill **before**
the first gap's fill, silently permuting the section contents.

Evidence (`/tmp/asmtest/two_marks.S`, two consecutive empty-original ALTERNATIVEs with
different fill bytes):

```
GAS  .text:  90 eb 04 | 90 90 90 | cc | c3
LCCC .text:  90 eb 04 | cc | 90 90 90 | c3      <- gaps swapped
```

Critically, **every recorded length still looked correct** — only the byte pattern revealed
it. *Fix:* sort ascending (stable) then `skips.reverse()`, which yields descending offsets
with ties in reverse source order.

### 1.2 Validation of the fixes

* `/tmp/asmtest/two_marks.S` — LCCC `.text` now byte-identical to GAS (`90eb0490 9090ccc3`).
* `/tmp/asmtest/two_dbg.S` — gap lengths `00 03 03 / 00 03 03`, matching GAS exactly
  (previously `00 06 06 / 00 00 00`).
* `/tmp/asmtest/two_blocks.S` — `.altinstructions` now `3/3` and `3/3` (previously `6/3`, `0/3`).
* `/tmp/asmtest/diag.S`, `negskip.S`, and the full `matrix.py` differential harness: all ok.
* **Real kernel file:** `cmp_alt.py` on `fs/readdir.o` (12 `alt_instr` entries) went
  **12 mismatching → 0 mismatching** against the GCC-built reference object.
* `tests/regression/kernel_altinstr_layout.c` extended with **Case 2c** (two consecutive
  empty-original ALTERNATIVEs). Because length checks alone cannot catch Bug B, the case uses
  **distinct fill bytes (`0x90` / `0xcc`) and verifies the byte pattern at runtime**.
  Passes under both lccc and gcc. (Negative control — reverting the fix and re-running the
  test — has **not** been done yet; it was deliberately skipped because rebuilding lccc
  mid-kernel-build would have swapped the binary out from under the running build.)

### 1.3 Kernel build results (lccc, after both fixes)

```
1015 translation units, 0 errors, ~22 min (-j2)
vmlinux            20017016 B   (gcc reference: 15483712 B -> lccc is +29 %)
bzImage             4318208 B   (gcc reference: 3593216 B)
setup.bin             25248 B   (gcc reference:   14748 B)
setup _end       0x7600 = 30208 B <= 32768  -> GATE PASS
objtool warnings          4     (gcc reference: 0)
```

Remaining objtool warnings (all present only in the lccc build; addresses stripped):

```
arch/x86/kernel/signal_64.o: x64_setup_rt_frame+: return with UACCESS enabled
init/main.o:                 start_kernel+:       rest_init() missing __noreturn
kernel/exit.o:               __do_sys_waitid+:    return with UACCESS enabled
kernel/sched/fair.o:         dequeue_entities+:   unreachable instruction
```

### 1.4 Boot-failure bisect (the main result)

QEMU (`-m 512 -smp 2 -accel tcg,thread=multi`, serial log) produced zero bytes for the lccc
image while the GCC reference produced 157 lines including the validation markers. Bisecting
by swapping image halves:

| Image | Result |
|---|---|
| gcc setup + gcc payload | **BOOTS** (157 lines) |
| **lccc setup + gcc payload (hybrid A)** | **BOOTS** (157 lines) |
| **gcc setup + lccc payload (hybrid B)** | **HANGS** (0 lines) |
| lccc setup + lccc payload | HANGS (0 lines) |

=> **lccc's real-mode setup code (`arch/x86/boot`, 16-bit) is correct.** The fault is in
`vmlinux.bin`, i.e. `arch/x86/boot/compressed/*` and/or the decompressed kernel.

Live register dump (QMP `human-monitor-command 'info registers'`, 25 s into the boot):

```
RIP=0x221bdf3  HLT=1  CR0=0x80050033 (paging on)  CR3=0x225a000
code at RIP:  eb fd    jmp 0x221bdf2        ; preceded by f4 (hlt)
```

`f4 eb fd` is the `while (1) asm("hlt")` infinite halt GCC-style compilers emit for the
kernel's `error()`/dead-end paths. Searching vmlinux for that 3-byte signature and scoring
candidates by how many sampled stack words fall inside the image localised it to:

```
HALT LOOP IS IN: fpu__init_system+0x11e
stack: fpu__init_system+0xfb, detect_art+0x1a1, early_cpu_init+0x162,
       tsc_early_init+0x161, alternative_instructions+0x11
```

Disassembling `fpu__init_system` in both vmlinux images shows **identical control flow**:

```
GCC :  mov __pi_boot_cpu_data+0x30,%rax ; test $0x1,%al ; jne <continue>
       ... else: _printk("..."); hlt ; jmp -3
LCCC:  mov __pi_boot_cpu_data+0x30,%rax ; and $1 ; cmp $0 ; setne ; test ; je <panic>
       ... panic path: _printk("..."); hlt ; jmp -3
```

Both halt iff `boot_cpu_data.x86_capability[0]` bit 0 (`X86_FEATURE_FPU`) is clear.
`__pi_boot_cpu_data` and `boot_cpu_data` are correctly aliased at the same address with the
same size (`0x150`) in **both** builds, and the static initializer is **all zeros in both**
builds — so the bit is set at *runtime* by the CPUID path.

**Conclusion:** the lccc-built kernel never sets `boot_cpu_data.x86_capability[0]` bit 0
before `fpu__init_system()` tests it. The defect is therefore in the **CPU feature
detection path** — `early_cpu_init()` / `identify_cpu()` / `get_cpu_cap()` and the `CPUID`
inline-asm / capability-bit-setting helpers in `arch/x86/kernel/cpu/common.c`,
`arch/x86/lib/cpu.c` (or their callees) — not in the alternative machinery and not in the
real-mode setup.

---

## 2. To-do (ordered by expected impact)

1. **Fix the CPU-feature-detection miscompile (blocker for booting).**
   Narrow it further with a targeted A/B: compile *only* `arch/x86/kernel/cpu/common.c`,
   `arch/x86/kernel/cpu/intel.c`, `arch/x86/lib/cpu.c` etc. with gcc into the otherwise
   lccc-built vmlinux, bisecting which TU restores the FPU bit. Then reduce that TU.
   Prime suspects: `cpuid`/`cpuid_count` inline asm constraints, `set_cpu_cap()` bit
   manipulation, `memcpy` of `struct cpuinfo_x86`, and any `__builtin_cpu_supports`-style
   construct. The `f4 eb fd`-signature + QMP register-dump technique in §1.4 is reusable and
   cheap (~30 s per experiment).
2. **Negative control for Bug B.** Temporarily revert the `skips.reverse()` change, rebuild
   lccc, and confirm the new Case 2c in `tests/regression/kernel_altinstr_layout.c` actually
   FAILS. This must be done **when no kernel build is running** (rebuilding swaps the binary
   out from under an in-flight build).
3. **Zero out the 4 remaining objtool warnings.** "return with UACCESS enabled" is
   potentially a real correctness issue (stac/clac pairing), not just noise.
4. **Code-size gap.** lccc `vmlinux` is +29 % vs gcc. Once it boots, profile the biggest
   deltas — with the icount harness now proven, size is a good proxy for the perf work.
5. **Then** the in-QEMU A/B performance work (`scripts/in_vm_codegen_bench.sh`,
   `scripts/qemu_icount_plugin/`) that the boot gate has been blocking.

## 3. Gaps, risks and technical debt

* The deferred-`.skip` subsystem still has **no negative-control coverage for Bug A** either
  (Case 2b passes, but has not been shown to fail on the pre-fix compiler).
* No unit tests exercise the `DeferredSkip` snapshot logic directly; all coverage is
  end-to-end assembly-level.
* `resolve_deferred_skips` is called, then `relax_jumps()` runs again afterwards. The
  interaction between the two passes is only covered indirectly; the reasoning in the
  session notes about it did not match observed behaviour, so it deserves a careful re-read
  with instrumentation rather than more inference.
* The kernel can only be built **in-tree** in this tree (`O=` is rejected: "source tree is
  not clean", and `make clean` does not satisfy it), so gcc-reference and lccc builds must
  run **sequentially** with a `make clean` between them, copying outputs out first.
* Disk is tight: 25 G root, ~6 G free. The 1.7 G kernel tree plus two 15–20 MB vmlinux
  copies per configuration is near the practical limit.

## 4. Artifacts

* `/home/user/artifacts/kernels/lccc-fixed-boot-bisect/` — `vmlinux.lccc`, `bzImage.lccc`,
  `setup.bin.lccc`, `hybA.bin`, `hybB.bin`, `build.log`.
* `/home/user/artifacts/kernels/` — GCC reference: `bzImage.gcc`, `vmlinux.gcc`,
  `setup.bin.gcc`.
* `/home/user/artifacts/repro/` — the assembly reducers (`two_marks.S`, `two_dbg.S`,
  `two_blocks.S`, `two_distinct.S`, `diag.S`, `negskip.S`) plus `matrix.py` and `cmp_alt.py`.
* Build logs: `/home/user/kernel-build-lccc3.log`, `/home/user/gcc-kbuild.log`,
  `/home/user/build-lccc5.log`.
* Boot logs: `/tmp/boot-gcc.log` (working GCC boot, 157 lines), `/tmp/bisect/o.*.log`.
* Auto-saved patch: `/home/user/ms178-1.patch` (snapshot S05).

## Session S10 (c5623a76) — trampoline + unroll defects fixed, two pre-existing ARM fails retired

- **nested_functions (rc=139) FIXED** (fd4bfd9b, 2 defects): (1) AArch64 trampoline
  `ldr (literal)` imm19 values were word-offsets, not byte/4 — x18 read the nop pad
  at +12, x17 read half the chain at +20; correct words 0x5800_0092/0x5800_00B1
  (data at +16/+24; x17 word byte-identical to GCC 14.2). (2) The builtin AArch64
  linker hardcoded RW PT_GNU_STACK; trampoline units emit `.note.GNU-stack,"x"` and
  the emitters must OR input notes into PF_X (i686 already did). All three emitters
  fixed; `bti c` still omitted until lccc emits .note.gnu.property (revisit with BTI).
- **lea_sib_fold (45 18 vs 45 25) FIXED** (c5623a76): mid-end, in
  `loop_unroll::do_unroll` — only the IV phi was threaded through the clone chain,
  so other loop-carried phis (accumulators) read stale values in every clone, their
  updates died, and k-1 of every k elements were dropped; the early-exit edges also
  carried the preheader phi value. Fixes: carried-phi threading (clone j reads
  clone j-1's renamed latch operand; header latch-incoming = last clone's copy),
  correct per-edge values for header-fed exit phis, and step 5b inserting real exit
  phis + post-exit reader rewrites when phis were already lowered (unique-def
  invariant preserved — an explicit-copy variant was caught by
  test_value_ids_unique_after_unroll and replaced).
- Gates: lib 1282/0 (-D warnings); ARM 379/93 A/B vs 377/95 (fail set otherwise
  identical); x86 corpus 508/0; check_* suite at its 5 documented pre-existing
  fails; 4-backend differential (remainder shapes, IV-after-loop, multi-carried)
  ALL OK; benchmark corpus instruction counts identical (6036→6036).
- Oracle (artifacts/evidence/oracle-S10.md): peep probe lccc 34 insns vs GCC 16.1
  48 (1.41x smaller); tramp probe words match GCC.
- Open: i686 trampoline emitter audit (emit_init_trampoline_impl) for the same
  hand-encoding bug class; ARM byte-loop shape thread; the 93-fail ARM corpus is
  dominated by x86-artifact tests (harness may need arch filtering).

## Session S11 (post-#285 rebase onto d4283869) — i686 audit clean; ARM peephole pass 6/7

- **i686 trampoline emitter audit: CLEAN** (negative result, no fix needed).
  Template `B9 imm32 | FF 25 disp32` verified at runtime (freestanding probe +
  full nested_functions rc=0 native), all of -O0/-O1/-O2/-fomit-frame-pointer/
  -fPIC green; .data.rel.ro template + RWE GNU_STACK chain already correct.
- **Driver defect fixed (2e41aca2):** i686 set_target() filtered the shared
  multilib GCC include dir (/usr/lib/gcc/x86_64-linux-gnu/<v>/include) out of
  the -m32 search path — stddef.h/stdarg.h unresolvable on Debian multilib
  (real `gcc -m32` uses that exact dir). Only x86-64 *C library* dirs are
  filtered now; dedicated i686/gcc-cross dirs still probed first.
- **ARM peephole pass 6 (spill-slot threading) + pass 7 (adrp+add remat CSE)**
  (this commit): pass 6 rewrites `str xS,[sp,#off] ... ldr xD,[sp,#off]`
  round-trips through an unused caller-saved scratch when xS is clobbered in
  between; store deleted only when a function-wide ±8-byte overlap-aware
  census proves total coverage. Guard rails learned from red-team A/B: (1)
  partial-word stores inside the slot's bytes (strb at #off+3) must block —
  exact-token census is unsound; (2) x18-x30 destinations must NOT be paired
  (ABI home restores become Move-kind and get deleted by ABI-unaware dead-
  move passes — costaware_devirt segfault class). Pass 7 deletes duplicated
  adrp+add remat pairs in-block (same-reg) or retargets the add (cross-reg),
  allowlist-based def detection, unknown insns kill all state.
  GCC-include-fixture test_gsf_invalidation_on_reg_overwrite updated to the
  strictly-stronger threaded value-flow shape (empirically verified).
- Gates: lib 1289/0 (-D warnings; +7 fixtures); ARM corpus 379/95 = baseline
  d4283869 (379/93 pre-#284-env + 2 pre-existing fails that fail on pristine
  too: i686_fused_mul_add_operand_order, segment_fill_copy_alias — came with
  PR #284 or the refreshed toolchain); benchmark corpus 8686 -> 8630 insns
  (15 programs shrink, 0 grow); 4-backend differentials green.
- Known-carry-over: the big ARM size gap vs GCC (2-5x on branchy mains) is
  dominated by RA slot-staging of loop-invariant constants/addresses — the
  recorded RA accumulator-machine item (DECISIONS-listed, do not interleave
  with MS-08/RA-06A).

## Session S12 (820a1077) — ARM pass 8: loop-invariant remat hoisting

- **Pass 8 (CCC_NO_INVAR_HOIST)**: hoists pure def-sequences (movz/movk
  chains, adrp+add pairs, sp-slot loads) out of call-free loops when the
  dest is self-contained (no other in-loop def, no mention before the seq,
  only use-first readers after, dead after the loop). Entry edges are the
  fallthrough plus branches from above the top — each gets a copy; the
  in-region jumps to the top are backedges (dest persists, nothing defines
  it in-loop). Net effect on real shapes (arm_gep_regbase_acc_cache,
  gep_family_stride_cse, range_check_fold, vectorize_iv_dependent_base):
  per-iteration loads leave the loop body — runtime win, size neutral.
- Design limits (measured): the RA reuses x0/x1 temps heavily, so most
  latch `movz` remats have a *different-value* co-def in the loop and are
  correctly rejected — those need the RA to keep loop-invariant remat
  candidates in dedicated registers (RA-06 territory, BACKLOG P0, Wimmer-
  style; DECISIONS-gated). Pass 8 fired zero times on the benchmark corpus
  (that corpus's loops keep remats in the latch with co-defs) but 4+
  regression shapes hoist.
- Gates: lib 1293/0 (-D warnings; +4 fixtures incl. 3 negative-soundness
  cases: in-loop store, post-loop reader, call in loop); ARM corpus
  379/95 = exact baseline fail set; asm A/B on regression corpus: 7 files
  differ, all in the intended direction.

## Session S12b — RISC-V peephole passes 6/7 (jump-near + sext elim)

- **Pass 6 (CCC_NO_JUMP_NEAR)**: `jump .Lxxx, tN` (auipc+jalr, 2 insns/8B)
  → `j .Lxxx` (jal, 1 insn/4B) when the target label exists in the same
  function and the function is < 100k insns (jal ±1MiB with 4x margin).
  Tail calls / cross-function jumps keep the unlimited-range form.
  Corpus: 740 far jumps → 0; -2.4% asm bytes; every branch transition also
  executes 1 insn instead of 2.
- **Pass 7 (CCC_NO_SEXT_ELIM)**: per-block sext-wide dataflow (li/lui/W-ops/
  lw/slli|srli n>=1/srai produce sign-extended-32; ld/64-bit ALU/lla don't;
  calls clear caller-saved; labels/branches clear all). Deletes ONLY the
  self form `sext.w rX, rX`: even a value-no-op sext.w *defines rD*, so the
  cross-register form must stay (cfg_phi_web: `sext.w t2,t0` deleted left
  `bne t2,t1` reading a stale register — caught by the corpus, fixed,
  fixture documented in-code). Corpus: 340 → 209 sext.w.
- Gates: lib 1294/0; RISC-V corpus 360/91 = exact switch-off baseline;
  arm corpus untouched (riscv-only change).
