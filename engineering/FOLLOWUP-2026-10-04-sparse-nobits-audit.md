# Follow-up: S22 sparse NOBITS — red-team audit, byte-size limit, alignment crashes

Session date: 2026-10-04.  Branch `pr747`, base `6c4a13cf` (upstream main),
snapshot tag `S22`.

This record is written for the next agent to pick the work up cold.  It states
what is proven, how it was proven, what is deliberately *not* done, and the
exact commands to reproduce every number quoted here.

## 0. What this revision contains

Six commits on top of `6c4a13cf`:

| commit | content |
|--------|---------|
| `8da859b0` | S21 content (rebased onto current main) |
| `f513b9f4` | sparse NOBITS tail, file-scope VM diagnostics, bounds guards, decay matrix test |
| `d418a1f4` | sparse NOBITS `.bss`: zero fills are a size, not bytes (`AsmItem::Zero(u64)`) |
| `77ad0c96` | array bounds: reject a byte size that cannot exist, never wrap it |
| `5c28b4e2` | assembler: NOBITS alignment is a size; GAS semantics for counts and padding |
| `30939d64` | gates: pin the byte-size limit and the sparse-NOBITS contract |

## 1. The feature, and how it is proven

`.bss` is written as NOBITS with an exact `sh_size`; the zero fills inside a
section are held as a size (`Section::zero_tail`) and never materialised.  The
proof is threefold and each part is reproducible:

    python3 /home/user/sparse-validate.py        # equivalence vs the pre-sparse compiler
    python3 scripts/check_array_bound_contract.py # 15 boundary/wrap/sparse cases
    bash tests/regression/check_sparse_bss_nobits.sh  # 21 runtime/parity probes

* **Semantic equivalence** over 160 regression sources against a compiler built
  from the pre-sparse commit `8da859b0`: 131 byte-identical objects, 24
  identical in every field that has meaning (section type/flags/size/alignment,
  PROGBITS contents by value, and every symbol), 1 refused by both, and 4 that
  only the current compiler can compile — each adjudicated *by GCC accepting the
  same file*, never by assumption.  The 24 layout-only cases are the deliberate
  change in §3; the check compares meaning, not file offsets, because a NOBITS
  section's `sh_offset` is not part of the object's meaning.

  Byte-identity alone is no longer the criterion, and that is a *stronger* claim
  than before, not a weaker one: it now also catches a difference in a section
  that merely moved in the file.

* **Memory profile**: `int a[N]` for N = 10^6, 10^8, 10^9, 4·10^9 compiles with
  the exact `.bss` (4·10^9 → 16 000 000 000) at 0 MiB peak RSS and 0.02 s wall.

* **Runtime**: a 4 GiB `.bss` array links into a 5 608-byte executable and
  exits with the right code; GCC agrees on the same program (`check_sparse_bss_nobits.sh`).

## 2. Defects found by the audit and fixed here

Every one of these was measured before it was touched; the oracle was GCC 14 /
binutils 2.47 in the same session.

