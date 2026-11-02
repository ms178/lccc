#!/usr/bin/env bash
# Regression gate for Linux 6.18.58 semantic-analysis and parser qualifier parity:
#   1. container_of_const / _Generic(ptr, const typeof(*ptr) *: ..., default: ...)
#      inside WRITE_ONCE (*(volatile typeof(x) *)&(x) = val):
#        - non-const pointer selects default:, stays writable, and preserves
#          pointee_volatile even when typeof(...) exceeds 64 tokens;
#        - const pointer selects const arm and diagnoses read-only assignment.
#   2. Function-pointer compatibility when return/parameter types are enums
#      (including typedef'd enums and forward-declared enums completed later).
#   3. __attribute__((__cleanup__(fn))) local scope-guard variables do not emit
#      false -Wunused-variable diagnostics under -Wall -Werror, including
#      post-initializer attributes on comma-separated declarators.
#   4. -funsigned-char plain char * / signed char * / unsigned char * for %s
#      and signed/unsigned same-rank integers/enums for %d/%u under -Wformat.
#
# The GCC oracle is optional: without it every LCCC leg and every negative
# diagnostic still runs; only the GCC cross-compile/differential legs are
# skipped with a note (same convention as check_operand_diagnostics.sh).
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CCC="${CCC:-$ROOT_DIR/target/fastbuild/lccc}"
GCC_BIN=""
if command -v "${GCC:-gcc-16}" >/dev/null 2>&1; then
    GCC_BIN="${GCC:-gcc-16}"
elif command -v gcc >/dev/null 2>&1; then
    GCC_BIN="gcc"
else
    echo "note: no GCC oracle found; GCC cross-check legs skipped (all LCCC legs still run)" >&2
fi

TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/lccc-k618-sema.XXXXXX")"
trap 'rm -rf "$TMP_DIR"' EXIT

fail() {
    echo "FAIL: $*" >&2
    exit 1
}

cat > "$TMP_DIR/valid_kernel_idioms.c" <<'EOF'
#include <stddef.h>

int printf(const char *, ...) __attribute__((format(printf, 1, 2)));

enum hrtimer_restart { HRTIMER_NORESTART = 0, HRTIMER_RESTART = 1 };
typedef enum irqreturn { IRQ_NONE = 0, IRQ_HANDLED = 1 } irqreturn_t;
typedef irqreturn_t (*irq_handler_t)(int, void *);

struct hrtimer;
extern void hrtimer_setup(struct hrtimer *, enum hrtimer_restart (*)(struct hrtimer *));
extern int request_irq(unsigned int, irq_handler_t);

static enum hrtimer_restart my_hrtimer(struct hrtimer *t) { (void)t; return HRTIMER_NORESTART; }
static irqreturn_t my_irq(int irq, void *dev) { (void)irq; (void)dev; return IRQ_HANDLED; }

enum Fwd;
extern void take_fwd_fn(enum Fwd (*)(void));
extern void take_fwd_ptr(enum Fwd *);
enum Fwd { FWD_A = 0, FWD_B = 1 };
static enum Fwd make_fwd(void) { return FWD_A; }

struct sock { int sk_pad; };
struct udp_sock { struct sock sk; int pad[16]; _Bool peeking_with_offset; };
extern struct sock *get_mutable_sk(void);

/* Prove the byte offset every assembly assertion below relies on, so a
   layout change fails here with a name instead of a silent count mismatch. */
_Static_assert(offsetof(struct udp_sock, peeking_with_offset) == 68,
               "peeking_with_offset must stay at byte 68");

extern unsigned long long __bpf_call_base(unsigned long long, unsigned long long);
extern unsigned long long __bpf_call_alt(unsigned long long, unsigned long long);
struct bpf_func_proto {
    unsigned long long (*func)(unsigned long long, unsigned long long);
};

struct io_apic_route {
    union {
        struct {
            unsigned long long vector : 8,
                               irr : 1,
                               delivery_status : 1,
                               ir_index_15 : 1,
                               ir_index_0_14 : 15;
        };
        struct {
            unsigned long long w1 : 32, w2 : 32;
        };
    };
};

static inline void guard_cleanup(int *p) { *p = 0; }

