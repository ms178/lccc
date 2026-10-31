#!/usr/bin/env bash
# `scripts/kconfig_query.sh` decides whether a kernel build is the package's
# kernel: it reads Kconfig values and reports the package features that
# `olddefconfig` dropped. Both answers are silent-failure material — a reader
# that confuses CONFIG_STACKPROTECTOR with CONFIG_STACKPROTECTOR_STRONG, or a
# comparison that treats "absent" as "fine", lets a build quietly stop being
# the artifact the package ships — so the two functions are pinned here,
# against synthetic config pairs, without needing a kernel tree.
#
# Usage: bash tests/regression/check_kconfig_query.sh
set -euo pipefail
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
# shellcheck source=../../scripts/kconfig_query.sh
. "$root/scripts/kconfig_query.sh"
tmp=$(mktemp -d)
trap 'rm -rf -- "$tmp"' EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }

# ---- 1. value reading, every spelling -------------------------------
cat > "$tmp/pkg.config" <<'EOF'
CONFIG_FUNCTION_TRACER=y
CONFIG_MODULES=m
CONFIG_STACKPROTECTOR=y
CONFIG_STACKPROTECTOR_STRONG=y
CONFIG_LOCALVERSION="-lccc"
CONFIG_CC_VERSION_TEXT="gcc (GCC) 16.2.0"
# CONFIG_DEBUG_INFO is not set
# CONFIG_KASAN is not set
CONFIG_DEBUG_ENTRY=y
CONFIG_SCHED_BORE=y
CONFIG_UNWINDER_ORC=y
EOF

value() { lccc_config_value "$tmp/pkg.config" "$1"; }
[[ $(value CONFIG_FUNCTION_TRACER) == y ]] || fail "y not read"
[[ $(value CONFIG_MODULES) == m ]] || fail "m not read"
[[ $(value CONFIG_LOCALVERSION) == '"-lccc"' ]] || fail "quoted string not read verbatim"
[[ $(value CONFIG_DEBUG_INFO) == n ]] || fail "the '# ... is not set' spelling not read as n"
[[ -z $(value CONFIG_NOT_IN_THIS_CONFIG) ]] || fail "an absent symbol must read empty"
[[ $(value CONFIG_STACKPROTECTOR) == y ]] || fail "prefix symbol confused with its longer sibling"
[[ $(value CONFIG_STACKPROTECTOR_STRONG) == y ]] || fail "longer sibling not read"
[[ $(value CONFIG_STACK) == "" ]] || fail "a shorter prefix must not match a longer symbol"
# The pattern must not match mid-line either.
printf 'CONFIG_X=1\n# CONFIG_XY is not set\n' > "$tmp/mid.config"
[[ $(lccc_config_value "$tmp/mid.config" CONFIG_XY) == n ]] || fail "mid-line match"

# ---- 2. drift: what the package enables and the result lost ----------
cat > "$tmp/result.config" <<'EOF'
CONFIG_FUNCTION_TRACER=y
CONFIG_MODULES=m
CONFIG_STACKPROTECTOR=y
CONFIG_STACKPROTECTOR_STRONG=n
CONFIG_LOCALVERSION="-lccc"
CONFIG_CC_VERSION_TEXT="lccc 0.1.0"
# CONFIG_DEBUG_INFO is not set
# CONFIG_KASAN is not set
CONFIG_SCHED_BORE=y
EOF

features=(FUNCTION_TRACER MODULES STACKPROTECTOR STACKPROTECTOR_STRONG
          UNWINDER_ORC DEBUG_ENTRY SCHED_BORE KASAN DEBUG_INFO NOT_IN_THIS_CONFIG)
prefixed=()
for symbol in "${features[@]}"; do prefixed+=("CONFIG_$symbol"); done

drift=$(lccc_config_drift "$tmp/pkg.config" "$tmp/result.config" "${prefixed[@]}") && \
    fail "a dropped feature set must report drift"
expected=$(printf '%s\n%s\n%s' \
    "CONFIG_STACKPROTECTOR_STRONG package=y result=n" \
    "CONFIG_UNWINDER_ORC package=y result=absent" \
    "CONFIG_DEBUG_ENTRY package=y result=absent")
[[ $drift == "$expected" ]] || fail "drift lines wrong:
--- want ---
$expected
--- got ---
$drift"

# ---- 3. no drift when the package's enables survive ------------------
lccc_config_drift "$tmp/pkg.config" "$tmp/pkg.config" "${prefixed[@]}" \
    || fail "identical configurations must report no drift"

# ---- 4. direction: a package DISABLE is not drift ---------------------
printf '# CONFIG_KASAN is not set\n' > "$tmp/off.config"
printf 'CONFIG_KASAN=y\n' > "$tmp/on.config"
lccc_config_drift "$tmp/off.config" "$tmp/on.config" CONFIG_KASAN \
    || fail "a package-disabled symbol turned on is not drift (the package did not ask for it)"

# ---- 5. a missing package config is not a crash -----------------------
: > "$tmp/empty.config"
lccc_config_drift "$tmp/empty.config" "$tmp/result.config" "${prefixed[@]}" \
    || fail "an empty package config enables nothing, so nothing can drift"

echo "kconfig query: PASS (value spellings, prefix anchoring, drift set, direction)"
