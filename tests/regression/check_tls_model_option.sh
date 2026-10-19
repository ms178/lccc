#!/usr/bin/env bash
# -ftls-model must affect separately compiled PIC objects.  glibc compiles its
# local TLS definitions with Initial-Exec and links them into libc.so later; a
# Local-Exec R_X86_64_TPOFF32 in that object makes every ELF linker reject the
# shared-library link.
set -euo pipefail
CCC=${CCC:-target/fastbuild/lccc}
READELF=${READELF:-readelf}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
cat >"$tmp/tls.c" <<'EOF'
static __thread int local_tls;
int *address(void) { return &local_tls; }
int read_tls(void) { return local_tls; }
void write_tls(int value) { local_tls = value; }
EOF
"$CCC" -O2 -fPIC -ftls-model=initial-exec -c "$tmp/tls.c" -o "$tmp/ie.o"
"$READELF" -rW "$tmp/ie.o" >"$tmp/ie.reloc"
grep -q 'R_X86_64_GOTTPOFF' "$tmp/ie.reloc"
if grep -q 'R_X86_64_TPOFF32' "$tmp/ie.reloc"; then
    echo 'FAIL: Initial-Exec object contains Local-Exec TPOFF32' >&2
    exit 1
fi
"$CCC" -O2 -fPIE -ftls-model=local-exec -c "$tmp/tls.c" -o "$tmp/le.o"
le_relocs=$("$READELF" -rW "$tmp/le.o")
grep -q 'R_X86_64_TPOFF32' <<<"$le_relocs"

# Dynamic TLS models are not approximated as Local- or Initial-Exec: that can
# create an object with stronger loader/lifetime assumptions than requested.
for model in global-dynamic local-dynamic bogus ''; do
    if "$CCC" -c -ftls-model="$model" "$tmp/tls.c" -o "$tmp/rejected.o" \
        >"$tmp/rejected.out" 2>"$tmp/rejected.err"; then
        echo "FAIL: unsupported TLS model '$model' was accepted" >&2
        exit 1
    fi
    grep -q "unsupported TLS model '$model'" "$tmp/rejected.err"
done
echo 'TLS model option: PASS'
