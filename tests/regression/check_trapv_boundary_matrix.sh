#!/usr/bin/env bash
# -ftrapv boundary matrix for one ABI. Split from check_driver_option_contract.sh
# so the two 6,776-case legs can run on different hosted runners in parallel.
set -euo pipefail

CCC=$(realpath "${CCC:-./target/fastbuild/lccc}")
bits=
while [ $# -gt 0 ]; do
    case $1 in
        --bits) bits=$2; shift 2 ;;
        -h|--help) echo "usage: $0 --bits -m64|-m32"; exit 0 ;;
        *) echo "$0: unknown argument: $1" >&2; exit 2 ;;
    esac
done
case $bits in
    -m64|-m32) ;;
    *) echo "$0: --bits must be -m64 or -m32" >&2; exit 2 ;;
esac
[ -x "$CCC" ] || { echo "$0: CCC is not executable: $CCC" >&2; exit 2; }
td=$(mktemp -d "${TMPDIR:-/tmp}/lccc-trapv-$bits.XXXXXX")
trap 'rm -rf "$td"' EXIT
# -ftrapv must trap exactly on overflow, never on a representable result.
# 14 operations x 22 x 22 boundary operands on one ABI.  The expected verdict
# and wrapped value come from a reference TU built WITHOUT -ftrapv using only
# division and unsigned wrap-around, so it shares no code with either
# overflow lowering.  The total must equal GCC 16.2's (-m64 and -m32: 910 of
# 6776), so an empty or degenerate matrix cannot pass.  i686 `long long`
# multiply used an I128 product, which ILP32 does not have, and trapped on
# every input including 0 * 0.
cat > "$td/trapv_ops.c" <<'EOF'
typedef long long ll;
int add32(int a, int b) { return a + b; }
int sub32(int a, int b) { return a - b; }
int mul32(int a, int b) { return a * b; }
int neg32(int a) { return -a; }
int inc32(int a) { a++; return a; }
int cadd32(int a, int b) { a += b; return a; }
int cmul32(int a, int b) { a *= b; return a; }
ll add64(ll a, ll b) { return a + b; }
ll sub64(ll a, ll b) { return a - b; }
ll mul64(ll a, ll b) { return a * b; }
ll neg64(ll a) { return -a; }
ll dec64(ll a) { a--; return a; }
short adds(short a, short b) { return a + b; }
unsigned addu(unsigned a, unsigned b) { return a + b; }
EOF
cat > "$td/trapv_matrix.c" <<'EOF'
#include <limits.h>
#include <stdio.h>
#include <sys/wait.h>
#include <unistd.h>
typedef long long ll;
typedef unsigned long long ull;
int add32(int, int); int sub32(int, int); int mul32(int, int); int neg32(int);
int inc32(int); int cadd32(int, int); int cmul32(int, int);
ll add64(ll, ll); ll sub64(ll, ll); ll mul64(ll, ll); ll neg64(ll); ll dec64(ll);
short adds(short, short); unsigned addu(unsigned, unsigned);
static const ll V[] = {0, 1, -1, 2, -2, 3, 46341, -46341, 65536, INT_MAX, INT_MIN,
    INT_MAX - 1, INT_MIN + 1, 3037000499LL, -3037000499LL, 3037000500LL, 4294967296LL,
    LLONG_MAX, LLONG_MIN, LLONG_MAX - 1, LLONG_MIN + 1, LLONG_MAX / 2 + 1};