int exercise_valid(struct hrtimer *t, struct sock *sk, char *s, unsigned char *us,
                   signed char *ss, enum Fwd *fp, const struct bpf_func_proto *fn,
                   struct io_apic_route entry, int val) {
    int guard1 __attribute__((__cleanup__(guard_cleanup))) = 1;
    int plain = 2, guard2 __attribute__((__cleanup__(guard_cleanup))) = 3;
    int bpf_imm = fn->func - __bpf_call_base;
    int bpf_sym_diff = __bpf_call_alt - __bpf_call_base;

    hrtimer_setup(t, my_hrtimer);
    request_irq(0, my_irq);
    take_fwd_fn(make_fwd);
    take_fwd_ptr(fp);
    printf("%s %s %s %d %u\n", s, us, ss, HRTIMER_RESTART, -1);
    printf("V(%02X) IRR(%1d) S(%1d) I(%04X) W(%u)\n",
           entry.vector, entry.irr, entry.delivery_status,
           (entry.ir_index_15 << 15) | entry.ir_index_0_14, entry.w1);

    if (val < entry.vector)
        plain += bpf_imm + bpf_sym_diff;

    /* >80 tokens inside typeof(_Generic(...)) before '*)' to guard against
       fixed lookahead truncation of pointee_volatile in WRITE_ONCE. */
    *(volatile typeof(_Generic(sk,
        const typeof(*sk) *: ((const struct udp_sock *)({
            void *__mptr = (void *)(sk);
            _Static_assert(sizeof(int) == 4 && sizeof(long) >= 4 && 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 == 8, "ok");
            ((struct udp_sock *)((char *)__mptr - 0));
        })),
        default: ((struct udp_sock *)({
            void *__mptr = (void *)(sk);
            _Static_assert(sizeof(int) == 4 && sizeof(long) >= 4 && 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 == 8, "ok");
            ((struct udp_sock *)((char *)__mptr - 0));
        }))
    )->peeking_with_offset) *)&((struct udp_sock *)sk)->peeking_with_offset = 1;

    *(volatile typeof(_Generic(get_mutable_sk(),
        const typeof(*get_mutable_sk()) *: (const struct udp_sock *)0,
        default: (struct udp_sock *)0
    )->peeking_with_offset) *)&((struct udp_sock *)sk)->peeking_with_offset = (val >= 0);

    return plain + ((struct udp_sock *)sk)->peeking_with_offset;
}
EOF

# Volatile-field store contract, measured at the object level and scoped to
# exercise_valid so unrelated frame slots can never satisfy the count. The
# field is at byte 68 (0x44, proven by the _Static_assert above); each of the
# two volatile WRITE_ONCE-style assignments must survive -O2 DSE as a byte
# store through the object pointer, and the final lvalue read must reload.
count_field_stores() { # $1 object -> number of byte stores to 0x44(reg)
    # Match the field in DESTINATION position (line-ending): both `movb
    # $1,0x44(%rbx)` and the width-by-register `mov %al,0x44(%rbx)` count,
    # while `movzbl 0x44(%rbx),%edx` (field as source) never can.
    objdump -d --no-show-raw-insn "$1" \
        | awk '
            /^[0-9a-fA-F]+ <exercise_valid>:/ { in_fn = 1; next }
            in_fn && /^[0-9a-fA-F]+ </ { in_fn = 0 }
            in_fn && /,0x44\(%r[a-z0-9]+\)[[:space:]]*$/ { n++ }
            END { print n + 0 }'
}
count_field_reloads() { # $1 object -> number of movzbl reloads from 0x44(reg)
    objdump -d --no-show-raw-insn "$1" \
        | awk '
            /^[0-9a-fA-F]+ <exercise_valid>:/ { in_fn = 1; next }
            in_fn && /^[0-9a-fA-F]+ </ { in_fn = 0 }
            in_fn && /movzbl[[:space:]]+0x44\(%r[a-z0-9]+\)/ { n++ }
            END { print n + 0 }'
}

"$CCC" -std=gnu18 -funsigned-char -Wall -Wextra -Werror -O2 -c \
    "$TMP_DIR/valid_kernel_idioms.c" -o "$TMP_DIR/lccc.o"
lccc_stores=$(count_field_stores "$TMP_DIR/lccc.o")
lccc_reloads=$(count_field_reloads "$TMP_DIR/lccc.o")
[ "$lccc_stores" -ge 2 ] || fail "expected >=2 volatile byte stores to peeking_with_offset (0x44), got $lccc_stores"
[ "$lccc_reloads" -ge 1 ] || fail "expected a volatile reload of peeking_with_offset (0x44), got $lccc_reloads"

