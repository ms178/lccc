# Follow-up — 2026-08-29: CachyMod kernel 6.18.47

**Task.** Rebase the in-tree kernel harness from linux-cachymod **6.18.46**
to **6.18.47** (`archpkgbuilds` commit `a494627`, pkgver 6.18.47, pkgrel 2.1),
build with lccc + lccc-ld, QEMU-boot (no `earlyprintk=`), then catalogue
codegen/perf leftovers.

**Why.** Upstream CachyMod and kernel.org both moved to 6.18.47 (tarball
`linux-6.18.47.tar.xz`, 2026-08-27). The 26-patch series in
`packages/linux-cachymod-6.18` is unchanged in *names*; the tree must be
regenerated because the Arena snapshot truncates ~55k-file kernel trees.

**Closed (do not re-open).** Early IDT stub `.fill` over-pad (PR #289 /
`FOLLOWUP-2026-08-29-early-idt-fill.md`). x86-64 `-T` `-u` (PR #287).
CPUID/FPU userspace. `.Lcommon_startup_64`. `earlyprintk=` livelock.

**Script defaults now 6.18.47.** `prepare_kernel_tree.sh`,
`arena_session_restore.sh`, `build_kernel_{boot,full,vm}.sh`,
`qemu_boot_test.sh`, `realmode_corpus.sh`. PKGDIR still
`/home/user/archpkgbuilds/packages/linux-cachymod-6.18`.

**Boot gate.** Userspace reaching init. Append
`console=ttyS0,115200 nokaslr panic=-1` only.

---

## Results this session (lccc `7b057b2` + 6.18.47 bump)

| Gate | Status |
|---|---|
| Tree regenerate, 26/26 CachyMod patches | **PASS** |
| Full bzImage, lccc + lccc-ld, `-j2` | **PASS** in 1348 s |
| 32 KiB setup `_end` | **PASS** `0x7600` = 30208 B |
| IDT `early_idt_handler_array` | **PASS** 32×9 B (IBT off); vec9 no longer over-pads |
| QEMU userspace / init | **FAIL** — compressed stub `error()` |

```
bzImage   4322304 B
vmlinux  20017192 B   text 11063662  data 2612480  bss 2498560
sha256    13f97a456ffe4b811e9eb1e0798a98ddbb9724b38f7f118905bdb968351ac2aa
```

### IDT (verified, not the hang)

`EARLY_IDT_HANDLER_SIZE = 9` (`CONFIG_X86_KERNEL_IBT` is not set).
`early_idt_handler_array` … `early_idt_handler_common` span 288 = 32×9.
Vec8/10/… errcode stubs: 7-byte body + 2×`cc`. Vec9: exact 9-byte body.
Later slots use rel8 `jmp` + 3×`cc`. PR #289 restretch is doing its job.

### Hang (new P0)

QEMU `-nographic` shows SeaBIOS then silence (VGA-only putstr). QMP after 8 s:

```
RIP=0x221bd73  HLT=1  CS64  CR0=80050033  CR3=0x225a000
code: f4 eb fd     hlt; jmp $-1
```

Bytes immediately before RIP match `arch/x86/boot/compressed/error.o`
(`lea r11, msg; mov rdi,r11; call warn/putstr; hlt`). Guest memory at
`RBX=0x221f515`:

```
"ZSTD-compressed data is corrupt"
```

That string is `handle_zstd_error()` in `lib/decompress_unzstd.c` for
`ZSTD_error_dstSize_tooSmall | corruption_detected | checksum_wrong`.

The on-disk payload is **not** corrupt:

- `vmlinux.bin.zst` magic `28 b5 2f fd`, `z_input_len = 4127531`
- `input_data` … `input_data_end` in `compressed/vmlinux` matches that length
- host `zstd` CLI reports “unsupported format” only because
  `cmd_zstd22_with_size` appends an 8-byte size trailer (`scripts/Makefile.lib`)
- `file(1)` still classifies it as Zstandard

So the **lccc-built preboot ZSTD decoder** (one TU: `compressed/misc.c`
`#include`s `lib/decompress_unzstd.c` → `lib/zstd/decompress_sources.h`)
rejects a host-zstd `-6 --ultra` frame that the kernel compressor produced.

Do **not** treat this as FPU/CPUID. `error()` here is the *compressed*
stub, identity-mapped around 34 MiB, not `fpu__init_system` in `.init.text`.

## Next (ordered)

1. **Userspace oracle of the preboot TU.** Compile `decompress_unzstd.c` with
   the compressed-boot flags (`-O2 -fno-strict-aliasing -fPIE -ffreestanding
   -fno-jump-tables -mcmodel=small -mno-red-zone`) under lccc vs gcc, feed
   `vmlinux.bin.zst` (with and without the size trailer). Pin the first
   failing helper (`zstd_find_frame_compressed_size` vs
   `zstd_decompress_dctx` vs xxhash checksum).
2. **Do not** `make CC=gcc bzImage` on the lccc tree — `.cmd` fingerprints
   rebuild the whole kernel. Rebuild only `arch/x86/boot/compressed/` or
   use a copy of the tree.
3. Once the guest reaches `start_kernel`, size vs GCC (`vmlinux` is still
   ~+29 % historically) and the four leftover objtool warnings.
4. In-VM icount (`scripts/in_vm_codegen_bench.sh`) after init.

**Perf after boot.** Size vs GCC on vmlinux/setup; objtool leftover
warnings; in-VM icount if the guest reaches init.

---

## P0 fix: preboot ZSTD miscompile (`simplify_cast` signed Copy)

**Symptom.** Intact host-zstd payload (`vmlinux.bin.zst` + 8-byte size trailer)
is rejected by the lccc-built preboot decoder: `error("ZSTD-compressed data
is corrupt")` then `hlt; jmp $-1`. Userspace oracle of the same TU
(`arch/x86/boot/compressed/preboot_oracle.c` + `/tmp/zstd-oracle/driver.c`):

| payload | gcc -O2 | lccc -O0 | lccc -O1/-O2 |
|---|---|---|---|
| `i%256`, 256 B | MATCH | MATCH | MATCH |
| `i%256`, ≥320 B / zeros / text | MATCH | MATCH | **SEGV** |
| random 1024 B | MATCH | MATCH | MATCH |

Isolated `ZSTD_copy16` / wildcopy is **not** the crash. `-fPIE`/`-mcmodel=small`
are not the crash (O0 MATCH with the same flags).

**Bisection.** O1 `CCC_DISABLE_PASSES=simplify` MATCH. Fine flags
`CCC_SIMPLIFY_SKIP=cast_same` MATCH; skip-bittest/reassoc/cast_ident still FAIL.

**Root cause.** `simplify_cast` folded

```
Cast(Cast(x, A→B), B→A)  =>  Copy(x)
```

for **signed** A (I32→I64→I32). x86 I32 homes are frequently zext (`movl`);
I32 narrow is `movslq` (sext). Copy drops that canonicalization; a later
64-bit use of the "I32" (pointer += length, C promotion) sees garbage high
bits. Match/RLE paths (≥320 B patterned) walk off into unmapped memory.

Unsigned Copy is sound (already zext-canonical) and is kept. Mixed B-widest
`Cast(A→B→C) => Cast(A→C)` now requires C ≤ A **or** A/C same signedness,
and never folds A→B→A (that is exclusively `cast_same`).

**Fix.** `src/passes/simplify.rs`: restrict widen-narrow-back Copy to
`inner_from.is_unsigned()`. Tests:
`test_cast_chain_widen_narrow_{unsigned,signed_not_copy}`,
`tests/regression/signed_i32_ptr_add.c`.

Do **not** relink the 6.18.47 `vmlinux` (966 gcc `*.o` newer than the
08:29 lccc link). Rebuild only `arch/x86/boot/compressed/` then `bzImage`.
