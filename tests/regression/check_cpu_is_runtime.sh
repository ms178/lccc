#!/usr/bin/env bash
# Gate: the semantics of `__builtin_cpu_is`.
#
# GCC lowers `__builtin_cpu_is("name")` to one comparison against a field of
# libgcc's `__cpu_model` (vendor/type/subtype); LCCC lowers it the same way
# from `src/common/cpu_is_table.rs`, which `scripts/gen_x86_cpu_is_table.py`
# measures from GCC 16.2's own code generation. This gate checks:
#
#   1. name space: every table name compiles; nonsense names, a non-literal
#      argument and the wrong arity are hard errors with GCC's wording;
#   2. constant context: a static initialiser is rejected like GCC does;
#   3. runtime truth on the executing host: for every name the oracle GCC
#      accepts, the LCCC-built and GCC-built answers agree; for every table
#      name, LCCC's answer equals a direct read of the `__cpu_model` field
#      (this covers `amdfam1ah`, which GCC documents but rejects);
#   4. initialisation order: the answer is already right inside the earliest
#      user constructor (GCC relies on libgcc's constructor having run);
#   5. assembly contract: a `__cpu_model` load, never a folded constant;
#   6. the i686 flavour agrees with the x86-64 one, and the AArch64 backend
#      diagnoses the builtin (Clang 23.1: "builtin is not supported on this
#      target"; GCC has no such builtin there).
set -uo pipefail

CCC=$(realpath "${CCC:-./target/fastbuild/lccc}")
CCC_I686=${LCCC_I686:-$(dirname "$CCC")/lccc-i686}
CCC_ARM=${LCCC_ARM:-$(dirname "$CCC")/lccc-arm}
REPO=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$REPO"
TABLE=src/common/cpu_is_table.rs
GCC=${GCC_BIN:-gcc}
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT
fail=0
ok()  { printf 'ok   %s\n' "$*"; }
bad() { printf 'FAIL %s\n' "$*"; fail=1; }

[ -x "$CCC" ] || { echo "FAIL: no compiler at $CCC"; exit 1; }

python3 - "$TABLE" "$td" <<'PY'
import re, sys
rows = re.findall(r'\("([^"]+)", CpuModelField::(\w+), (\d+)\)', open(sys.argv[1]).read())
td = sys.argv[2]
off = {"Vendor": 0, "Type": 1, "Subtype": 2}
with open(f"{td}/names.txt", "w") as f:
    f.write("".join(f"{n}\n" for n, _, _ in rows))
with open(f"{td}/accept.c", "w") as f:
    for i, (n, _, _) in enumerate(rows):
        f.write(f'int a{i}(void) {{ return __builtin_cpu_is("{n}"); }}\n')
with open(f"{td}/values.c", "w") as f:
    f.write("#include <stdio.h>\nint main(void) {\n")
    for n, _, _ in rows:
        f.write(f'    printf("{n}=%d\\n", !!__builtin_cpu_is("{n}"));\n')
    f.write("    return 0;\n}\n")
# The same answers from the raw libgcc fields (`struct __processor_model`).
with open(f"{td}/fields.c", "w") as f:
    f.write("#include <stdio.h>\n"
            "extern struct { unsigned v[3]; unsigned f[1]; } __cpu_model;\n"
            "int __cpu_indicator_init(void);\n"
            "int main(void) {\n    __cpu_indicator_init();\n")
    for n, field, val in rows:
        f.write(f'    printf("{n}=%d\\n", __cpu_model.v[{off[field]}] == {val}u);\n')
    f.write("    return 0;\n}\n")
PY
names=$(wc -l < "$td/names.txt")
[ "$names" -gt 60 ] || bad "table parse found only $names names"

# ---------------------------------------------------------------- 1. name space
if "$CCC" -O2 -c "$td/accept.c" -o "$td/accept.o" 2>"$td/accept.err"; then
    ok "all $names table names compile"
else
    bad "table name rejected"; head -3 "$td/accept.err" | sed 's/^/      /'
fi
diag_row() {  # description, source, expected wording
    printf '%s\n' "$2" > "$td/diag.c"
    if "$CCC" -c "$td/diag.c" -o /dev/null 2>"$td/diag.err"; then
        bad "$1: accepted"
    elif grep -qF "$3" "$td/diag.err"; then
        ok "$1 rejected: $3"
    else
        bad "$1: wrong message"; head -2 "$td/diag.err" | sed 's/^/      /'
    fi
}
for n in x86-64 haswel amd64 skylake-avx512x; do
    diag_row "unknown name '$n'" "int f(void) { return __builtin_cpu_is(\"$n\"); }" \
        "parameter to builtin not valid: $n"
done
diag_row "non-literal argument" \
    'int f(const char *s) { return __builtin_cpu_is(s); }' \
    "parameter to builtin must be a string constant or literal"
diag_row "no argument" 'int f(void) { return __builtin_cpu_is(); }' \
    "too few arguments to function '__builtin_cpu_is'"
diag_row "two arguments" 'int f(void) { return __builtin_cpu_is("intel", 1); }' \
    "too many arguments to function '__builtin_cpu_is'"

# ------------------------------------------------------- 2. constant context
diag_row "static initialiser" 'static int v = __builtin_cpu_is("intel"); int g(void) { return v; }' \
    "initializer element is not constant"