if [ -n "$GCC_BIN" ]; then
    "$GCC_BIN" -std=gnu18 -funsigned-char -Wall -Wextra -Werror -O2 -c \
        "$TMP_DIR/valid_kernel_idioms.c" -o "$TMP_DIR/gcc.o"
    gcc_stores=$(count_field_stores "$TMP_DIR/gcc.o")
    # Differential DSE contract: LCCC must keep at least as many volatile
    # stores to the qualified field as the GCC oracle.
    [ "$lccc_stores" -ge "$gcc_stores" ] || \
        fail "volatile store count $lccc_stores below GCC oracle ($gcc_stores)"
fi

# Runtime check: GNU function-pointer subtraction must step by 1 byte, not sizeof(void *),
# and pointer-sized integer struct fields must relocate `(0 ? 0 : (unsigned long)&"STR")`
# (drivers/tty/serial/8250/8250_pci.c PCI_VDEVICE).
cat > "$TMP_DIR/fn_ptr_diff_run.c" <<'EOF'
struct pci_id {
    unsigned int vendor, device, subvendor, subdevice, cls, cls_mask;
    unsigned long driver_data;
};
static const struct pci_id pci_tbl[] = {
    { .vendor = 0x4348, .device = 0x7053, .subvendor = ~0U, .subdevice = ~0U,
      0, 0, (0 ? 0 : (unsigned long)&"PARPORT_SERIAL") },
    { 0 }
};
static const struct pci_id pci_single = {
    .vendor = 0x1c00, .driver_data = (0 ? 0 : (unsigned long)&"SINGLE_SERIAL")
};

__attribute__((noinline)) int f_alpha(int x) { return x + 1; }
__attribute__((noinline)) int f_beta(int x) { return x + 2; }
int main(void) {
    int (*pf)(int) = f_beta;
    long d1 = pf - f_alpha;
    long d2 = f_beta - f_alpha;
    long expected = (const char *)f_beta - (const char *)f_alpha;
    const char *s0 = (const char *)pci_tbl[0].driver_data;
    const char *s1 = (const char *)pci_single.driver_data;
    if (!s0 || s0[0] != 'P' || s0[8] != 'S' || s0[14] != '\0') return 2;
    if (!s1 || s1[0] != 'S' || s1[7] != 'S' || s1[13] != '\0') return 3;
    return (d1 == expected && d2 == expected) ? 0 : 1;
}
EOF
"$CCC" -std=gnu18 -Wall -Werror -O2 "$TMP_DIR/fn_ptr_diff_run.c" -o "$TMP_DIR/fn_ptr_diff_run"
"$TMP_DIR/fn_ptr_diff_run"
if [ -n "$GCC_BIN" ]; then
    "$GCC_BIN" -std=gnu18 -Wall -Werror -O2 "$TMP_DIR/fn_ptr_diff_run.c" -o "$TMP_DIR/fn_ptr_diff_run_gcc"
    "$TMP_DIR/fn_ptr_diff_run_gcc"
fi

# Negative check 1: const pointer through container_of_const MUST fail read-only check.
cat > "$TMP_DIR/neg_readonly.c" <<'EOF'
struct sock { int a; };
struct udp_sock { struct sock sk; _Bool peeking_with_offset; };
void bad(const struct sock *csk) {
    *(volatile typeof(_Generic(csk,
        const typeof(*csk) *: (const struct udp_sock *)0,
        default: (struct udp_sock *)0
    )->peeking_with_offset) *)&((struct udp_sock *)csk)->peeking_with_offset = 1;
}
EOF

if "$CCC" -std=gnu11 -fsyntax-only "$TMP_DIR/neg_readonly.c" 2>"$TMP_DIR/ro.err"; then
    fail "expected read-only error on const container_of_const"
fi
grep -q "assignment of read-only location" "$TMP_DIR/ro.err" || {
    echo "FAIL: read-only case did not name the expected diagnostic; got:" >&2
    sed 's/^/  /' "$TMP_DIR/ro.err" >&2
    exit 1
}
if [ -n "$GCC_BIN" ]; then
    "$GCC_BIN" -std=gnu11 -fsyntax-only "$TMP_DIR/neg_readonly.c" 2>"$TMP_DIR/ro.gcc.err" || true
    grep -q "assignment of read-only location" "$TMP_DIR/ro.gcc.err" || {
        echo "FAIL: GCC oracle disagrees on the read-only diagnostic; got:" >&2
        sed 's/^/  /' "$TMP_DIR/ro.gcc.err" >&2
        exit 1
    }
