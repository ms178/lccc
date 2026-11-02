#!/usr/bin/env bash
# Neutral GCC driver for benchmark_native_batched.py when an ICX object needs
# Intel's libirc (e.g. _intel_fast_memcpy). Compilation/probes go to GCC as-is;
# all link arms get the same library search/rpath/--as-needed link inputs.
# Only for user-space research; never use this to compile kernel code.
set -euo pipefail
real=${LCCC_BENCH_REAL_GCC:-gcc-16}
irc=${LCCC_BENCH_IRC_LIBDIR:?set LCCC_BENCH_IRC_LIBDIR to the directory containing libirc.so}
[[ -f "$irc/libirc.so" ]] || { echo "missing $irc/libirc.so" >&2; exit 1; }
for arg in "$@"; do
    case "$arg" in
        -c|-S|-E|--version|-dump*|-print-*) exec "$real" "$@" ;;
    esac
done
exec "$real" "$@" -L"$irc" -Wl,-rpath,"$irc" -lirc
