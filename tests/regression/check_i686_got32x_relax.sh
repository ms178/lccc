#!/usr/bin/env bash
# Regression gate for i386 GOT32/GOT32X linking by lccc-ld (executables):
#
#   1. Every R_386_GOT32X form ld 2.47 converts for a symbol defined in a
#      non-PIC executable is converted the same way, so no GOT load is left:
#        movl x@GOT(%b),%r -> movl $x,%r     OPl x@GOT(%b),%r -> OPl $x,%r
#        testl %r,x@GOT(%b) -> testl $x,%r   pushl x@GOT(%b) -> %cs: pushl $x
#        call *x@GOT(%b) -> addr32 call x    jmp *x@GOT(%b) -> jmp x; nop
#      with or without the base register (elf_i386_convert_load_reloc).
#   2. Non-convertible references keep a slot of their own, and a base-less
#      one takes the slot's ABSOLUTE address (i386 psABI G + A): GCC's
#      `-fno-pic -fno-plt` `call *puts@GOT` used to jump through GOT-relative
#      garbage and segfault.
#   3. A local symbol only uses a GOT slot that holds its own address: slots
#      are keyed by name, so a second object's `static lv` used to bind to
#      the first object's -- now a link error instead of a wrong value.
#   4. The linked program runs and every check passes.
#
# The objects are assembled by lccc (byte/relocation parity with GAS 2.47 is
# the i686 asm-diff corpus' job, tests/asm-diff/i686/symgot.casefile).
# ELF32 execution follows check_i686_tls_ie_relax.sh: native, then
# $LCCC_I686_RUNNER, then a probed qemu-i386; no working path SKIPs loudly.
set -euo pipefail
cd "$(dirname "$0")/../.."
ccc=${LCCC_BIN:-target/fastbuild/lccc}
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT

if ! printf '#include <stdio.h>\n' | "$ccc" -m32 -x c - -fsyntax-only >/dev/null 2>&1; then
    if printf '#include <stdio.h>\n' | gcc -m32 -x c - -fsyntax-only >/dev/null 2>&1; then
        echo "FAIL: i686-got32x-relax probe cannot parse <stdio.h> though gcc -m32 can" >&2
        exit 1
    fi
    echo "SKIP: i686-got32x-relax - no i386 libc headers on this host (CI installs gcc-multilib)"
    exit 0
fi

# The `.ascii` markers delimit the converted code for the form check (they
# sit between functions and are never executed).
cat >"$td/ga.s" <<'EOF'
 .data
 .globl gv
gv: .long 7
lv: .long 9
msg: .asciz "via-got"
 .text
 .ascii "GOTXBEG!"
 .globl get_gv_mov, get_gv_add, get_gv_test, get_lv_mov, get_lv_a, call_gf
 .globl get_gv_abs, jmp_gf, call_gf_abs, get_gv_abs2, get_gv_or, get_gv_sub
 .globl push_gv, say
get_gv_mov:  push %ebx; call 1f; 1: pop %ebx; addl $_GLOBAL_OFFSET_TABLE_+[.-1b], %ebx
 movl gv@GOT(%ebx), %eax; pop %ebx; ret
get_gv_add:  push %ebx; call 1f; 1: pop %ebx; addl $_GLOBAL_OFFSET_TABLE_+[.-1b], %ebx
 xorl %eax, %eax; addl gv@GOT(%ebx), %eax; pop %ebx; ret
get_gv_test: push %ebx; call 1f; 1: pop %ebx; addl $_GLOBAL_OFFSET_TABLE_+[.-1b], %ebx
 movl $-1, %eax; xorl %ecx, %ecx; testl %eax, gv@GOT(%ebx); setne %cl; movl %ecx, %eax; pop %ebx; ret
get_lv_mov:  push %ebx; call 1f; 1: pop %ebx; addl $_GLOBAL_OFFSET_TABLE_+[.-1b], %ebx
 movl lv@GOT(%ebx), %eax; pop %ebx; ret
call_gf:     push %ebx; call 1f; 1: pop %ebx; addl $_GLOBAL_OFFSET_TABLE_+[.-1b], %ebx
 call *gf@GOT(%ebx); pop %ebx; ret
get_gv_abs:  movl gv@GOT, %eax; movl (%eax), %eax; ret
jmp_gf:      push %ebx; call 1f; 1: pop %ebx; addl $_GLOBAL_OFFSET_TABLE_+[.-1b], %ebx
 pop %ebx; jmp *gf@GOT(%ebx)
