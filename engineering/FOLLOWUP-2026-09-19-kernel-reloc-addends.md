# Follow-up 2026-09-19 — x86 relocation addends and CachyMod QEMU boot

Base: upstream `main` at `407eb4927889e9b1b30426c5622e8d07067c99c5` (PR #557 merged).
Target: patched `linux-cachymod-6.18.52`; compiler and target linker are exclusively
`target/fastbuild/lccc` and `target/fastbuild/lccc-ld`. Host-only Kbuild tools use
the system toolchain, as documented by `build_kernel_vm.sh`.

## Result

The current tree builds the boot setup, the complete minimal CachyMod VM kernel,
and boots two SMP CPUs under QEMU/TCG. The in-guest suite reaches clean poweroff
and passes all 16 feature, scheduler, networking, SMP, and serial-integrity gates.
The setup image remains below Linux's hard 32 KiB boundary.

| Gate | Result | Evidence |
|---|---:|---|
| Swap | PASS | 4 GiB `/swapfile`, active throughout builds |
| LCCC build policy | PASS | `fastbuild`, Rust `opt-level=1`, no LTO, `-j2` |
| Package patch stack | PASS | 28/28 patches applied to Linux 6.18.52 |
| Package-config boot setup | PASS | `_end=31168`, 1600 B headroom; flat image byte-identical to `ld.bfd` |
| Post-fix minimal-config setup | PASS | `_end=28416`, 4352 B headroom; flat image byte-identical to `ld.bfd` |
| Clean full VM kernel build | PASS | 5,071,872 B `bzImage`; 20,577,712 B `vmlinux`; setup `_end=29856` |
| QEMU boot | PASS | Linux banner identifies lccc; 2 CPUs; all 16 validation checks pass |

The post-fix full build was clean (`make ARCH=x86_64 clean`) rather than an
incremental relink, so every target C/assembly translation unit was regenerated
by the fixed LCCC. No target object was substituted from GCC or Clang.

## Correctness defects fixed

### 1. Bare absolute memory displacement lost its addend

The x86 parser must leave a bare operand such as `ext+9` as `Operand::Label`:
without the instruction mnemonic it cannot know whether the expression is a
control-flow target or an absolute memory operand. Memory instruction dispatch
later synthesized `Displacement::Symbol("ext+9")`. The normal parenthesized
memory parser, in contrast, produced `SymbolPlusOffset("ext", 9)`.

The synthesized path eventually called `add_relocation`, but the relocation
choke point preserved the literal spelling. The object therefore contained an
undefined symbol *named* `ext+9`, not symbol `ext` plus addend 9. This affected
loads, stores, and other bare-memory operations (the reproducer includes `bt`).

Fix: `add_relocation` now uses the parser's one canonical
`split_relocation_symbol_addend` grammar. It combines the source addend exactly
once with the architecture-supplied addend, then performs the existing `@PLT`
normalization. Overflow is never wrapped into a different address.

### 2. `loop` / `jrcxz` addend target escaped as an undefined internal reloc

The same parser ambiguity affected short-only branches such as
`loop .Lagain+1`. The encoder recorded an internal `R_X86_64_PC8_INTERNAL`
relocation against the literal undefined symbol `.Lagain+1`. Because internal
resolution could not find that name, the private relocation type escaped into
the ELF object and the displacement remained zero.

Normalizing at the common relocation boundary turns the target into `.Lagain`
and combines `+1` with the PC8 base addend `-1` exactly once. The same-section
resolver then emits the GAS-identical bytes (`e2 ff`, `e3 fd`) and no PC8
relocation remains in the object.

## Regression and oracle

`tests/asm-diff/reloc-addend.casefile` is a whole-object GNU-as differential,
not merely a disassembly check. It pins:

- positive and negative bare-memory addends;
- both load and store directions;
- 32- and 64-bit operations;
- a non-mov memory operation (`btq`);
- internal PC8 `loop` and `jrcxz` targets with addends.

Before the fix the one case reported 7 extra LCCC relocations, five bogus
undefined symbols (`ext+9`, `ext-7`, ...), two escaped private PC8 relocations,
and wrong branch bytes. After the fix:

```text
=== asm-diff: 1 passed, 0 failed (1 cases, oracle=/usr/bin/as) ===
```

The parser unit test additionally pins whitespace, `constant+symbol`, local
labels, plain symbols, and a non-constant symbol difference.

## Reproduction commands

```bash
./scripts/ensure_swap.sh
./scripts/build_lccc_fast.sh
./scripts/prepare_kernel_tree.sh
OUT=/home/user/bootbuild ./scripts/build_kernel_boot.sh
python3 scripts/asmdiff.py --lccc target/fastbuild/lccc \
  --as /usr/bin/as tests/asm-diff/reloc-addend.casefile -v

make -C /home/user/kernel-work/linux-6.18.52 ARCH=x86_64 clean
BUILD_LOG=/home/user/kernel-vm-build-fixed.log JOBS=2 \
  ./scripts/build_kernel_vm.sh
BOOT_LOG=/home/user/qemu-boot-fixed.log ./scripts/qemu_boot_test.sh
./scripts/ci_local.sh --fast
```

## Follow-up work

1. Add a 32-bit counterpart to the whole-object addend corpus even though the
   i686 encoder already normalizes bare addresses through `split_label_offset`.
2. Make the kernel tool-identity stamp invalidate compile products as well as
   link products when the compiler hash changes; this session explicitly
   cleaned the tree to avoid relying on Kbuild command-line timestamps.
3. Run the package's broad desktop config on a host with sufficient persisted
   disk/time; the QEMU milestone intentionally uses the audited minimal config
   while retaining CachyMod's BORE, cache scheduler, HZ=800, CACHY, BBRv3,
   preemption, SMP, ACPI, PCI, and serial paths.
4. Continue CPU hotplug stress beyond the two-CPU boot gate and preserve serial
   logs as compact CI artifacts.

## Default-config follow-up: pointer-table initializer extent

GDB proved the apparent `delay_loop` hang was the panic delay, not the original
fault.  The first exception was a NULL+0xa0 dereference in
`default_acpi_madt_oem_check()`: `.apicdrivers` was 0x2a0 bytes instead of the
required three 8-byte pointer slots.  Each input object declared an 8-byte
symbol but LCCC appended 0xd8 zero bytes, because global brace initialization
passed the pointee `struct apic` layout while lowering a pointer object.

`lower_declarator_init()` now suppresses pointee layouts for scalar pointer
objects.  The focused regression
`global_braced_pointer_to_struct_section_extent.c` checks both the exact
three-slot section extent and pointer contents against GCC.  The regression
passes, and recompiling the real x2APIC objects shrinks every `.apicdrivers`
input section from 0xe0 to 8 bytes.  An incremental real-objtool default-config
`bzImage` rebuild completed successfully and now reaches the serial kernel
banner and initcalls.

A later, separate default-config failure remains: during `scsi_init_devinfo`,
entry 28 of `scsi_static_device_list` has its model pointer changed in guest
memory from the linked `ffffffff82d5858c` to `ffffffff00000000`, causing a
fault in `strlen`.  The stripped and unstripped ELF files both contain the
correct pointer and `R_X86_64_64` relocation at `ffffffff8382b558`; GDB confirms
the corruption is already present in the table at fault time.  This is now the
next default-config root-cause target and is distinct from the fixed
pointer-table extent bug.

A hardware GDB watchpoint subsequently identified the writer: at
`llc_populate_cpu_shard_id+0x2d9`, the generated store for
`cpu_shard_id[c] = shard_id` computes `c << 16` rather than `c * 4`.  For
`c == 1`, that changes the unrelated SCSI pointer exactly 64 KiB above
`cpu_shard_id` to `ffffffff00000000`.  LCCC's declaration metadata at lowering
time is correct (`elem_size=4`, `strides=[4]`, `CType::Array(Int, 64)`), and the
load of the same expression uses a scale-4 address.  The remaining defect is
therefore downstream in O2 IR optimization/instruction selection/register
allocation of the lvalue store, not relocation or initializer lowering.