enum { NV = sizeof V / sizeof V[0], NF = 14 };
static ll run(int f, ll a, ll b) {
    switch (f) {
    case 0: return add32((int)a, (int)b);   case 1: return sub32((int)a, (int)b);
    case 2: return mul32((int)a, (int)b);   case 3: return neg32((int)a);
    case 4: return inc32((int)a);           case 5: return cadd32((int)a, (int)b);
    case 6: return cmul32((int)a, (int)b);  case 7: return add64(a, b);
    case 8: return sub64(a, b);             case 9: return mul64(a, b);
    case 10: return neg64(a);               case 11: return dec64(a);
    case 12: return adds((short)a, (short)b);
    default: return addu((unsigned)a, (unsigned)b);
    }
}
/* Reference: does the mathematically exact result fit, and the wrapped value. */
static int mul_ovf(ll a, ll b) {
    if (a == 0 || b == 0) return 0;
    if ((a == -1 && b == LLONG_MIN) || (b == -1 && a == LLONG_MIN)) return 1;
    return (ll)((ull)a * (ull)b) / b != a;
}
static int expect(int f, ll a, ll b, ll *w) {
    int x = (int)a, y = (int)b;
    ll e;
    switch (f) {
    case 0: case 5: e = (ll)x + y; *w = (int)(unsigned)e; return e != (int)e;
    case 1: e = (ll)x - y; *w = (int)(unsigned)e; return e != (int)e;
    case 2: case 6: e = (ll)x * y; *w = (int)(unsigned)e; return e != (int)e;
    case 3: *w = (int)(0u - (unsigned)x); return x == INT_MIN;
    case 4: *w = (int)((unsigned)x + 1u); return x == INT_MAX;
    case 7: *w = (ll)((ull)a + (ull)b); return (b > 0 && a > LLONG_MAX - b) || (b < 0 && a < LLONG_MIN - b);
    case 8: *w = (ll)((ull)a - (ull)b); return (b < 0 && a > LLONG_MAX + b) || (b > 0 && a < LLONG_MIN + b);
    case 9: *w = (ll)((ull)a * (ull)b); return mul_ovf(a, b);
    case 10: *w = (ll)(0ull - (ull)a); return a == LLONG_MIN;
    case 11: *w = (ll)((ull)a - 1ull); return a == LLONG_MIN;
    case 12: *w = (short)(unsigned short)((short)a + (short)b); return 0;
    default: *w = (unsigned)((unsigned)a + (unsigned)b); return 0;
    }
}
int main(void) {
    int traps = 0, bad = 0;
    for (int f = 0; f < NF; f++)
        for (int i = 0; i < NV; i++)
            for (int j = 0; j < NV; j++) {
                int p[2];
                if (pipe(p)) return 2;
                pid_t pid = fork();
                if (pid == 0) {
                    close(p[0]);
                    ll r = run(f, V[i], V[j]);
                    _exit(write(p[1], &r, sizeof r) == (ssize_t)sizeof r ? 0 : 3);
                }
                close(p[1]);
                ll r = 0, w = 0;
                ssize_t got = read(p[0], &r, sizeof r);
                close(p[0]);
                int st;
                waitpid(pid, &st, 0);
                int trapped = !(WIFEXITED(st) && WEXITSTATUS(st) == 0 && got == (ssize_t)sizeof r);
                if (trapped && !(WIFSIGNALED(st) && WTERMSIG(st) == 6)) {
                    printf("op %d (%lld, %lld): died without SIGABRT (status %#x)\n", f, V[i], V[j], st);
                    bad++;
                }
                int ovf = expect(f, V[i], V[j], &w);
                traps += trapped;
                if (trapped != ovf || (!trapped && r != w)) {
                    if (bad < 8)
                        printf("op %d (%lld, %lld): trapped=%d expected=%d result=%lld want=%lld\n",
                               f, V[i], V[j], trapped, ovf, r, w);
                    bad++;
                }
            }
    printf("traps=%d bad=%d\n", traps, bad);
    return bad != 0 || traps != 910;
}
EOF
for bits in "$bits"; do
    "$CCC" $bits -O2 -ftrapv -c "$td/trapv_ops.c" -o "$td/trapv_ops$bits.o"
    "$CCC" $bits -O2 -c "$td/trapv_matrix.c" -o "$td/trapv_matrix$bits.o"
    "$CCC" $bits "$td/trapv_matrix$bits.o" "$td/trapv_ops$bits.o" -o "$td/trapv_matrix$bits"
    if ! "$td/trapv_matrix$bits" > "$td/trapv_matrix$bits.log" 2>&1; then
        echo "FAIL: -ftrapv $bits traps on a representable result or misses an overflow:" >&2
        cat "$td/trapv_matrix$bits.log" >&2
        exit 1
    fi
done

echo "trapv boundary matrix ($bits): PASS (910 expected traps, 0 mismatches)"
