#!/usr/bin/env bash
# ============================================================================
# kconfig_query.sh — read Kconfig values, and compare the configuration a
# package ships with the one `olddefconfig` produced.
#
# Sourced by scripts/build_kernel_full.sh. Deliberately pure: no side effects,
# no output except the two functions' own results, so the behaviour that
# decides whether a kernel build is the package's kernel can be unit-tested
# (tests/regression/check_kconfig_query.sh) without a kernel tree.
# ============================================================================

# lccc_config_value <file> <symbol>
#
# Prints the value `file` gives `symbol`: `y`, `m`, any other value verbatim
# (strings keep their quotes), `n` for the `# CONFIG_X is not set` spelling,
# and nothing at all when the file never mentions the symbol (a symbol newer
# than that configuration).
#
# The pattern anchors on `SYMBOL=`. Without the anchor, CONFIG_STACKPROTECTOR
# would read CONFIG_STACKPROTECTOR_STRONG's value — the exact confusion that
# would let a dropped hardening feature pass a parity check.
lccc_config_value() {
    awk -v sym="$2" '
        index($0, sym "=") == 1 { print substr($0, length(sym) + 2); exit }
        $0 == "# " sym " is not set" { print "n"; exit }
    ' "$1"
}

# lccc_config_drift <package-config> <resulting-config> [symbol...]
#
# Prints one line per symbol the package *enables* (`y` or `m`) that the
# resulting configuration does not keep:
#
#     <symbol> package=<value> result=<value|absent>
#
# Returns 1 when anything drifted, 0 when the package's feature set survived.
#
# Only enables are compared, in that direction: a package that disables a
# symbol is never second-guessed by a tree that turns it on, because the
# question this answers is "did the build quietly drop a feature the package
# asked for", not "is this the package's config byte for byte".
lccc_config_drift() {
    local package_config=$1 resulting_config=$2
    shift 2
    local drifted=0 symbol want got
    for symbol in "$@"; do
        want=$(lccc_config_value "$package_config" "$symbol")
        case $want in
            y | m) ;;
            *) continue ;;
        esac
        got=$(lccc_config_value "$resulting_config" "$symbol")
        if [[ $got != "$want" ]]; then
            printf '%s package=%s result=%s\n' "$symbol" "$want" "${got:-absent}"
            drifted=1
        fi
    done
    return "$drifted"
}
