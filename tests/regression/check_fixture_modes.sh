#!/usr/bin/env bash
# Run self-checking C fixtures (static assertions plus a nonzero exit on any
# runtime mismatch) with the host C oracle and with lccc, at -O0 and -O2, on
# x86-64 and, when the host can run i386 binaries, on i686. The oracle leg
# proves each expectation; the lccc leg checks lccc against it. The regression
# corpus only covers x86-64, and type layout facts differ on i686 (an 8-byte
# enum or `long long` is 4-aligned there), hence this gate.
#
# usage: check_fixture_modes.sh FIXTURE.c...   (paths relative to tests/regression)
set -euo pipefail

root=$(cd -- "$(dirname -- "$0")/../.." && pwd)
CCC=${CCC:-"$root/target/fastbuild/lccc"}
CC=${CC:-gcc}
(($# > 0)) || { echo "usage: $0 FIXTURE.c..." >&2; exit 2; }
[[ -x "$CCC" ]] || { echo "missing LCCC compiler: $CCC" >&2; exit 1; }
command -v "$CC" >/dev/null 2>&1 || { echo "missing C oracle: $CC" >&2; exit 1; }
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
# shellcheck source=tests/regression/i386_exec.sh
source "$root/tests/regression/i386_exec.sh"

modes=(-m64)
case "$(i386_capability "$CC" "$work")" in
    run) modes+=(-m32) ;;
    *) echo "fixture modes: i686 run leg unavailable on this host (SKIP -m32)" ;;
esac
legs=0
for fixture in "$@"; do
    src="$root/tests/regression/$fixture"
    [[ -f "$src" ]] || { echo "missing fixture: $src" >&2; exit 1; }
    for mode in "${modes[@]}"; do
        for opt in -O0 -O2; do
            for compiler in "$CC" "$CCC"; do
                exe="$work/t$legs"
                "$compiler" -std=gnu17 "$mode" "$opt" -w "$src" -o "$exe"
                "$exe" >/dev/null || { echo "FAIL: $fixture: $compiler $mode $opt" >&2; exit 1; }
                legs=$((legs + 1))
            done
        done
    done
done
echo "fixture modes: PASS ($# fixtures, $legs legs)"