# --------------------------------------------------------- 3. runtime truth
run_values() {  # compiler-label, compiler, output, flags...
    local label=$1 cc=$2 out=$3; shift 3
    if ! "$cc" -O2 "$@" "$td/values.c" -o "$td/$out.bin" 2>"$td/$out.err"; then
        bad "$label: compile failed"; head -3 "$td/$out.err" | sed 's/^/      /'; return 1
    fi
    "$td/$out.bin" > "$td/$out.txt"
}
if run_values "lccc -m64" "$CCC" lccc; then
    if "$GCC" -O2 "$td/fields.c" -o "$td/fields.bin" 2>"$td/fields.err"; then
        "$td/fields.bin" > "$td/fields.txt"
        if cmp -s "$td/lccc.txt" "$td/fields.txt"; then
            ok "all $names answers equal the raw __cpu_model fields"
        else
            bad "answer differs from the raw __cpu_model field"
            diff "$td/lccc.txt" "$td/fields.txt" | sed -n '1,6s/^/      /p'
        fi
    else
        bad "field reader failed to build"; head -3 "$td/fields.err" | sed 's/^/      /'
    fi
    # The oracle GCC rejects the names it does not accept; compare the rest.
    cp "$td/values.c" "$td/values_gcc.c"
    for _ in 1 2 3; do
        "$GCC" -O2 "$td/values_gcc.c" -o "$td/gcc.bin" 2>"$td/gcc.err" && break
        grep -o 'parameter to builtin not valid: [A-Za-z0-9._-]*' "$td/gcc.err" \
            | awk '{print $NF}' | sort -u > "$td/unknown.txt"
        [ -s "$td/unknown.txt" ] || break
        grep -vF -f <(sed 's/.*/"&"/' "$td/unknown.txt") "$td/values_gcc.c" > "$td/v.tmp"
        mv "$td/v.tmp" "$td/values_gcc.c"
    done
    if [ -x "$td/gcc.bin" ]; then
        "$td/gcc.bin" > "$td/gcc.txt"
        if grep -xF -f "$td/gcc.txt" "$td/lccc.txt" | cmp -s - "$td/gcc.txt"; then
            ok "$(wc -l < "$td/gcc.txt") answers equal $("$GCC" -dumpfullversion)'s"
        else
            bad "answer differs from GCC"
            grep -vxF -f "$td/lccc.txt" "$td/gcc.txt" | sed -n '1,6s/^/      gcc /p'
        fi
    else
        bad "GCC reference failed to build"; head -3 "$td/gcc.err" | sed 's/^/      /'
    fi
    grep -q '=1$' "$td/lccc.txt" || bad "no name matched this host (vendor at least must)"
fi

# --------------------------------------------------- 4. initialisation order
cat > "$td/ctor.c" <<'EOF'
#include <stdio.h>
static int early_intel = -1, early_amd = -1;
__attribute__((constructor(101))) static void early(void) {
    early_intel = __builtin_cpu_is("intel");
    early_amd = __builtin_cpu_is("amd");
}
int main(void) {
    int ok = early_intel == __builtin_cpu_is("intel") && early_amd == __builtin_cpu_is("amd");
    printf("%d %d %d\n", ok, early_intel, early_amd);
    return !ok;
}
EOF
if "$CCC" -O2 "$td/ctor.c" -o "$td/ctor" 2>"$td/ctor.err" && "$td/ctor" > "$td/ctor.out"; then
    ok "answers are right inside the earliest constructor ($(cat "$td/ctor.out"))"
else
    bad "earliest-constructor answer wrong"; sed -n '1,3p' "$td/ctor.err" "$td/ctor.out" 2>/dev/null
fi

# --------------------------------------------------- 5. assembly contract
printf 'int f(void) { return __builtin_cpu_is("znver4"); }\n' > "$td/asm.c"
for flags in "-O2" "-O2 -march=znver4" "-O2 -fPIC"; do
    # shellcheck disable=SC2086
    if "$CCC" $flags -S "$td/asm.c" -o "$td/asm.s" 2>/dev/null \
            && grep -q '__cpu_model' "$td/asm.s" && grep -q '__cpu_indicator_init' "$td/asm.s"; then
        ok "runtime __cpu_model compare under $flags"
    else
        bad "no runtime __cpu_model compare under $flags"
    fi
done

# ------------------------------------------------- 6. other targets
if [ -x "$CCC_I686" ]; then
    if "$CCC_I686" -O2 "$td/values.c" -o "$td/l32.bin" 2>"$td/l32.err" \
            && "$td/l32.bin" > "$td/l32.txt"; then
        if cmp -s "$td/l32.txt" "$td/lccc.txt"; then
            ok "i686 answers equal the x86-64 ones"
        else
            bad "i686 answers differ"; diff "$td/l32.txt" "$td/lccc.txt" | sed -n '1,4s/^/      /p'
        fi
    else
        bad "i686 build failed"; head -3 "$td/l32.err" | sed 's/^/      /'
    fi
fi
if [ -x "$CCC_ARM" ]; then
    printf 'int f(void) { return __builtin_cpu_is("intel"); }\n' > "$td/arm.c"
    if "$CCC_ARM" -c "$td/arm.c" -o /dev/null 2>"$td/arm.err"; then
        bad "AArch64 accepted __builtin_cpu_is"
    elif grep -qF "builtin is not supported on this target" "$td/arm.err"; then
        ok "AArch64 diagnoses __builtin_cpu_is"
    else
        bad "AArch64: wrong message"; head -2 "$td/arm.err" | sed 's/^/      /'
    fi
fi

[ "$fail" -eq 0 ] && echo "PASS check_cpu_is_runtime" || echo "FAIL check_cpu_is_runtime"
exit "$fail"
