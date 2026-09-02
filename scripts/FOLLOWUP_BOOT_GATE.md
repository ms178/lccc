# Linux Cachymod 6.18.47 boot-code work — follow-up notes

Date: 2026-09-02.  Goal: build `arch/x86/boot` with lccc + lccc-ld and keep
`setup.elf` under the 32 KiB gate (`_end <= 0x8000`).

## Current gate
- lccc (this session): `_end = 0x898c` region; `.text = 23620`, `.bss = 4960`.
  Gate FAILS with overflow ~ 2.4 KiB.
- GCC oracle: `.text = 10154`, `_end = 0x4200`.
- lccc `.text` is ~2.3× GCC. The gap is register-allocation quality, not any
  single missing pattern.

## What this session changed (i686 text peephole, `src/backend/i686/codegen/peephole.rs`)
1. `eliminate_dead_frame_allocation` — remove `subl $N,%esp` / `addl $N,%esp`
   for register-only leaves (single alloc, matching deallocs, no frame
   pointer / call / push / slot ref / indirect jmp; every body path crosses
   the alloc). Wired into Phase 4 after `eliminate_unused_callee_saves`.
   Kill switch `CCC_NO_I686_DEAD_FRAME_ELISION`.
2. Widen store-forwarding in `forward_slot_loads`: narrower slot store feeding
   a wider unsigned reload (`movb %al,slot` / `movl slot,%eax` ->
   `movzbl %al,%eax`).
3. `forward_immediate_slot_loads` + immediate homes in
   `eliminate_never_read_stores_range`: `movl $K,slot` ... `movl slot,%reg`
   materializes `$K` directly; never-read immediate stores are deleted.

All 1555 lib unit tests pass. `plain(u16){return (u8)(a+1);}` now compiles to
the 4-instruction GCC-shape leaf (`movzwl; incl; movzbl; ret`); `__inb` is a
tight 5-instruction frameless function.

## Remaining gap — ROOT CAUSES (ranked by expected payoff)
All visible in the boot asm (`/tmp/boots3/*.s`).

1. **Per-value stack spilling (biggest).** The accumulator/regalloc backend
   homes nearly every SSA value in a frame slot and reloads it for the next
   use, even across a single intervening instruction in the same straight
   block: `movl %eax,N(%esp); <one insn>; movl N(%esp),%edi`. GCC keeps the
   value in a callee-saved register. `number()` (printf.c) alone has 103
   `N(%esp)` refs and is 1148 B vs GCC 600; `set_video` 2478 vs 1204. The
   windowed `forward_slot_loads` already folds adjacent pairs; the fix needs a
   cross-instruction (still intra-block, barrier-delimited) value-availability
   analysis: after `movl %R,slot`, keep `slot = R` until R or the slot is
   clobbered or a barrier (call/label/push/indirect) is hit, and rewrite every
   same-width reload of `slot` in that range to `%R`. Much of the bookkeeping
   already exists in `forward_slot_loads`; its WINDOW=48 stops early on
   `LineKind::Label`/`Call` (correct) but also on many harmless straight-line
   instructions. Profile why the window terminates early on `number()`.

2. **Redundant callee-save push/pop.** `get_cpuflags` does
   `pushl %ebx/%esi/%edi/%ebp` with ZERO body uses of any of them (each appears
   only in its own push + two epilogue pops). `eliminate_unused_callee_saves`
   only acts on single-`subl` framed functions; these bodies have nested
   `subl $16` outgoing-arg areas (multi-subl), so the transform is skipped and
   all four pairs survive. Extending the envelope proof to multi-alloc
   functions (or first eliminating #3) frees the pairs + frame growth.

3. **Nested outgoing-argument area per call.** Around every stack-arg call
   lccc emits `subl $16,%esp; <leal arg-area>; ...; addl $16,%esp` AND
   duplicates the argument staging (values stored to frame slots above, then
   reloaded and re-stored into the pushed area:
   `movl 32(%esp),%eax; movl %eax,4(%esp)`). ~21 `subl $16` sites in boot.
   GCC reserves one outgoing area in the function frame and fills it directly.
   This is the interaction that creates the multi-subl shape blocking #2 and
   produces the duplicate `movl slot,%eax; movl %eax,slot` chains.

4. Redundant entry `movzwl %ax,%eax` for -m16 regparm u16 args that are
   consumed only through `%ax`/`%dx` and never need 32-bit zero-extension in
   16-bit context (GCC emits none). `eliminate_redundant_zext_i686` does not
   seed a u16 fact for function-entry parameter registers.

## Build / harness facts
- Compile lccc: `bash scripts/build_lccc_fast.sh` (-O1 -j2 fastbuild, ~15-30 s
  incremental). Tests: `cargo test --profile fastbuild --lib`.
- Boot gate: `bash scripts/build_kernel_boot.sh`; read the `32 KiB gate:` line
  (script always exits 0). Objects land in `/tmp/bootbuild`.
- Snapshot after each validated fix: `bash scripts/lccc-snapshot.sh "<slug>"
  "<desc>"`; refreshes `/home/user/ms178-1.patch`.
- Do NOT run `cargo fmt` — it reformats the whole tree and pollutes the patch.
  Hand-format new code to match surrounding style.
- GCC oracle link: `ld.bfd --gc-sections -m elf_i386 -z noexecstack -T
  setup-gc.ld <objs>`.
