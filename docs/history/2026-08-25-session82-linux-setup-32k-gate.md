# Session 82 — linux-cachymod x86 setup: authentic 32 KiB gate closed

Date: 2026-08-25  
Upstream base at session start: `65a0da93a67e4a1c4ab54bc73a25cb3459969ac5`  
First validated checkpoint: `effd194deb66c9a01f57f4ff18686c0e074335b7` (session 81 natural i686 slots)  
Compiler build: `scripts/build_lccc_fast.sh`, Rust/Cargo 1.98.0, Rust `-O1`, `-j2`, clang 19 + mold 2.37.1  
Memory discipline: 1.9 GiB RAM + active 8 GiB swap (`/swapfile-lccc`)  
Kernel package source: `ms178/archpkgbuilds` `6e6b9a3773142917938ae323d9e771ba83cdf3c7`

## Result

The patched linux-cachymod 6.18.44 real-mode setup code now compiles entirely with LCCC, links with standalone `lccc-ld`, satisfies the **unmodified numerical constraints** in Linux `arch/x86/boot/setup.ld`, and produces the same flat setup image as GNU BFD and LLD.

Final authoritative layout:

```text
section        bytes   address
.bstext          495   0x0000
.header          125   0x01ef
.entrytext       103   0x026c
.inittext        386   0x02d3
.initdata         30   0x0455
.text          23065   0x0473
.text32           30   0x5e8c
.pecompat          9   0x6000
.rodata         1374   0x6010
.videocards       84   0x6570
.data            140   0x65d0
.signature         4   0x665c
.bss            4960   0x6660
_end                   0x79c0 = 31,168
```

Hard gate: `_end <= 0x8000` **PASS**, with **1,600 bytes headroom**.  
`setup_size = 0x7000`, `setup_sects = 56` (limit 64) **PASS**.  
Flat `setup.bin`: 26,208 bytes, SHA-256
`0df65ec1ef998fe8367645ffb8b520ad6e89d005a567d8876fa7ac446235fb04`.

The LCCC-linked flat image is byte-for-byte identical to both:

- GNU `ld.bfd --gc-sections -m elf_i386`;
- LLVM `ld.lld --gc-sections -m elf_i386`.

This is stronger than comparing section sizes: relocations, linker-script data commands, addresses, padding, the setup signature, and every emitted byte agree.

## Kernel source and configuration provenance

`scripts/prepare_kernel_tree.sh` regenerated the tree from the signed kernel.org 6.18.44 tarball, then applied all **26/26** patches in the PKGBUILD's order. Relevant package/custom functionality is present and selected:

