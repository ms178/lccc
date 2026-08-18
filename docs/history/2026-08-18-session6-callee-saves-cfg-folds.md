# Session 6 (2026-08-18, part 2): callee-saves, CFG folds, and a wide-operand miscompile fix

## Base

Re-based onto upstream `d7a64eb` (PR #118 = user's merge of session-5/6a work,
commit `14e34f3` "Restore lost S08/S17 codegen correctness and tighten i686
peephole fusion"). **The merge again took a squashed form**: the six audit
fixes and the two i686 zext/lea peepholes are IN, but the five session-6a
commits (function-scoped DSE, windowed-DSE fix, 2b-mem, 2d fix, global-fnptr
fold) were NOT in #118 — they were re-applied from the `ms178-1.patch`
snapshot (patches 5–9) and re-validated. All six defect reproducers green
after re-base.

## Landed this session (each committed + snapshotted after validation)

1. **Unused callee-save elimination, multi-epilogue + frameless shapes**
   (`eliminate_unused_callee_saves` rewrite). Old pass required exactly one
   subl + one addl; early-return epilogues (get_cpuflags, go_to_protected_mode)
   and frameless functions (bcmp post-tail-call) kept 20 dead push/pop pairs.
   New shapes: FRAMED (1 subl, n same-value addls, per-epilogue pop
   qualification, +4/pair on alloc AND every dealloc) and FRAMELESS (no ESP
   arithmetic AND no (%esp) references → shifting body ESP unobservable).
   bcmp is now literally `jmp memcmp`. 33194 → 33138.

2. **Global-fnptr fold: linear-seal proof** (in addition to slot ownership).
   Walking forward from the call across straight-line code (no label, branch,
   ret, push/pop, ESP arith, or overlapping slot access), if the FIRST access
   overlapping the slot's 4 bytes is an exact full-width movl store, the
   staged value is observable only by our call. Chains fold by induction.
   Overlap windows [d−3,d+3] in both proofs. 33138 → 32926.

3. **CFG-liveness load/materialize retarget**: `movl SRC,%eax; movl %eax,%REG`
   → `movl SRC,%REG` when liveness proves %eax dead across the branch edge
   (phi-move-before-jmp shape, invisible to windowed passes). 32926 → 32735.

4. **MISCOMPILE FIX (pre-existing on main): folded memory operands are RANGES.**
   spectral_norm printed 44.72 instead of 1.274 at `-m32 -O2/-Os`.
   `fstpl 20(%esp)` writes 20..28 but three i686 passes compared
   `ebp_offset == slot` (point equality), so forward_slot_loads kept a
   `%edx holds 24(%esp)` mapping alive across the x87 store and the sum's
   high word came from the previous iteration. Fixed with
   `ranges_overlap(slot, bytes, folded_off, 16)` in forward_slot_loads,
   eliminate_dead_stores, and the 2b-mem deadness scan. **x86-64 twins had
   the same latent defect** (access_size 1 in store_forwarding
   invalidate_slots_at; 8-byte read ranges in DSE) — fixed defensively, no
   reproducer found (slot spacing currently hides it). Unit test pins the
   20-line reproducer. Cost +15B (unsound forwardings given back): 32735 → 32750.

5. **CFG-liveness compare-with-memory fold**: `movl SLOT,%R; testl %R,%R; jcc`
   (or `cmpl $imm,%R`) with %R dead on all successor paths →
   `cmpl $0|$imm, SLOT; jcc`. Flag-exact (testl r,r ≡ cmpl $0,mem for all
   jcc). 64 candidate sites. 32750 → 32610.

6. **CFG-liveness copy-then-spill retarget**: `movl %REG,%eax; movl %eax,SLOT`
   → `movl %REG,SLOT` (17 sites). 32610 → 32574.

## Numbers

- Realmode boot corpus (19 .c of arch/x86/boot at kernel flags):
  **32574** vs GCC 12999 (2.51×). Session start: 33194.
- Full trajectory: 42175 → … → 33194 (session start) → 32574.
- x86-64 -O2 benchmark corpus: **25027 vs GCC 25796 — still smaller than GCC**.
- setup.elf (assert-relaxed link, gas .S objects + dummy z/voffset):
  `_end = 43424` vs limit 32768 (GCC: 22944). NOTE: `.pecompat` has 4K
  alignment, so `_end` moves in 4K steps — `.text` (31806) must drop below
  ~24.5K before `_end` can reach 32768. **Remaining goal ≈ −8.4K of .text.**

## Correctness status

851→853 unit tests, 50/50 correctness, 9/9 check scripts, regparm
conformance -O0..-Os, uchar-255 matrix, contract matrix, nbody -O0 exact,
fnptr runtime matrix (struct dispatch + volatile fp + store-between,
-O0..-Os × regparm on/off), benchmark parity vs gcc at -m32 AND -m64
(nbody, spectral_norm, binary_trees).

## Where the remaining fat is (fresh census, post-folds)

- 119 `movl S1,%eax; movl %eax,S2` + 109 reverse — CROSS-slot value routing,
  inter-value, needs coalescing/regalloc, not windows (widening windows to
  160 gained 0 bytes — measured).
- 54 `leal S,%edx; movl $I,%eax; call intcall` — biosregs setup, IR-level.
- 52 `movzbl %al,%eax; movl %eax,SLOT` — byte-width slot stores (store
  `movb %al` + track byte slots) still unimplemented (session-5 TODO #3).
- Inline-asm operands forced to slots (`allow_inline_asm_regalloc=false`
  for i686 in regalloc.rs ~line 2334): cpuid_count is 33 insns vs GCC 19.
  x86-64 asm emitter checks reg_assignments; i686 could too — BIG win
  (video/cpuflags/cpucheck are asm-heavy), but needs careful audit of
  i686 scratch conventions (%ecx/%edx staging).
- 90 `popl %ebx; ret` etc. are legitimate epilogues now (dead pairs gone).

## Next steps

1. i686 inline-asm operand regalloc (`allow_inline_asm_regalloc` path):
   the single biggest remaining lever (~2-3K est. across corpus).
2. Byte-width slot stores to kill movzbl+movl pairs (−52 sites).
3. Slot coalescing for the 228 cross-slot copies (hard; regalloc-level).
4. Once .text < ~23.4K: full bzImage + QEMU boot test
   (`qemu-system-x86_64 -kernel bzImage -initrd busybox.cpio -append
   "console=ttyS0 rdinit=/init" -nographic -no-reboot -m 256M`).
5. On every re-base: re-run the 6 audit reproducers + spectral_norm -m32
   -O2 (new reproducer #7: must print 1.274224152).

## Environment re-provisioning (unchanged, see session-5 doc)

Swap → rustup → apt (incl. gcc-multilib) → kernel 6.18.44 + 26 patches in
PKGBUILD order → defconfig+prepare → /tmp/rmflags.sh + /tmp/rmbench.sh +
/tmp/build_setup.sh (this doc's measurements). `git am` the snapshot's
unmerged tail patches after every re-base; check which ones upstream took.
