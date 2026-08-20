# 2026-08-20 (session 28) — real-mode branch relaxation restores 895 boot bytes

**Base:** `upstream/main` @ `fd289b0ecab8157e08ba878806ae22f4f4d82941`
**Target:** patched linux-cachymod 6.18.44 `arch/x86/boot`, `-m16 -Os`,
`lccc` + `lccc-ld`, package config and all 26 PKGBUILD patches.

## Root cause

The i686 instruction encoder correctly encoded `.code16` near branches as
`jcc rel16` (4 bytes) and `jmp rel16` (3 bytes), and correctly returned a
`JumpDetection`. The shared ELF writer then discarded that detection by
re-checking the encoded instruction against hard-coded rel32 lengths (6 and 5
bytes). Consequently no compiler-generated local branch in the kernel boot
code was registered with the fixed-point branch relaxer.

A byte-level census of the baseline objects found 756 branches, 579 of which
fit a signed 8-bit displacement. Their theoretical avoidable cost was 916
bytes. A minimal probe confirmed the defect:

```text
LCCC: 66 31 c0 0f 84 03 00 e9 02 00 ...
GAS:  66 31 c0 74 02       eb 02       ...
```

This was an assembler architecture bug, not an optimization heuristic and not
a linker-size-accounting error. GNU bfd and `lccc-ld` produced identical
layouts from the same LCCC objects.

## Fix

`JumpInfo` now retains the architecture/mode-specific near-form length.
The writer trusts the encoder's detection instead of imposing rel32 lengths.
The optimistic shortest-fixed-point algorithm can therefore shrink rel16
branches, and, when alignment moves a target back out of rel8 range, restore
the original 4/3-byte form with `R_386_PC16`, addend `-2`, and a two-byte patch
field. The rel32 path remains 6/5 bytes with a four-byte field.

The relocation-width restoration matters for correctness: growing a code16
branch with the old hard-coded `R_386_PC32` path would overwrite the following
instruction.

`tests/regression/check_i686_code16_relax.sh` is a byte-exact GNU-as oracle. It
covers:

- a local Jcc shortened from rel16 to rel8;
- a local Jmp shortened from rel16 to rel8;
- a deliberately distant branch first shortened optimistically and then grown
  back to rel16;
- exact `.text` agreement with GNU as.

## Measured result

linux-cachymod real-mode C corpus (20 files):

| Metric | Before | After | Delta |
|---|---:|---:|---:|
| LCCC executable text | 30,011 B | **29,116 B** | **-895 B (-2.98%)** |
| GCC 14 executable text | 13,327 B | 13,327 B | 0 |
| LCCC/GCC ratio | 2.25 | **2.18** | -0.07 |

No-ASSERT setup layout after the fix:

```text
entrytext   103 B
inittext    380 B
text      29235 B
_end      0x99c0 (39360)
limit     0x8000 (32768)
```

The real 64-sector ASSERT still fails. This patch closes 895 bytes without
changing generated control flow, but the remaining 6,592-byte `_end` gap is
still dominated by i686 slot traffic/register pressure. Crossing the next
4-KiB `.pecompat` alignment boundary is necessary before the final layout can
fit; disabling the ASSERT or weakening `.pecompat` alignment is not a fix.

## Validation

- warning-free `scripts/build_lccc_fast.sh` (`opt-level=1`, two Cargo jobs);
- targeted code16 differential against pinned GAS 2.47.20260726: byte-exact PASS;
- `cargo test --lib`: 959 passed, 6 ignored, 0 failed;
- all 24 boot objects compiled with LCCC;
- real `setup.ld` linked with `lccc-ld` and failed only the expected 64-sector
  size ASSERT;
- diagnostic no-ASSERT links with LCCC and GNU bfd 2.47 agree
  section-for-section (`_end = 0x99c0` in both).

`build_kernel_boot.sh` now preserves the real failed-link status but follows a
size-ASSERT failure with a clearly marked, non-bootable diagnostic link. It
removes only the `setup_sects <= 64` and `_end <= 0x8000` guards, keeps every
other linker-script ASSERT, and reports exact decimal headroom/overflow. The
validated output is `_end=39360`, overflow **6592 bytes**.

## Follow-up (highest ROI)

1. Generalize short-lived i686 value materialization so SSA expression chains
   stay in GPRs instead of acquiring one stack home per intermediate. Current
   boot objects have about 2,075 stack references versus GCC's 382.
2. Add segment-aware allocation/splitting for non-call-spanning expression
   corridors, guarded by the m32, slot-RMW and regparm differential fuzzers.
3. Re-run the full boot link after every size change; the `.pecompat` 4096-byte
   alignment makes raw text deltas non-linear at boundary crossings.
4. QEMU boot validation remains impossible in the current VM until QEMU is
   installed and a complete bzImage builds; do not describe the boot code link
   as a boot test.
