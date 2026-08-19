# 2026-08-19 (session 11) — kernel boot bloat: memory-compare fold + sign-ext elimination

**Base:** `origin/main` @ `1c127e47` (PR #126 — sessions 9–10 landed upstream)
**Snapshot:** `/home/user/ms178-1.patch`

## Re-base

Sessions 9–10 are upstream (PR #126, "backend: tighten liveness and
speculative codegen invariants"). Re-based onto `1c127e47` cleanly; the
working tree was byte-identical to upstream modulo the usual 37 lost `+x`
bits. Re-extracted linux-6.18.44 + re-cloned archpkgbuilds (both wiped),
re-applied the 26 patches, regenerated the boot-code stubs
(voffset.h/zoffset.h/capflags.c/cpustr.h/utsversion.h).

## Boot gate state (re-measured)

setup.ld layout at `1c127e47` (before this session's changes):

```
.bstext 495  .header 125  .entrytext 114  .inittext 429  .initdata 30
.text 31402  .text32 30  .pecompat 9 (@0x8000, 4K-aligned)  .rodata 1369
.videocards 84  .data 140  .signature 4  .bss 4960   _end = 39344
```

The `.pecompat` 4K alignment forces `.text <= ~27449` for `setup_size <=
0x8000`; we start ~4 KB over. GCC builds the same objects at 19078 bytes
text (lccc ~1.78×).

## Root cause (measured, not guessed)

Per-object text (lccc vs gcc): printf 4875/1829, string 3878/1182, video
3859/1726, early_serial 2203/966, cpucheck 2080/861, cmdline 1684/483.
The .S objects are identical to GCC; the C-object gap is ~100 % *three*
i686-codegen taxes:

1. **Sub-word load + test**: `movsbl (%r),%R; testl %R,%R` where GCC emits
   `cmpb $0,(%r)`.
2. **Redundant re-sign-extension**: `movsbl (%m),%eax; movsbl %al,%eax` and
   `setbe %al; movzbl %al,%eax; movsbl %al,%eax`.
3. **Accumulator/%eax reservation**: only %ecx/%edx are caller-saved and
   both are hazard-restricted (indirect access stages the address through
   %ecx), so leaf loops spill every live value to %ebx..%ebp and pay 3–4
   push/pop pairs. GCC uses %eax/%ecx/%edx freely. `strlen` is 17 insns +
   3 push/pop vs GCC's 13 with none.

## Fixed this session (validated)

### 1. `load; cmp {Eq|Ne, imm}` → `cmpb/cmpw/cmpl $imm, (mem)`

- `generation.rs`: `detect_load_cmp_mem_fold()` finds Loads whose **single
  use** is the **adjacent** `Cmp { Eq|Ne }` against a foldable immediate,
  and `cmp_fold_imm()` bounds the immediate to where sign- and
  zero-extension agree on ZF (0, or `[0,127]` bytes / `[0,32767]` words;
  any i32 for full-width loads). Gated by `ArchCodegen::supports_load_cmp_mem_fold()`
  (i686 only).
- The instruction loop skips the Load and records `(ptr, ty, imm)` in
  `state().pending_load_cmp` (cleared per block); the i686 `emit_int_cmp_impl`
  and `emit_fused_cmp_branch_impl` consume it and emit the memory compare,
  staging the pointer exactly like an indirect load. A defensive fallback
  re-materializes the load via `emit_load_default` if anything mismatches.
- Soundness: Eq/Ne only consumes ZF, and `cmp{b,w} $imm,(mem)` agrees with
  `movX + cmpl` on ZF for the admitted immediates; adjacency keeps the
  pointer live at the compare site; `use_count == 1` proves no other
  consumer reads the skipped value.

### 2. `eliminate_redundant_sign_ext_i686` (peephole)

Tracks per-register-family "sign-extended" (`bits 8..31 == sign(bit7)`) and
"bool" (`setcc`) state, fail-closed at every barrier, mirroring the existing
`eliminate_redundant_zext_i686`:
- `movsbl %al,%eax` on an already-sign-extended %eax → removed.
- `movsbl %al,%REG` on an already-sign-extended %eax → `movl %eax,%REG`.
- `setcc %al; movzbl %al,%eax` marks %eax bool/sign-extended, so the
  trailing `movsbl %al,%eax` of the `setcc;movzbl;movsbl` idiom is removed.

4 new unit tests cover the no-op, cross-reg, setcc chain, and the negative
case (`movzbl` of a high-byte value is NOT sign-extended).

## Result

- setup.elf `.text` **31402 → 31271 (−131 bytes)**; string.o −82,
  early_serial_console.o −26, printf.o −19. Still ~3.8 KB over the gate.
- 914 unit tests (was 910), 50/50 correctness, 339+6 regression,
  240-case i686 differential fuzz (O0/O2/Os), warning-free fastbuild.

## Remaining work (ranked, with the mechanism identified)

1. **%eax allocatability** — the 4-push/pop tax. Hard because the whole
   i686 backend funnels through %eax (`operand_to_eax`/`store_eax_to`,
   returns, casts, div/rem); a per-point conservative hazard whitelist
   cannot cover it (a Copy's dest register isn't known until allocation).
   Needs the accumulator paths made tolerant of a live %eax. Multi-session.
2. **Pointer-in-%ecx** (experimented, reverted — measured 0 bytes):
   `emit_load_ptr_from_slot` always stages the address through %ecx, so the
   load's OWN pointer value cannot live in %ecx even though `movl %ecx,%ecx`
   is a no-op. I implemented a per-value self-exemption in Phase 2
   (`ecx_self_exempt`, `overlaps_inclusive_except`) and validated it with the
   full fuzz/regression suites — but it changed **nothing**: Phase 1 hands
   the hot loop values to the callee-saved pool first, so %ecx/%edx in
   Phase 2 only ever see short-lived leftovers regardless of the exemption.
   The real enabler is a **caller-saved-first allocation order** (a "Phase 0"
   that gives hazard-eligible values %ecx/%edx before callee-saved spill),
   which is a deliberate allocator rework with its own regression surface.
3. **Multi-use load → two memory compares** (`strchr`: `while (*s && *s !=
   c)`): today `use_count == 2` keeps the byte in %esi; GCC emits `cmpb
   %c,(%s); je; cmpb $0,(%s); jne`. Generalize the fold to "every use is a
   foldable compare" (track remaining consumes in `pending_load_cmp`).
4. **Cast-after-signext relay** (`movl %esi,%eax; movl %eax,%edi; cmpl
   %ebp,%edi` for `*s == c`): the i8→i32 cast of an already-sign-extended
   load is a no-op; the compare should use %esi directly.
