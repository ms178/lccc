#!/usr/bin/env bash
# Call-result typing must use the visible lexical binding, not a hidden
# file-scope function. Positive cases run; negative cases must be rejected.
set -euo pipefail
repo=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cc=${LCCC:-$repo/target/fastbuild/lccc}
gcc=${GCC16:-gcc-16}
clang=${CLANG23:-clang-23}
tmp=$(mktemp -d)
trap 'rm -rf -- "$tmp"' EXIT

positive="$repo/tests/regression/fnptr_result_shadows_void_function.c"
cat > "$tmp/negative.c" <<'EOF'
extern int f(int);
int g(void) {
    int f = 0;
    return f(1);
}
EOF

check_positive() {
  local compiler=$1 opt=$2
  "$compiler" "$opt" -Wall -Wextra -Werror "$positive" -o "$tmp/pos"
  "$tmp/pos"
}
check_negative() {
  local compiler=$1
  if "$compiler" -Wall -Wextra -Werror -c "$tmp/negative.c" -o "$tmp/neg.o" 2>"$tmp/neg.err"; then
    echo "FAIL: $compiler accepted a call through a non-callable shadow" >&2
    exit 1
  fi
  [[ -s "$tmp/neg.err" ]]
}

for opt in -O0 -O2; do check_positive "$cc" "$opt"; done
check_negative "$cc"
if command -v "$gcc" >/dev/null 2>&1; then
  for opt in -O0 -O2; do check_positive "$gcc" "$opt"; done
  check_negative "$gcc"
fi
if command -v "$clang" >/dev/null 2>&1; then
  for opt in -O0 -O2; do check_positive "$clang" "$opt"; done
  check_negative "$clang"
fi
echo 'PASS: call-result shadowing positive/negative across available compilers'
