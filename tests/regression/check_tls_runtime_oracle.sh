#!/usr/bin/env bash
# Hermetic tests: no compiler or toolchain required.
set -euo pipefail
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
source "$here/tls_runtime_oracle.sh"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cat >"$work/program" <<'PROGRAM'
#!/usr/bin/env bash
case ${0##*/} in
    good*) printf 'same\n' ;;
    wrong) printf 'different\n' ;;
    fail) printf 'same\n'; exit 7 ;;
    empty-fail) exit 9 ;;
    hang) exec sleep 30 ;;
esac
PROGRAM
chmod +x "$work/program"
for name in good good-ref wrong fail empty-fail hang; do
    ln -s program "$work/$name"
done
count=0
for pair in 'good good-ref' 'fail good' 'good fail' 'fail fail' \
    'empty-fail empty-fail' 'wrong good' 'hang good' 'good hang'; do
    read -r candidate reference <<<"$pair"
    expected=1
    [[ $pair != 'good good-ref' ]] || expected=0
    actual=0
    tls_compare_runtime "$work/$candidate" "$work/$reference" "$work/out" 0.1 \
        2>"$work/diagnostic" || actual=$?
    [[ $actual -eq $expected ]] || {
        printf 'FAIL: %s: expected %s, got %s\n' "$pair" "$expected" "$actual" >&2
        exit 1
    }
    count=$((count + 1))
done
printf 'TLS runtime oracle: %s cases PASS\n' "$count"