1. **A byte size that wraps was silently accepted.** `CType::Array(elem, Some(n))`
   computed `elem.size * n` with a plain `usize` multiply.  `int a[2^62]` is
   2^64 bytes, which wrapped to **0**: the compiler accepted it and emitted a
   zero-byte `.bss` with no diagnostic. `int a[5000000000000000000]` wrapped to
   a bogus 1.5 EiB section; `int a[0x7fffffffffffffff]` wrapped to a negative
   value and leaked an *assembler* message (".zero 18446744073709551612") at the
   user.  Fixed by `MAX_OBJECT_BYTES` (PTRDIFF_MAX, GCC's own limit) tested in
   `u128` in `eval_array_bound_len`, plus saturating arithmetic everywhere else
   a (count, element size) pair is multiplied.  Boundary now agrees with GCC on
   every case: 2^61−1 ints and 2^63−1 chars accepted with the exact `.bss`;
   2^61 ints, 2^62 ints, 5e18 ints, 2^63−1 ints, and the nested/struct-element
   variants all rejected from the *source*, with no object written.

2. **`.section .bss` + `.p2align 40` aborted the process** ("memory allocation of
   1099511628408 bytes failed"). GAS is the opposite: it records
   `sh_addralign` = 2^40 and writes no bytes (rc 0, 608-byte object).  The gap
   was built as a dense `vec![fill; padding]` and then the object writer padded
   the *file* to align a NOBITS section's `sh_offset` — which the ELF spec does
   not require and GAS does not do.  Now: NOBITS padding is held as a size, the
   marker is still recorded (so `reconcile_section_alignments`, which derives
   `sh_addralign` from markers, is unaffected), the layout skips file-offset
   alignment for NOBITS, and every materialised padding run goes through
   `fill_run` (memory-checked, fallible).  `.bss` alignment 4/20/40 now matches
   GAS exactly; `.text` + `.p2align 40` is a *diagnostic*
   ("cannot write an object of 1099511628416 bytes …"), never a signal.

3. **A negative `.zero`/`.skip`/`.space` count was an error**, where GAS warns
   and *ignores* it ("repeat count is negative, ignored") with rc 0 — measured
   with `as -o` and with `gcc` driving it.  The parser now follows GAS, which
   needed a warnings channel: `parse_asm_with_warnings` (with `parse_asm` as the
   wrapper for tests), printed by `assemble` as `ccc: warning: …`.

4. **`sizeof(void)` was 0**, so `sizeof((void)0)` and `sizeof((0, (void)0))`
   disagreed with GCC (1) and with `_Alignof(void)` (already 1).  This is GCC's
   documented extension, and the three now agree.

5. **`_Alignas(2^40)` aborted the process**; GCC rejects it at the source
   ("requested alignment … exceeds maximum 268435456", `MAX_OFILE_ALIGNMENT`).
   The same cap and message class is now applied in sema, so the value never
   reaches the assembler.

6. **`.p2align 70` silently dropped the alignment** (`sh_addralign` = 1, no
   padding, no diagnostic) because the exponent clamp to 63 had no warning; the
   comment claimed one that did not exist.  The clamp now warns with GAS's text
   ("alignment too large: 63 assumed").

7. **`vzeroupper_after_ymm.c` used `alignas`**, which GCC and Clang reject in
   C17 without `<stdalign.h>`; the portable spelling in every mode is `_Alignas`
   (the test is ours, the compiler was right).

Not fixed, deliberately, with the reason:

* `alignas`/`alignof` are accepted as keywords in *every* `-std=` mode, where
  GCC needs C23 or `<stdalign.h>`.  With `<stdalign.h>` both compilers agree
  exactly (measured), so this is an accepts-more extension with no miscompile;
  CLANG is the outlier in the other direction (it refuses even with the header
  in gnu17).  Recorded here rather than "fixed" into a parser change whose blast
  radius is real: code that relies on the extension would stop compiling.

## 3. The one intentional change in object layout

A NOBITS section no longer gets its *file offset* aligned.  Consequence: objects
whose `.bss` alignment was previously honoured by inserting file padding shift
by up to the alignment — 24 of 160 regression objects differ from the pre-sparse
compiler in `sh_offset`/`e_shoff` only (same section sizes, same alignment, same
contents, same symbols).  For a large alignment this is the difference between a
700-byte and a multi-megabyte object.  GAS behaves the way the new code does.

## 4. Performance

* `scripts/oracle_delta_gate.py` (report `/home/user/results/oracle-delta-s22.json`)
  passes; ratios lccc/best-oracle: `gzip_crc32_loop` **0.93** (beats the oracle),
  `expat_name_length` 1.04, `ra01_global_match_probe` 1.60 (88 insns vs gcc 55 /
  icx 56), `expat_scan_document` 1.62, `zlib_ng_adler32_c` 2.04.
* **Known, diagnosed, not fixed**: `ra01_global_match_probe` rematerialises
  `leaq window(%rip)` / `leaq prev(%rip)` *inside* the loop where GCC and ICX
  hoist them.  The rematerialisable-global-address set is built by
  `build_rematerializable_global_addr_set_for` (`src/backend/generation.rs`),
  gated by `supports_global_addr_remat` (`src/backend/traits.rs`, x86 impl in
  `codegen/emit.rs`), with `CCC_NO_GLOBAL_ADDR_REMAT` as the escape.  This is a
  register-allocator *policy* tradeoff (remat saves a callee-saved register at
  the cost of one `leaq` per iteration); changing it belongs in its own commit
  with its own before/after measurement on the 14700KF, not in this one.
* `zlib_ng_adler32_c` (2.04) and `expat_scan_document` (1.62) are the next two
  items after that.
* A width-truncating self-move (`movl %r12d, %r12d`) survives because
  `machinst_emit.rs` elides self-moves only when the two `MachOperand`s are
  exactly equal; this one needs subregister liveness the emitter does not have.
  Do not force-elide it without that liveness.

## 5. Verification commands (all green on this revision)

    export PATH="$HOME/.cargo/bin:$PATH"
    scripts/build_lccc_fast.sh                              # fastbuild preset, -j2
    python3 /home/user/sparse-validate.py                   # SPARSE NOBITS VALIDATION PASS
    python3 scripts/check_array_bound_contract.py           # 15/15
    bash tests/regression/check_sparse_bss_nobits.sh        # 21 passed
    LCCC_BIN=target/fastbuild/lccc python3 tests/correctness/run_correctness.py   # 60 passed
    cargo fmt --all -- --check                              # clean
    python3 scripts/check_ci_gate_parity.py                 # PASS, 139 commands
    CI_LOCAL_JOBS=2 scripts/ci_local.sh --fast              # ALL GATES GREEN

Host prerequisites the CI installs and a wiped sandbox loses — without them the
i386 legs of `map-i64-two-lane` and `linker-suite` fail *for host reasons*, and
the reference compiler fails them identically (measured):

    sudo apt-get install -y --no-install-recommends gcc-multilib g++-multilib libc6-dev-i386

The linker suite pins this down: 282 pass / 19 fail without those packages,
302 pass / 0 fail with them.

Also required after a wipe: a swap file (1.9 GiB RAM, 2 cores), a pre-sparse
reference build at `/var/tmp/lccc-ref/ref-target/fastbuild/lccc` for
`sparse-validate.py` (worktree at `/var/tmp/lccc-ref/ref`), and the pinned
binutils oracle at `~/.cache/gas-2.47-x86_64-linux-gnu/bin` for `linker-suite`.
`/tmp` is a 993 MB tmpfs: the 713 MB reference build must not live there.

## 6. To-do for the next session

1. RA-01 global-address rematerialisation: hoist `leaq sym(%rip)` out of loops
   (measured 88 vs gcc 55 insns on `ra01_global_match_probe`), then re-measure
   `zlib_ng_adler32_c` (2.04) and `expat_scan_document` (1.62).
2. Add a warning channel of the same shape for the *code* generator's
   diagnostics?  Not needed yet; the parser sink is the only consumer.
3. Consider gating `alignas`/`alignof` on C23-or-`<stdalign.h>` (§2, deferred).
4. `-m32` object writer paths share `object_writer.rs`; the i386 legs are green
   with multilib installed, but a 32-bit *huge* `.bss` case is not covered by
   `check_sparse_bss_nobits.sh` (it would need an i386 linker and runner).
