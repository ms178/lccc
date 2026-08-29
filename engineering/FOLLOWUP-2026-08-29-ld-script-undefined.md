# Follow-up — 2026-08-29: `-u` on script-driven links

**Symptom.** After `vmlinux` linked, `arch/x86/boot/header.o` failed:

```
ccc: error: assembler error: "32-bit and 64-bit EFI entry points do not match"
```

Generated `arch/x86/boot/zoffset.h` had no `ZO_efi32_stub_entry` /
`ZO_efi64_stub_entry`. Compressed `vmlinux` `nm` showed `startup_32/64` and
`efi_get_*` but no stub entries. `efi-mixed.o` defines
`efi32_stub_entry@0` and `efi64_stub_entry@0x200` (the `.if` in
`header.S:533` would pass if those symbols were in the image).

**Root cause.** Linux links the decompressor as

```
lccc-ld -m elf_x86_64 -pie ... -u efi_pe_entry -T compressed/vmlinux.lds \
    ... libstub/lib.a startup/lib.a
```

`lccc-ld` collected `-u` only into `passthrough` (`-Wl,-u,efi_pe_entry`) for
the userspace `link_builtin` path. The `-T` path never reads `passthrough`,
so `-u` was a no-op. Archive members were scanned against an empty undefined
set; `x86-stub.stub.o` (`T efi_pe_entry`) and then `efi-mixed.o`
(`D efi_is64`, the two stub entries) never came in.

**Fix.** `load_inputs_for_ld` now takes the `-u`/`--undefined` name list and
seeds those names as undefined globals *before* the first archive scan.
`lccc-ld` collects `-u SYM`, `--undefined SYM`, and `--undefined=SYM` into
that list for both `-T` and `-r` loads.

**Regression.** `tests/linker/run_linker_tests.py` case
`script_undefined_pulls_archive` (filter `undefined` / tag `script`).

**Not this bug.** The assembler `.error` is not an assembler defect. Do not
re-debug ICE/GS (`fpu__init_system` GS prefix is already in objects). Do not
wipe the kernel tree.

**Next after this lands.** Relink compressed `vmlinux` with the new
`lccc-ld`, regenerate `zoffset.h`, assemble `header.o`, finish `bzImage`.
Then 32 KiB `_end` gate and `qemu_boot_test.sh`. Remaining boot risk is
CPUID/FPU (`f4 eb fd` in `fpu__init_system` if `boot_cpu_has(FPU)` is false).
