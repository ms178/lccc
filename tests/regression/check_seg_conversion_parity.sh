#!/usr/bin/env bash
# Table-driven parity gate for named-address-space conversions.
#
# Each row of tests/regression/seg_conv/parity_table.tsv is one conversion
# shape (assignment, argument, return, initializer, comparison, struct,
# pointee chain). The row's `expect` column is GCC 16.2's verdict, recorded
# when the table was written; the gate re-checks all three:
#   expect == GCC 16.2 (gcc -w)  the table itself has not drifted from GCC;
#   expect == lccc                lccc agrees with the oracle on every shape.
# Verdicts are accept/reject only. Diagnostic wording is not oracle-compared
# (see the seg_conv README); a reject must carry an address-space,
# disjoint-space or incompatible-pointer diagnostic so a reject for an
# unrelated reason cannot pass.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
table=tests/regression/seg_conv/parity_table.tsv
. tests/regression/lib/gcc_oracle.sh
require_gcc_oracle "check_seg_conversion_parity" || exit 1

# exit_verdict <status> -> accept | reject | crash:<status>. Only the ordinary
# exit codes are verdicts; a signal or abort is a failure, not a rejection.
exit_verdict() {
    case "$1" in
        0) echo accept ;;
        1) echo reject ;;
        *) echo "crash:$1" ;;
    esac
}

tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-seg-parity.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
gcc_oracle_positive_control "$CCC" "$tmp" || exit 1
gcc_oracle_positive_control "$GCC_ORACLE_BIN" "$tmp" || exit 1

prelude='typedef int __seg_gs *gp_t;
typedef int __seg_fs *fp_t;
typedef struct s { int m; } s_t;
typedef int __seg_gs sint;
sint gobj;
struct seg_in { int __seg_gs *p; int x; };
struct seg_out { struct seg_in i; int __seg_gs *q2; };
struct seg_arr { struct seg_in v[2]; };
union seg_un { int __seg_gs *p; int x; };
int *q;
gp_t gp, gq;
fp_t fp;
int arr[4];
int __seg_gs *garr[4];
struct s __seg_gs *sgp;
struct s *sq;
int __seg_gs **pp;
int **qq;
int __seg_gs **a;
int **b;
void take_gp(gp_t p);
void take_q(int *p);
void take_vp(void *p);'

fail=0
rows=0
while IFS=$'\t' read -r id expect ctx body; do
    case "$id" in ''|\#*) continue ;; esac
    rows=$((rows + 1))
    case "$ctx" in
        stmt) fn="void fx(void) { $body }" ;;
        ret)  fn="gp_t fx(void) { $body }" ;;
        int)  fn="int fx(void) { $body }" ;;
        *) echo "FAIL: $id: unknown ctx '$ctx'" >&2; fail=1; continue ;;
    esac
    f="$tmp/$id.c"
    printf '%s\n%s\n' "$prelude" "$fn" > "$f"

    set +e
    lerr=$("$CCC" -c -o "$tmp/l.o" "$f" 2>&1)
    lrc=$?
    "$GCC_ORACLE_BIN" -w -std=gnu11 -c -o "$tmp/g.o" "$f" >/dev/null 2>&1
    grc=$?
    set -e
    lverdict=$(exit_verdict "$lrc")
    gverdict=$(exit_verdict "$grc")
    if [ "$gverdict" != "$expect" ]; then
        echo "FAIL: $id: table says $expect but the GCC 16 oracle says $gverdict; fix the table" >&2
        fail=1
    fi
    if [ "$lverdict" != "$expect" ]; then
        echo "FAIL: $id: lccc $lverdict, expected $expect" >&2
        grep -m1 . <<<"$lerr" >&2 || true
        fail=1
        continue
    fi
    if [ "$expect" = reject ] && ! grep -q -e 'address space' -e 'disjoint' -e 'incompatible pointer type' -e 'specified for structure field' <<<"$lerr"; then
        echo "FAIL: $id: rejected without an address-space or pointer diagnostic" >&2
        grep -m1 . <<<"$lerr" >&2 || true
        fail=1
    fi
done < "$table"

if [ "$rows" -lt 40 ]; then
    echo "FAIL: parity table has $rows shapes; the contract is at least 40" >&2
    fail=1
fi
if [ "$fail" -ne 0 ]; then
    echo "check_seg_conversion_parity: FAILED"
    exit 1
fi
echo "check_seg_conversion_parity: PASSED ($rows shapes, lccc == GCC $GCC_ORACLE_MAJOR)"
