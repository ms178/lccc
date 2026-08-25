# Session 81 — i686 natural-width slots and semantic Copy widths

Date: 2026-08-25  
Initial upstream base: `65a0da93a67e4a1c4ab54bc73a25cb3459969ac5`  
Build: `scripts/build_lccc_fast.sh` (`fastbuild`, Rust 1.98.0, effective Rust `-O1`, `-j2`, clang + mold)  
Kernel corpus: patched linux-cachymod 6.18.44 from `ms178/archpkgbuilds`, all 26 PKGBUILD patches in package order, package config, `-m16 -Os -mregparm=3`

## Problem and root cause

The authentic x86 setup link still failed both unmodified `setup.ld` size gates. On the initial tree, LCCC emitted 27,954 bytes across the 20 real-mode C objects (GCC 14.2: 13,327), and `lccc-ld` measured `_end = 0x99c0`, 6,592 bytes beyond the hard `0x8000` limit.

One avoidable part of the gap was structural: every i686 scalar SSA spill occupied an 8-byte stack slot even though I8/I16/I32/U8/U16/U32/pointer/F32 values are accessed with at most one 32-bit word. In `number()` this produced a 304-byte frame and pushed frequently used slots across the disp8 boundary.

Simply enabling 4-byte slots was unsound. Phi elimination represents a U32 constant incoming value as an untyped `Copy` whose constant container is `IrConst::I64`. Two independent problems followed:

1. a one-pass allocation walk could encounter an untyped backedge Copy before its typed producer and assign it an 8-byte fallback slot, preventing same-width Copy coalescing;
2. i686 `emit_copy_value` treated every `IrConst::I64` container as a semantic 64-bit value and wrote a second zero word. With adjacent 4-byte slots this overwrote live state. The reduced mixed U32/U64 phi loop failed at O0/O2/O3/Os/Oz before the fix.

## Implemented fix

- Enable the existing exact-width small-slot infrastructure for `Target::I686` as well as x86-64.
- Before slot classification, infer i686 ≤4-byte values to a fixed point through untyped `Copy` webs. Typed seeds use `IrType::size() <= 4`, which correctly includes i686 pointers; narrow constants are seeds too.
- Keep exact 4/8/16/32-byte slot classes and all existing Tier-2/Tier-3/copy-alias width guards unchanged.
- Make semantic destination width authoritative for I64-container constants in the i686 Copy emitter. A second word is emitted only when the destination is known wide or the integer cannot fit one 32-bit word.
- Add the libc-free ELF32 runtime regression `i686_small_slot_copy_width.c`, covering adjacent U32/U64 loop-carried phi/Copy webs under `-m32 -Os -nostdlib -static`.

`CCC_NO_SMALL_SLOTS=1` remains the same-binary A/B control.

## Measured generated-code result

Identical patched-kernel source and flags:

| real-mode C corpus | bytes | delta vs control |
|---|---:|---:|
| 8-byte-slot control (`CCC_NO_SMALL_SLOTS=1`) | 27,619 | — |
| natural i686 slots | **26,896** | **-723 (-2.62%)** |
| original session baseline (before semantic-I64 fix too) | 27,954 | **-1,058 (-3.78%)** |
| GCC 14.2 oracle | 13,327 | — |

The semantic-I64 correction independently removes 335 bytes from the control build; it deletes high-word stores that were never part of U32 semantics. The setup link is materially smaller but this checkpoint does **not** claim that the 32 KiB gate is closed yet.

## Correctness and validation

- warning-free fastbuild with Rust/Cargo 1.98.0, `-O1 -j2`, clang 19 + mold 2.37.1;
- Rust library tests: **1,177 passed, 6 ignored, 0 failed**;
- correctness suite: **50/50**;
- regression corpus: **492 passed, 10 documented GCC compare skips, 0 failed**;
- full small-slot A/B regression harness: **464 passed, 5 GCC skips, 0 A/B divergences**;
- focused i686 runtime at O0/O2/O3/Os/Oz: pass; `CCC_NO_SMALL_SLOTS=1` control: pass;
- m32 differential fuzz: **900/900** (300 seeds × O0/O2/Os);
- regparm=3 differential fuzz: **450/450**;
- slot-RMW differential fuzz: **750/750**;
- i686 ALU torture: exact GCC checksum at O0/O2/Os.

The alias fuzzer's pre-existing scenario-3 crashes at O2/Os reproduce identically with `CCC_NO_SMALL_SLOTS=1`; they are not introduced by this change.

## Next work for the boot milestone

The authentic setup linker script is still binding. Continue with target-aware `-m16 -Os` pass-cost modelling and function-attributed assembly work; do not weaken either size ASSERT. Every subsequent candidate must retain the focused mixed-width regression and the m32/regparm differential gates above.