call_gf_abs: call *gf@GOT; ret
get_gv_abs2: movl gv@GOT, %ecx; movl %ecx, %eax; ret
get_gv_or:   push %ebx; call 1f; 1: pop %ebx; addl $_GLOBAL_OFFSET_TABLE_+[.-1b], %ebx
 xorl %edx, %edx; orl gv@GOT(%ebx), %edx; movl %edx, %eax; pop %ebx; ret
get_gv_sub:  push %ebx; call 1f; 1: pop %ebx; addl $_GLOBAL_OFFSET_TABLE_+[.-1b], %ebx
 movl $0, %esi; subl gv@GOT(%ebx), %esi; negl %esi; movl %esi, %eax; pop %ebx; ret
push_gv:     push %ebx; call 1f; 1: pop %ebx; addl $_GLOBAL_OFFSET_TABLE_+[.-1b], %ebx
 pushl gv@GOT(%ebx); popl %eax; pop %ebx; ret
 .ascii "GOTXEND!"
get_lv_a:    leal lv, %eax; ret
# puts is defined in libc.so: never converted; the base-less form takes the
# slot's absolute address.
say:         pushl $msg; call *puts@GOT; addl $4, %esp; ret
EOF
cat >"$td/gb.s" <<'EOF'
 .data
lv: .long 11
 .text
 .globl get_lv2_mov, get_lv2_a
get_lv2_mov: push %ebx; call 1f; 1: pop %ebx; addl $_GLOBAL_OFFSET_TABLE_+[.-1b], %ebx
 movl lv@GOT(%ebx), %eax; pop %ebx; ret
get_lv2_a: leal lv, %eax; ret
EOF
cat >"$td/gm.c" <<'EOF'
#include <stdio.h>
extern int gv; int gf(void) { return 5; }
int *get_gv_mov(void), *get_gv_add(void), *get_lv_mov(void), *get_lv_a(void);
int *get_lv2_mov(void), *get_lv2_a(void), *get_gv_abs2(void), *get_gv_or(void);
int *get_gv_sub(void), *push_gv(void);
int call_gf(void), get_gv_abs(void), get_gv_test(void), jmp_gf(void), call_gf_abs(void);
void say(void);
int main(void) {
    int ok = 1, n = 0;
#define T(c) do { n++; if (!(c)) { ok = 0; printf("FAIL %d: %s\n", n, #c); } } while (0)
    T(get_gv_mov() == &gv); T(get_gv_add() == &gv); T(get_gv_test() == 1);
    T(call_gf() == 5); T(get_gv_abs() == 7);
    T(get_lv_mov() == get_lv_a() && *get_lv_mov() == 9);
    T(get_lv2_mov() == get_lv2_a() && *get_lv2_mov() == 11);
    T(jmp_gf() == 5); T(call_gf_abs() == 5); T(get_gv_abs2() == &gv);
    T(get_gv_or() == &gv); T(get_gv_sub() == &gv); T(push_gv() == &gv);
    say();
    printf("%s %d\n", ok ? "ALL_OK" : "FAIL", n);
    return !ok;
}
EOF
"$ccc" -m32 -c "$td/ga.s" -o "$td/ga.o"
"$ccc" -m32 -c "$td/gb.s" -o "$td/gb.o"
"$ccc" -m32 -no-pie "$td/gm.c" "$td/ga.o" "$td/gb.o" -o "$td/gt"

want_out=$'via-got\nALL_OK 13'
run32() {
    if [ -n "$elf32_runner" ]; then
        # shellcheck disable=SC2086 # multi-word runner command is intended
        $elf32_runner "$1"
    else
        "$1"
    fi
}
elf32_runner=""
if out=$("$td/gt" 2>/dev/null) && [ "$out" = "$want_out" ]; then
    :
elif [ -n "${LCCC_I686_RUNNER:-}" ]; then
    # shellcheck disable=SC2086
    if out=$(${LCCC_I686_RUNNER} "$td/gt" 2>/dev/null) && [ "$out" = "$want_out" ]; then
        elf32_runner=$LCCC_I686_RUNNER
    fi
fi
if [ -z "$elf32_runner" ] && ! out=$("$td/gt" 2>/dev/null); then
    for q in qemu-i386 qemu-i386-static; do
        command -v "$q" >/dev/null 2>&1 || continue
        if out=$("$q" "$td/gt" 2>/dev/null) && [ "$out" = "$want_out" ]; then
            elf32_runner=$q
            break
        fi
    done