fi

# Negative check 2: distinct enum return types remain incompatible function pointers.
cat > "$TMP_DIR/neg_enum_fn.c" <<'EOF'
enum E1 { A1 = 0 };
enum E2 { A2 = 0 };
void take(enum E1 (*)(void));
enum E2 wrong(void);
void bad(void) { take(wrong); }
EOF

if "$CCC" -std=gnu11 -fsyntax-only "$TMP_DIR/neg_enum_fn.c" 2>"$TMP_DIR/fn.err"; then
    fail "expected incompatible-pointer-types error on distinct enum return"
fi
grep -q "incompatible pointer type" "$TMP_DIR/fn.err" || {
    echo "FAIL: enum case did not name the expected diagnostic; got:" >&2
    sed 's/^/  /' "$TMP_DIR/fn.err" >&2
    exit 1
}

# Parser lookahead: the cast-qualifier scans run to EOF with balanced
# ()/[]/{} depth instead of a 64-token cap. Valid nested spellings must keep
# detecting the pointee qualifier, and malformed input must diagnose rather
# than hang or run off the token stream.
cat > "$TMP_DIR/lookahead_valid.c" <<'EOF'
#include <stddef.h>
#include <stdint.h>
struct lv { int a; int pad[16]; _Bool flag; };
long lv_test(struct lv *p) {
    const volatile int *cv = (const volatile int *)(long)(uintptr_t)p;
    char *m = (char *)({ struct lv *t = p; t; }) + offsetof(struct lv, flag);
    const int (*deep)[4] = (const int (*)[4])(void *)0;
    const int *from_array = (const int *)&((int[]){1, 2, 3})[1];
    (void)cv; (void)deep; (void)from_array;
    struct lv *sk = p;
    *(volatile typeof(_Generic(sk,
        const typeof(*sk) *: ((const struct lv *)({ void *q = (void *)(sk); (struct lv *)q; })),
        default: ((struct lv *)({ (void)sk; sk; }))
    )->flag) *)&((struct lv *)sk)->flag = 1;
    return (long)m;
}
EOF
"$CCC" -std=gnu18 -Wall -Wextra -Werror -O2 -c "$TMP_DIR/lookahead_valid.c" -o "$TMP_DIR/lookahead_valid.o"
if [ -n "$GCC_BIN" ]; then
    "$GCC_BIN" -std=gnu18 -Wall -Wextra -Werror -O2 -c "$TMP_DIR/lookahead_valid.c" -o "$TMP_DIR/lookahead_valid_gcc.o"
fi

# Unterminated cast: the scan must hit EOF and the parser must recover with a
# diagnostic. A timeout would mean the lookahead lost its termination.
cat > "$TMP_DIR/lookahead_eof.c" <<'EOF'
int f(void) { int x = (const int *; return x; }
EOF
eof_rc=0
timeout 10 "$CCC" -std=gnu18 -fsyntax-only "$TMP_DIR/lookahead_eof.c" 2>"$TMP_DIR/eof.err" || eof_rc=$?
[ "$eof_rc" -ne 124 ] || fail "cast lookahead hung on unterminated input"
[ "$eof_rc" -ne 0 ] || fail "unterminated cast was accepted"
grep -q "error" "$TMP_DIR/eof.err" || fail "unterminated cast produced no diagnostic"

# Stray closing bracket: saturating depth subtraction must not desync the scan.
cat > "$TMP_DIR/lookahead_bracket.c" <<'EOF'
int f(void) { return (int) 0] ; }
EOF
brk_rc=0
timeout 10 "$CCC" -std=gnu18 -fsyntax-only "$TMP_DIR/lookahead_bracket.c" 2>"$TMP_DIR/brk.err" || brk_rc=$?
[ "$brk_rc" -ne 124 ] || fail "cast lookahead hung on mismatched bracket"
[ "$brk_rc" -ne 0 ] || fail "mismatched bracket was accepted"

echo "PASS: Linux 6.18.58 sema/qualifier/format/cleanup parity"
