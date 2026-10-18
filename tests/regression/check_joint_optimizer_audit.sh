#!/usr/bin/env bash
# Joint #862/#863 contracts: guarded scans and shared-object interposition.
set -euo pipefail
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
ccc=${CCC:-$root/target/fastbuild/lccc}
gcc_bin=${GCC:-gcc-16}
[[ -x "$ccc" ]] || { echo 'FAIL: lccc not built' >&2; exit 1; }
[[ $("$gcc_bin" -dumpfullversion) == 16.2.0 ]] || {
    echo 'FAIL: GCC 16.2.0 required' >&2; exit 1;
}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
src=$root/tests/regression/guarded_bit_scan_audit.c
for opt in -O0 -O1 -O2 -O3 -Os; do
    CCC_VERIFY_IR=1 "$ccc" "$opt" "$src" -o "$work/scan"
    "$work/scan"
    "$gcc_bin" "$opt" -Wall -Wextra -Werror "$src" -o "$work/reference"
    "$work/reference"
done
# Executable symbols preempt the shared object's default-visible definitions.
cat >"$work/provider.c" <<'EOF'
int audit_target(int x) { return x + 1; }
int audit_probe(int x) { return audit_target(x); }
EOF
cat >"$work/caller.c" <<'EOF'
#include <dlfcn.h>
int audit_target(int x) { return x + 7; }
int main(int argc, char **argv) {
    if (argc != 2) return 1;
    void *h = dlopen(argv[1], RTLD_NOW);
    if (!h) return 2;
    int (*p)(int) = (int (*)(int))dlsym(h, "audit_probe");
    int rc = !p || p(10) != 17;
    if (dlclose(h)) return 3;
    return rc;
}
EOF
"$gcc_bin" -O2 -Wall -Wextra -Werror -rdynamic "$work/caller.c" -ldl -o "$work/caller"
for flags in '-shared' '-shared -fPIC' '-shared -fno-pic'; do
    # Deliberate word splitting of this fixed flag table.
    "$ccc" -O2 $flags "$work/provider.c" -o "$work/provider.so"
    "$work/caller" "$work/provider.so"
done
echo 'PASS: guarded bit scans and shared-object interposition'
