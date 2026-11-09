#!/usr/bin/env bash
# Source from the repository root. Restores the pinned harness-cache-8
# toolchain, cached Debian runtime packages, Python ELF/DWARF packages and
# the strict (-D warnings) fastbuild environment. Never touches the network.
set -euo pipefail
LCCC_AGENT_CACHE_PREFIX="${LCCC_AGENT_CACHE_PREFIX:-/home/user/.cache/lccc/agent-tools}"
if [[ ! -f "$LCCC_AGENT_CACHE_PREFIX/env.sh" ]]; then
  echo "agent-env.sh: missing $LCCC_AGENT_CACHE_PREFIX/env.sh" >&2
  return 1 2>/dev/null || exit 1
fi
# shellcheck source=/dev/null
source "$LCCC_AGENT_CACHE_PREFIX/env.sh"
export PYTHONPATH="$LCCC_AGENT_CACHE_PREFIX/pyyaml-root:$LCCC_AGENT_CACHE_PREFIX/debs-root/usr/lib/python3/dist-packages${PYTHONPATH:+:$PYTHONPATH}"
export VALGRIND_LIB="$LCCC_AGENT_CACHE_PREFIX/debs-root/usr/libexec/valgrind"
export PATH="$LCCC_AGENT_CACHE_PREFIX/debs-root/usr/bin:$LCCC_AGENT_CACHE_PREFIX/qemu-user-11.1.3-x86_64/bin:$PATH"
export LD_LIBRARY_PATH="$LCCC_AGENT_CACHE_PREFIX/debs-root/usr/lib/x86_64-linux-gnu:$LCCC_AGENT_CACHE_PREFIX/debs-root/usr/lib32:$LCCC_AGENT_CACHE_PREFIX/debs-root/lib32${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
# The cached binutils 2.47 `ld` has no Debian multiarch SEARCH_DIR.  GCC
# honours LIBRARY_PATH; LCCC's built-in linker uses its own absolute paths,
# which prepare_local_env.sh reproduces with verified symlinks.
export LIBRARY_PATH="/usr/lib/x86_64-linux-gnu:/usr/lib/i386-linux-gnu:/usr/lib32:/lib32:/usr/lib/gcc/i686-linux-gnu/12/32${LIBRARY_PATH:+:$LIBRARY_PATH}"
# GCC reads these; LCCC deliberately does NOT read C_INCLUDE_PATH (the
# sysroot contract).  The -I-free gates use prepare_local_env.sh's symlinks.
export C_INCLUDE_PATH="$LCCC_AGENT_CACHE_PREFIX/debs-root/usr/include:/usr/include/x86_64-linux-gnu${C_INCLUDE_PATH:+:$C_INCLUDE_PATH}"
export CPLUS_INCLUDE_PATH="$C_INCLUDE_PATH${CPLUS_INCLUDE_PATH:+:$CPLUS_INCLUDE_PATH}"
export RUNNER_TEMP="${RUNNER_TEMP:-/tmp/lccc-runner-temp}"
mkdir -p "$RUNNER_TEMP"
export CARGO_BUILD_JOBS=1
# Reuse an already-active EXACTLY 4 GiB file instead of allocating a second
# swap image and exhausting this 21 GiB sandbox. /proc/swaps reports usable
# pages (slightly below stat's byte count), so verify the file size itself.
# With no qualifying active file, keep the sandbox-local fallback path.
LCCC_SWAPFILE="$HOME/.cache/lccc-swap.img"
if [[ -r /proc/swaps ]]; then
    while read -r active_path active_type _; do
        [[ $active_type == file && -f $active_path ]] || continue
        if [[ $(stat -c %s "$active_path" 2>/dev/null || echo 0) -eq $((4 * 1024 * 1024 * 1024)) ]]; then
            LCCC_SWAPFILE=$active_path
            break
        fi
    done </proc/swaps
fi
export LCCC_SWAPFILE
export RUSTFLAGS="-D warnings"
unset RUSTUP_TOOLCHAIN RUSTUP_HOME CCC_NO_BB_SLP
export LCCC_BIN="${LCCC_BIN:-$PWD/target/fastbuild/lccc}"
export CCC="${CCC:-$LCCC_BIN}"
export CCC_I686="${CCC_I686:-$PWD/target/fastbuild/lccc-i686}"
export CCC_ARM="${CCC_ARM:-$PWD/target/fastbuild/lccc-arm}"
export CCC_RISCV="${CCC_RISCV:-$PWD/target/fastbuild/lccc-riscv}"
export CCC_X86="${CCC_X86:-$PWD/target/fastbuild/lccc-x86}"
export LCCC_LD="${LCCC_LD:-$PWD/target/fastbuild/lccc-ld}"
export LCCC_FASTBUILD_BIN="${LCCC_FASTBUILD_BIN:-$LCCC_BIN}"
export GCC16="${GCC16:-gcc-16}"
export GCC="${GCC:-gcc-16}"
export CLANG="${CLANG:-clang-23}"
export ICX="${ICX:-}"
export LCCC_TEST_CC="${LCCC_TEST_CC:-gcc-16}"
export LCCC_TEST_CLANG="${LCCC_TEST_CLANG:-clang-23}"
export LCCC_TEST_ASSEMBLER="${LCCC_TEST_ASSEMBLER:-$LCCC_AGENT_CACHE_PREFIX/binutils-2.47-x86_64-linux/bin/as}"
export LCCC_CALLGRIND="${LCCC_CALLGRIND:-valgrind --tool=callgrind --callgrind-out-file=/dev/null --error-exitcode=99}"
