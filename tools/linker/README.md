# Linker verification gates

Four tools, each of which produced a verdict that changed code in this series.
They live in the tree rather than in a scratch directory so a reviewer can
re-derive every claim in the commit messages instead of trusting them.

## `reloc_number_audit.py` — the relocation *number* gate

Checks every relocation constant in the tree against the authoritative source
(`/usr/include/elf.h` — the ABI headers binutils and the kernel are built from)
and reports any constant whose value disagrees with its name.

    python3 tools/linker/reloc_number_audit.py     # expect: 0 wrong

It found six on first run: five rotated AArch64 `R_AARCH64_TLSLE_MOVW_TPREL_*`
constants — whose `_NC` ("no check") suffixes made the resulting mis-encodings
silent, since the arm that should have rejected an out-of-range value was the
arm that got the wrong number — plus an i686 `R_386_TLS_LE_32` and a name,
`R_386_32S`, that does not exist in the i386 ABI at all. Run it after touching
any `R_*` constant.

## `oracle_cmp.py` — differential comparison against another linker

Links the same inputs with lccc-ld and with an oracle (GNU ld by default) and
reports divergences both in the accept/reject verdict and in the bytes written.

    python3 tools/linker/oracle_cmp.py --help

## `setup_oracles.sh` — build the corroborating linkers

mold and wild from git HEAD (never release tarballs: a stale tarball oracle
produced two false lccc failures earlier in this series), lld pinned to
release/23.x with `LLVM_TARGETS_TO_BUILD=X86`, all `-march=native`. Idempotent.
GNU ld is assumed present and is the *primary* oracle — every "matches GNU ld"
claim in this series was produced with it, because it is the only one guaranteed
to be installed. The others corroborate; they are not required.

    bash tools/linker/setup_oracles.sh

## `recover_env.sh` — restore the build environment

Toolchain paths, the 8 GiB swap file (this work runs on a 2-core, ~2 GB machine)
and the fastbuild preset. Run first in a fresh session.

    bash tools/linker/recover_env.sh

## The suites themselves

    # linker acceptance suite: 178 pass / 0 fail / 0 warn, differential vs bfd
    python3 tests/linker/run_linker_tests.py --lccc target/fastbuild/lccc-x86

    # unit tests, including the --defsym classifier and the field-width table
    cargo test --profile fastbuild --lib

    # the reloc tag alone: r_offset sweeps + per-field-width range verdicts
    python3 tests/linker/run_linker_tests.py --lccc target/fastbuild/lccc-x86 --tag reloc -v

Always build with the fastbuild preset (`scripts/build_lccc_fast.sh`); a debug
build of this tree does not fit in 2 GB.
