#!/usr/bin/env bash
# Make-dependency honesty law: -MD/-MMD/-M/-MM must list every header the
# preprocessor actually opened, with GCC's directory-based system filter.
#
# Regression: lccc emitted `target: source` only — headers were never
# recorded.  kbuild's fixdep greps .d files for CONFIG_* tokens carried by
# headers; config flips silently stopped rebuilding dependents and the
# failure surfaced three stages later as undefined-symbol link errors
# (sched_ext/xfrm after a DEBUG_INFO_BTF flip in the 6.18 kernel build).
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
# The gate cd's into its sandbox; a relative CCC default must survive that.
case $CCC in
    /*) ;;
    *) CCC=$PWD/${CCC#./} ;;
esac
tmp=${TMPDIR:-/tmp}/lccc-deps.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/inc" "$tmp/sys"

cat > "$tmp/inc/local.h" << 'EOH'
#ifdef CONFIG_LOCAL_FEATURE
void local_feat(void);
#endif
EOH
printf '#define CONFIG_SYS_FEATURE 1\n' > "$tmp/sys/sysfeat.h"
printf '#define H "%s"\n' "$tmp/inc/local.h" > "$tmp/computed.h"
cat > "$tmp/probe.h" << 'EOH'
#if defined(CONFIG_LOCAL_FEATURE)
void probe_never_used(void);
#endif
EOH
cat > "$tmp/main.c" << 'EOC'
#include "inc/local.h"
#include <sysfeat.h>
#include "computed.h"
#include H
#if __has_include("probe.h")
#include "probe.h"
#endif
int main(void) { return 0; }
EOC

cd "$tmp"
# -isystem makes sysfeat.h a SYSTEM-directory header (the -MMD filter class);
# -I keeps inc/ a user directory.
SYS=(-isystem "$tmp/sys" -I "$tmp")

# Pin A: -MMD lists source + user headers (computed include resolved),
# excludes system-directory headers.
"$CCC" -MMD -MF a.d -c -o main.o main.c "${SYS[@]}"
grep -q "main.c" a.d || { echo "FAIL: source missing from deps" >&2; exit 1; }
grep -q "local.h" a.d || { echo "FAIL: user header missing from deps" >&2; exit 1; }
if grep -q "sysfeat.h" a.d; then
    echo "FAIL: -MMD must exclude system-directory headers" >&2; exit 1
fi

# Pin B: -MD keeps system-directory headers.
"$CCC" -MD -MF b.d -c -o main.o main.c "${SYS[@]}"
grep -q "local.h" b.d && grep -q "sysfeat.h" b.d || {
    echo "FAIL: -MD must include system-directory headers" >&2; exit 1
}

# Pin C: -MM/-M stdout rules use the same filter.
"$CCC" -MM main.c "${SYS[@]}" > c_mm.out
"$CCC" -M  main.c "${SYS[@]}" > c_m.out
grep -q "local.h" c_mm.out || { echo "FAIL: -MM dropped user header" >&2; exit 1; }
if grep -q "sysfeat.h" c_mm.out; then
    echo "FAIL: -MM must exclude system-directory headers" >&2; exit 1
fi
grep -q "sysfeat.h" c_m.out || {
    echo "FAIL: -M must include system-directory headers" >&2; exit 1
}

# Pin D: -MP emits a phony rule per prerequisite.
"$CCC" -MMD -MP -MF d.d -c -o main.o main.c "${SYS[@]}"
grep -q "^$tmp/inc/local.h:$" d.d || {
    echo "FAIL: -MP phony rule missing" >&2; exit 1
}

# Pin E: a __has_include probe without the include is NOT a dependency
# (GCC lists only files actually opened)...
cat > main2.c << 'EOC'
#include "inc/local.h"
#if __has_include("probe.h") && 0
#include "probe.h"
#endif
int main(void) { return 0; }
EOC
"$CCC" -MMD -MF e2.d -c -o main2.o main2.c "${SYS[@]}"
grep -q "local.h" e2.d || { echo "FAIL: e2 lost user header" >&2; exit 1; }
if grep -q "probe.h" e2.d; then
    echo "FAIL: __has_include probe leaked into deps" >&2; exit 1
fi
# ...while the include behind a TRUE probe is.
grep -q "probe.h" a.d || {
    echo "FAIL: actually-included header missing from deps" >&2; exit 1
}

# Pin E2: -E with -MF must write the REAL dependency list — the -E path
# drained the collector twice, so the .d file lost every header while the
# compile path stayed correct (kernel .lds.S preprocessing shape).
"$CCC" -E -MMD -MF e_e.d -o e_e.i main.c "${SYS[@]}"
grep -q "local.h" e_e.d || { echo "FAIL: -E -MF dep file lost headers" >&2; exit 1; }
if grep -q "sysfeat.h" e_e.d; then
    echo "FAIL: -E -MF must filter system headers under -MMD" >&2; exit 1
fi

# Pin F: -include files are dependencies.
"$CCC" -MMD -MF f.d -c -o main.o main.c -include "$tmp/inc/local.h" "${SYS[@]}"
grep -q "local.h" f.d || {
    echo "FAIL: -include file missing from deps" >&2; exit 1
}

# Pin G: GCC cross-check — the prereq SET matches GCC's -MMD output for the
# same translation unit (paths normalized to basenames; GCC emits relative
# quoted-include paths, lccc absolute resolved paths).  Dedup policy differs
# by design: GCC dedups by raw path string (one file reachable via two paths
# is listed twice), lccc by resolved path — the unique file SET is the
# contract fixdep and make actually consume.
if command -v gcc >/dev/null; then
    gcc -MMD -MF g.d -c -o gmain.o main.c "${SYS[@]}"
    norm() { tr ' \\' '\n\n' < "$1" | sed -n '2,$p' | grep -v '^$' | xargs -n1 basename 2>/dev/null | sort -u; }
    if ! diff <(norm a.d) <(norm g.d) > /dev/null; then
        echo "FAIL: dep set differs from GCC" >&2
        diff <(norm a.d) <(norm g.d) >&2
        exit 1
    fi
fi

echo "dep-files gate: PASS"
