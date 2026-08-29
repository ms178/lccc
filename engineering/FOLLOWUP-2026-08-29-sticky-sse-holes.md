# Follow-up — 2026-08-29: sticky `-mno-sse` holes after #296

**Base.** `ms178/lccc` main `b416d21` (#299). Sticky `-mno-sse` vs
`-march=native` is already in #296 (`6f668f0`).

## Holes

1. **CPU profiles still set AVX bits.** `enable_x86_avx*_profile` only
   skipped `no_sse = false` when `sse_explicitly_disabled`.
   `-mno-sse -march=haswell` therefore still set `enable_avx2`.
2. **64-byte memcpy YMM path.** `emit_memcpy_impl_impl` selected
   `vmovdqu` on `avx2_enabled` without `!no_sse`. Combined with (1),
   `-mno-sse -march=haswell` could emit YMM while CR4.OSFXSR=0.
3. **Explicit `-mavx` after `-mno-sse`.** GCC last-ISA-flag wins; we
   now clear the sticky latch on `-mavx`/`-mavx2` (same as `-msse`).
4. **`-mgeneral-regs-only`.** Now latches sticky `-mno-sse` (kernel
   decompressor / realmode also pass this).

Integer BMI/LZCNT/MOVBE from v3 profiles remain enabled under
`-mno-sse` (not SSE).

## Tests

- `mno_sse_survives_march_haswell_without_avx`
- `mavx_reenable_after_mno_sse`
- `tests/regression/mno_sse_memcpy64.c` (freestanding `-mno-sse
  -march=haswell` must not emit xmm/ymm)

## QEMU path

`scripts/build_kernel_compressed.sh` wraps intact
`kernel-work/intact/vmlinux` with LCCC compressed objects + lccc-ld.
Do **not** `make bzImage` as a file target. Piggy is `zstd -6 --ultra`
+ 4-byte LE size. SIMD audit of `compressed/vmlinux` must show no
xmm/ymm.

`build_kernel_boot.sh` must pass `-DSVGA_MODE=NORMAL_VGA` when assembling
`header.S` (kbuild does). Without it `vid_mode` is ASK_VGA and setup
blocks 30s on a VGA prompt (`Press <ENTER> to see video modes`).

## QEMU result (this session)

Sticky-SSE holes are **not** the remaining ZSTD abort.

| wrap | QMP after ~8s |
|---|---|
| lccc `misc.o` -O2 / -O1 / -O0 | CS64 RIP ~`0x221xxxx` `hlt; jmp $-1`, RBX → `"ZSTD-compressed data is corrupt"` |
| **gcc** `misc.o` -O2, **all other objects lccc**, lccc-ld | RIP `ffffffff81efa85e` (uncompressed kernel VA). Decompress **succeeded**. |

Piggy is intact (`28 b5 2f fd`, `input_len=4127535`, `output_len=18638776`).
Userspace `zstd_preboot_oracle.sh` of a **decompress-only** TU still MATCHES;
the failing TU is full `misc.c` (extract_kernel + STATIC `__decompress`).
`-O0` still fails (undefined `accept_memory` stubbed), so this is **lowering /
codegen**, not `simplify`/`gvn`/`vectorize`.

Do not relink intact `vmlinux`. Next: O0 codegen of `decompress_kernel` /
`__decompress` in `misc.c` vs gcc (same flags, lccc-ld).