fi
if [ -z "$elf32_runner" ] && ! out=$("$td/gt" 2>/dev/null); then
    echo "SKIP: no working ELF32 execution path (native i386 blocked and no"
    echo "      runner produced the expected output; set LCCC_I686_RUNNER)"
    exit 0
fi
out=$(run32 "$td/gt" || true)
if [ "$out" != "$want_out" ]; then
    printf 'FAIL: i686 GOT32X runtime output:\n%s\n(expected:\n%s)\n' "$out" "$want_out"
    exit 1
fi

# ── forms: every conversion present, no GOT access left between markers ──
python3 - "$td/gt" <<'PEOF'
import re, struct, subprocess, sys, tempfile
data = open(sys.argv[1], "rb").read()
b, e = data.find(b"GOTXBEG!"), data.find(b"GOTXEND!")
assert 0 <= b < e, "markers not found in the executable"
with tempfile.NamedTemporaryFile(suffix=".bin") as f:
    f.write(data[b + 8:e]); f.flush()
    dis = subprocess.run(["objdump", "-D", "-b", "binary", "-m", "i386", f.name],
                         capture_output=True, text=True, check=True).stdout
ins = [l.split("\t")[1:] for l in dis.splitlines() if re.match(r"^\s+[0-9a-f]+:\t", l)]
code = [(raw.strip(), (txt[0] if txt else "").strip()) for raw, *txt in ins]
def count(rx):
    return sum(1 for raw, _ in code if re.fullmatch(rx, raw))
want = {
    "movl $x,%eax  (mov, based and base-less)": (r"c7 c0( ..){4}", 3),
    "movl $x,%ecx  (base-less)": (r"c7 c1( ..){4}", 1),
    "addl $x,%eax": (r"81 c0( ..){4}", 1),
    "orl $x,%edx": (r"81 ca( ..){4}", 1),
    "subl $x,%esi": (r"81 ee( ..){4}", 1),
    "testl $x,%eax": (r"f7 c0( ..){4}", 1),
    "%cs: pushl $x": (r"2e 68( ..){4}", 1),
    "addr32 call x  (based and base-less)": (r"67 e8( ..){4}", 2),
    "jmp x": (r"e9( ..){4}", 1),
}
bad = []
for name, (rx, n) in want.items():
    got = count(rx)
    if got != n:
        bad.append(f"{name}: {got} (expected {n})")
jmp_nop = any(re.fullmatch(r"e9( ..){4}", code[i][0]) and code[i + 1][0] == "90"
              for i in range(len(code) - 1))
if not jmp_nop:
    bad.append("jmp x is not followed by its nop")
left = [t for raw, t in code if "(%ebx)" in t or re.fullmatch(r"(8b|ff) (05|0d|15|25|35)( ..){4}", raw)]
if left:
    bad.append("GOT accesses left: " + "; ".join(left))
if bad:
    print("FAIL: i686 GOT32X conversions:\n  " + "\n  ".join(bad))
    sys.exit(1)
PEOF

# ── a local name bound to another object's slot is refused ──
# Two objects each need a slot for their own `static lv` (`leal lv@GOT`
# takes the slot's address, R_386_GOT32): the name-keyed slot can hold only
# one of the two addresses, so the other reference must fail rather than
# read the wrong object's slot.
for i in 1 2; do
    cat >"$td/gc$i.s" <<EOF
 .data
lv: .long $i
 .text
 .globl other$i
other$i: push %ebx; call 1f; 1: pop %ebx; addl \$_GLOBAL_OFFSET_TABLE_+[.-1b], %ebx
 leal lv@GOT(%ebx), %eax; movl (%eax), %eax; pop %ebx; ret
EOF
    "$ccc" -m32 -c "$td/gc$i.s" -o "$td/gc$i.o"
done
printf 'int main(void) { return 0; }\n' >"$td/gcm.c"
if err=$("$ccc" -m32 -no-pie "$td/gcm.c" "$td/gc1.o" "$td/gc2.o" -o "$td/gbad" 2>&1); then
    echo "FAIL: a non-convertible GOT reference to a colliding local name linked"
    exit 1
fi
if ! grep -q "needs a GOT slot of its own" <<<"$err"; then
    printf 'FAIL: unexpected diagnostic for the colliding local GOT slot:\n%s\n' "$err"
    exit 1
fi

echo "OK: i686 GOT32X (ld 2.47 conversions, absolute base-less slots, per-object local slots; runtime correct)"
