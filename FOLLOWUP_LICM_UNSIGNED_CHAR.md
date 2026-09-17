# FOLLOWUP: LICM unsigned-char miscompile and aggregate_copy_forward OOB

## Summary

Linux-cachymod-6.18.52 QEMU boot failed with:

```
kobject: attempted to be registered with empty name!
...
BUG: kernel NULL pointer dereference in __device_attach+0x2d
```

Root cause is **two independent miscompiles** in lccc that are exposed by
`-funsigned-char` (kernel's default) and by heavy inlining in `mm/execmem.c`.

## 1. LICM miscompile in lib/vsprintf.c:number()

### Repro

```c
// /tmp/number_test.c — minimal repro of vsprintf number()
#include <stdio.h>
...
// build with -funsigned-char -O2 reproduces
/tmp/number_test: n=0 len=2 hex: 00 30 (should be 30, len=1)
// leading null + digit -> empty kobject name after %d formatting
```

- `/home/user/lccc/target/fastbuild/lccc -O2 -funsigned-char -c /tmp/number_test.c`
  => BUG `00 30`
- with `CCC_DISABLE_PASSES=licm` => OK `30`
- with licm-only (`CCC_DISABLE_PASSES=ccp,copy_prop,dead_code,dse,gvn,inline,inst_combine,loop_unroll,loop_peel,mem2reg,sccp,simplify_cfg,tail_call,loop_invariant` etc) => still BUG
- adding any call (printf) or store to global inside loops fixes because LICM
  then sees call/global-store and skips hoisting.

### IR evidence

LICM logs (instrumented fastbuild at `/home/user/lccc/target/fastbuild/lccc`):

```
LICM loop header 15 body {15} preheader Some(14)
=== DETAILED for header 27 ===
Body: {15}
Preheader: Some(14)
Block 15 IR:
  0: Phi num U64
  1: Phi i I32
  ...
  9: GlobalAddr hex_asc_upper Value(150)
  13: Cast locase U8->I32 Value(155) src Value(42)
  17: Cast shift I32->U64 Value(159) src Value(303)
  ...
Preheader Block 14 IR:
  0: GEP spec+1
  1: Load base U8
  2: Cast U8->I32
  3: Sub mask
  4: Cmp base==16
  5: Select shift 4/3
=== END DETAILED ===
```

After hoist, preheader gains GlobalAddr + Cast locase + Cast shift, loop loses them.
Loop body still uses Value(150) and Value(155) now defined in preheader that is
inside `base!=10` branch. `number()` has two distinct paths using
`hex_asc_upper`:

- small path `num<base` (base10): `tmp[i++]=hex_asc_upper[(u8)num & mask]|locase`
- base 8/16 loop: same but in loop

If GVN merges the two GlobalAddrs into one Value(150) that lives inside the
loop, hoisting it to preheader 14 (inside base!=10 branch) breaks dominance
for small path (base10) which is outside that branch. Small path then uses
undefined address -> null or wrong digit, producing `i=2` with `tmp[1]=0`
leading to empty kobject name.

With `-fsigned-char`, no hoisting occurs (6 loops, 0 hoistable). With
`-funsigned-char`, 3 insts hoisted, bug appears.

### Fix (temporary, unblocks boot)

In `src/passes/licm.rs`, disable LICM when plain char is unsigned:

```rust
if crate::common::types::char_is_unsigned() {
    return 0;
}
```

Proper fix: ensure hoisted defs dominate all uses and preheader dominates
all uses outside loop: collect all blocks using Value, ensure preheader
dominates them via dom tree, and if GlobalAddr has uses outside loop not
dominated by preheader, clone instead of move (create new Value in preheader
and replace only loop uses). Also guard Cast hoisting where src defined in
preheader but hoisted Cast inserted after src.

This fix alone makes `/tmp/number_test` pass and `lib/vsprintf.o` correct.

## 2. aggregate_copy_forward OOB in mm/execmem.c

```
thread '<unnamed>' panicked at src/passes/aggregate_copy_forward.rs:614:47:
removal index (is 8) should be < len (is 1)
```

Hoist logic picks `(bi, gep_idx, insert_at)` from `def_site` but does not
check bounds. With heavy inlining, `def_site` can be stale: block len 1 but
gep_idx 8. Also insertion index not adjusted for shift after removal.

### Fix

Guard bounds and adjust insertion:

```rust
if bi >= func.blocks.len() { return 0; }
let block_len = func.blocks[bi].instructions.len();
if gep_idx >= block_len || insert_at > block_len { return 0; }
let adjusted_insert = if insert_at > gep_idx { insert_at-1 } else { insert_at };
...
```

This fixes `mm/execmem.o` ICE.

## 3. Other codegen gaps blocking full kernel build

After fixing 1+2, full kernel build still hits:

- `block/bio.o: bio_iov_iter_get_pages` stale home value 297
  - `gvn` or `inline` alone fixes, both enabled triggers
  - `CCC_DISABLE_PASSES=gvn` or `inline` works

- `intel_gmch_probe` value 186 and `conntrack_mt` value 596
  - need `gvn,inline` disabled

- `nf_conntrack_proto_tcp.c: movdqu` SSE in file built with `-mno-sse`
  - `KCFLAGS=-Os` avoids 16-byte memcpy -> movdqu
  - proper fix: respect `-mno-sse` in memcpy lowering

- `__fortify_strlen, fortify_memset_chk` undefined with `CONFIG_FORTIFY_SOURCE=y`
  - disable `FORTIFY_SOURCE, FORTIFY_KUNIT_TEST, HARDENED_USERCOPY` etc.

### Workarounds in build_kernel_full.sh

```bash
./scripts/config \
  --disable FORTIFY_SOURCE --disable FORTIFY_KUNIT_TEST \
  --disable HARDENED_USERCOPY ... --disable KASAN ...

exec make ... KCFLAGS="-Os" ...
```

And for full build:

```
CCC_DISABLE_PASSES=gvn,inline   # or all passes
```

With `CCC_DISABLE_PASSES=gvn,inline` and `KCFLAGS=-Os`, kernel builds past
`block/bio.o` and `nf_conntrack_proto_tcp.o`, but still hits
`intel_gmch_probe` and `conntrack_mt`. With all passes disabled, it should
build fully (tested: passes `mm/execmem.o`, `block/bio.o`, `nf_conntrack`,
but needs more time).

## 4. Boot gate

`arch/x86/boot/setup.elf` ASSERT `_end <= 0x8000` still PASS with our fixes:

```
Total 28393, _end=28416, headroom=4352
```

## 5. Next steps

- Proper LICM dominance fix (clone vs move)
- Fix aggregate_copy_forward stale indices properly (rebuild def_site after each hoist)
- Fix RA stale home (gvn/inline interaction) — likely copy_prop creates value
  whose home is register that gets clobbered by call
- Respect `-mno-sse` in backend: don't emit `movdqu` for memcpy when SSE disabled
- Implement `__fortify_*` or make Kconfig disable it by default for lccc
- Re-enable gvn/inline after RA fix and re-measure vs gcc16.2/clang/icc/icx

## Artifacts

- `/home/user/lccc/target/fastbuild/lccc` instrumented logging LICM
- `/tmp/number_test.c` / `/tmp/test_licm_only` repros
- `/home/user/ms178-1.patch` S01 snapshot with licm+aggfwd fixes
- Kernel tree `/home/user/kernel-work/linux-6.18.52` with `.lccc-prepared`
- Build logs `/tmp/build_full.log`

## Validation

- `/tmp/number_test_fixed` passes for n=0..15
- `build_kernel_boot.sh` PASS
- QEMU boot with old bzImage still fails at `__device_attach+0x2d` (null kobject)
  because old objects built with buggy lccc remain. Full rebuild with
  `CCC_DISABLE_PASSES=gvn,inline` + `KCFLAGS=-Os` + FORTIFY disabled should
  produce bootable bzImage (in progress, process
  `kernel-build-gvn-inline-os-9ab41bb5` and `kernel-build-all-passes-disabled-9de26589`).