- `CONFIG_CACHY=y`;
- `CONFIG_SCHED_BORE=y` and patched `kernel/sched/bore.c`;
- cache-aware scheduler code in `kernel/sched/topology.c`;
- `CONFIG_X86_NATIVE_CPU=y` (the package's Raptor Lake/native path);
- `CONFIG_R8169=m` with `CONFIG_PAGE_POOL=y` (the custom multi-queue dependency);
- `CONFIG_EFI_MIXED=y`;
- EDD and video setup support are present in the boot image.

The build-local linker script preserves `.videocards` (84 bytes) and EFI mixed-mode `.pecompat` (9 bytes), so section GC does not obtain its win by deleting the requested boot functionality.

## Root-cause chain and fixes

### 1. Natural-width i686 scalar slots

Session 81 enabled exact 4-byte spill slots for i686 scalar/pointer/F32 values, added fixed-point width inference through untyped Copy/phi webs, and fixed the semantic-width bug where a U32 constant carried in `IrConst::I64` wrote a destructive second word. This reduced the real-mode C corpus from 27,954 to 26,896 bytes and is separately fuzz/regression validated in `2026-08-25-session81-i686-natural-slots.md`.

### 2. Debug metadata no longer disables i686 peepholes

The kernel enables `-g`; `.loc` directives sat between a spill and reload and were classified as hard barriers. The generated machine instructions were therefore worse with debug line tables enabled. `.loc` is now transparent to bounded local data-flow scans while remaining in the assembly output. Section/alignment/CFI directives remain barriers. A unit regression proves that the spill is removed while `.loc` survives.

### 3. Measured `-m16 -Os/-Oz` phase policy

On this six-GPR accumulator backend, post-structural inlining, if-conversion, global-address CSE, and LICM enlarge the final real-mode image through merged live ranges and extra spills. A same-revision A/B measured **26,657 bytes** with the full pipeline versus **25,004 bytes** with the code16 size policy: **-1,653 bytes (-6.20%)**. They remain enabled for ordinary `-m32` and performance-oriented optimization levels. Only explicit `.code16gcc` size builds disable the four measured losers; `CCC_M16_FULL_PIPELINE=1` is the A/B override.

This is a target/goal cost model, not a claim that the optimizations are generally bad.

### 4. Real ELF32 linker-script section GC

`lccc-ld` previously accepted `--gc-sections` on `-T` script links and silently ignored it. The script path now:

1. parses `ENTRY` and every `KEEP` input specification;
2. seeds the shared relocation-reachability collector with those symbol and section roots;
3. follows local and global ELF32 REL targets transitively;
4. omits dead input sections from symbol resolution and placement; and
5. retains existing non-ALLOC/debug behaviour and no-GC layout unchanged.

The feature is implemented in the address-width-neutral script engine, so x86-64 script links gain the same semantics. A new ELF32 differential test proves ENTRY reachability, transitive call reachability, KEEP registry reachability, dead function removal, and exact flat-binary parity with BFD.

### 5. Safe Linux integration

`build_kernel_boot.sh` now compiles with `-ffunction-sections` and links with `lccc-ld --gc-sections`. It derives `setup-gc.ld` from the kernel script and adds `KEEP` only around protocol payloads/registries whose existence is communicated by layout rather than a relocation:

```text
.bstext .header .entrytext .inittext .initdata .text32 .pecompat .videocards
```

No address, alignment, data command, section order, or ASSERT is changed. The kernel source tree stays pristine. The script fails if `.videocards` or configured `.pecompat` disappears, then independently links with available BFD/LLD and requires byte-identical flat images.

## Performance/size evidence

| Checkpoint | setup/C-object metric | Result |
|---|---:|---:|
| Initial authentic link | `_end` | `0x99c0` (6,592 B over) |
| Initial real-mode C corpus | text | 27,954 B |
| Natural slots + semantic Copy width | text | 26,896 B |
| Final default `-m16 -Os` object corpus | text before link GC | 25,004 B |
| Same function-section objects, link GC disabled | setup `.text` / `_end` | 25,123 B / `0x89c0` (2,496 B over) |
| Final reachable setup `.text` after GC | text | **23,065 B (-2,058 B)** |
| Final authentic link | `_end` | **0x79c0** (1,600 B headroom) |

Object sums deliberately include dead externally visible string helpers and are therefore not the linker gate. Reachable linked sections are the authoritative artifact once `-ffunction-sections --gc-sections` is enabled.

## Validation

Completed gates:

- warning-free fastbuild (`-O1`, `-j2`, active 8 GiB swap);
- Rust library tests: **1,179 passed, 6 ignored, 0 failed**;
- correctness suite: **50/50**;
- main regression corpus: **492 passed, 10 documented GCC compare skips, 0 failed**;
- small-slot treatment/control suite: **464 passed, 5 GCC skips, 0 A/B divergences**;
- complete linker oracle suite: **161 passed, 0 failed/warned/skipped**;
- `-g` m32 differential fuzz: **900/900** (300 seeds × O0/O2/Os);
- `-g -mregparm=3` differential fuzz: **450/450**;
- `-g` slot-RMW differential fuzz: **750/750**;
- authentic `build_kernel_boot.sh` link with LCCC + lccc-ld;
- both original Linux size ASSERTs;
- non-empty `.videocards` and configured `.pecompat` payload checks;
- exact BFD and LLD flat-image differential;
- focused ELF32 script-GC/KEEP linker regression;
- focused `.loc`-transparent peephole and code16-policy unit regressions.

## Honest scope / non-claim

This task explicitly limited the build to the kernel's boot/setup code. The generated `zoffset.h`/`voffset.h` values are documented stubs because a real value requires `compressed/vmlinux`; therefore this checkpoint does **not** claim a QEMU or hardware boot of a complete bzImage. It proves that the exact current blocker—the real-mode setup compiler/linker pipeline and 32 KiB layout—has been removed without deleting EFI/video registries.

A direct full-Kbuild `prepare` with LCCC also exposed the next independent blocker: this package config enables `-pg`, while LCCC intentionally diagnoses that it does not yet emit `mcount`/`__fentry__` instrumentation. Do not silence or accept-and-ignore `-pg`; either implement the ABI correctly or make the LCCC kernel config disable function tracing. This does not affect the isolated real-mode setup build, whose authentic flags do not require profiling instrumentation.

## Follow-up priority

1. Replace stub z/voffsets with a real compressed kernel build, produce bzImage, and run a serial-console QEMU smoke test with a deterministic initramfs.
2. Implement correct x86 `-pg`/`-mfentry` kernel instrumentation (including objtool/ftrace expectations) before claiming a full package build; retain the current hard diagnostic until then.
3. Add an opt-in full-Kbuild integration wrapper that applies the same function-section/KEEP/GC contract through Kbuild variables rather than the focused setup harness.
4. Re-run setup after every i686/code16 change. Reject any loss of the 1,600-byte safety margin unless a measured correctness/performance requirement justifies it.
5. Continue generated-code work on `printf`, `string`, and video helpers. GC solves the hard layout limit but does not erase the remaining 1.88x pre-GC C-text gap to GCC.
6. On the real i7-14700KF, measure boot/setup execution only where a stable workload exists; static size and oracle identity are appropriate evidence for this hard-cap milestone, not a runtime-speed claim.
