# Follow-up — 2026-08-29: preboot ZSTD, rebase onto `90b6d87`

**Base.** `ms178/lccc` main `90b6d87` (PR #295 merge). Workspace was truncated
(no `.git`, no kernel tree); restored by cloning main, rustup 1.98.0, 8G
swap, `prepare_kernel_tree.sh` for linux-cachymod **6.18.47**.

## This session

### Sticky `-mno-sse` (still required)

`-march=native` / CPU profiles used to clear `no_sse`. Kernel decompressor
passes `-mno-sse` then Cachy `-march=native`; GCC keeps the disable.

Fix in `src/driver/cli.rs` + `pipeline.rs`: `sse_explicitly_disabled`.
`-mno-sse` is sticky against `-march=native` in either order; explicit
`-msse`/`-msse2` re-enables. Tests: `mno_sse_survives_march_native`,
`msse_reenable_after_mno_sse`. Runtime: `tests/regression/mno_sse_memcpy16`.

### P0 ZSTD on latest main: userspace oracle of **real `misc.o`** MATCHES

Previous sessions: standalone `__decompress` MATCH, combined LCCC `misc.o`
HLT `ZSTD-compressed data is corrupt` in QEMU; gcc `misc.o` booted.

On **`90b6d87` + sticky SSE**, the same `misc.c` TU compiled with compressed
flags (`-O2 -fPIE -mcmodel=small -mno-sse`) **decompresses**:

| payload | gcc `misc.o` | lccc `misc.o` |
|---|---|---|
| patterned 256 / 320 / 1K / 4K / 64K ± 4-byte size trailer | DECOMPRESS-OK | DECOMPRESS-OK |
| `vmlinux.bin` 16 539 648 B, zstd `-6 --ultra` + 4-byte trailer (4 129 387 B) | DECOMPRESS-OK | DECOMPRESS-OK |

Driver: `/tmp/zstd-oracle/misc_driver.c` (linked against `misc.o`; `error()`
longjmps; “Kernel is not a valid ELF file” after success is `parse_elf` on
non-ELF patterned data / stripped `vmlinux.bin` without PT_LOAD layout).
Do **not** use libc `malloc` — `misc.c` exports `MALLOC_VISIBLE malloc`.
Do **not** compile the string helpers at `-O2` without `-fno-builtin` (gcc
turns `memset` into a recursive call).

Likely already fixed upstream by **#294** (cast from-width: source type from
the defining instruction’s actual width, pr81588) and/or **#295** (narrow
compares / non-negative immediates). The earlier #291 unsigned-only Copy
restriction is in tree.

**QEMU boot is still the gate.** Userspace does not exercise PIE relocate of
the compressed stub, `extract_kernel`, IDT, or in-place overlap. Need a
bzImage (intact `vmlinux` + lccc compressed objects + lccc-ld). Tree was
truncated so `arch/x86/boot/compressed/*.o` and EFI stub / `startup/lib.a`
must be rebuilt; do **not** `make bzImage` as a file target with
`-mcmodel=kernel`.

## Do not re-open

Early IDT `.fill`; `-u`; `earlyprintk=`; `.Lcommon_startup_64`; blaming
standalone unzstd / `string.o` memcpy; `BOOT_HEAP_SIZE` as 4 MiB (x86 ZSTD
is `0x30000`); extra `\\\"` on `KBUILD_MODNAME` (must be `'"misc"'`).
