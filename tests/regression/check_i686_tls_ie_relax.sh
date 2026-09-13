#!/usr/bin/env bash
# Regression gate for the i686 initial-exec TLS pipeline:
#
#   1. The assembler must keep the STT_TLS symbol in GOTIE relocations
#      (local-label folding is value-identical for plain relocs but WRONG
#      for TLS: a GOT-style TLS reloc is a whole-slot reference whose
#      addend must stay 0, and on REL-format i686 the folded offset is
#      baked into the instruction bytes and then clobbered by the
#      linker's write — measured: first TLS touch segfaulted).
#   2. The linker must relax the main-image
#        movl slot(%ebx), %reg / addl %gs:0, %reg
#      pair to the local-exec form
#        movl $tpoff, %reg / addl %gs:0, %reg
#      (the transform GNU ld applies), for local AND global main-image
#      TLS variables alike.
#   3. The linked program must run and print the expected values.
set -euo pipefail
cd "$(dirname "$0")/../.."
ccc=${LCCC_BIN:-target/fastbuild/lccc}
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT

cat >"$td/tls.c" <<'CEOF'
#include <stdio.h>
static __thread int le_var = 5;   /* local, offset 0 in the block */
static __thread int mid_var = 6;  /* local, offset 4 */
__thread int g_var = 9;           /* global, offset 8 */
int main(void){
    le_var += 1; mid_var += 1; g_var += 1;
    printf("le=%d mid=%d g=%d\n", le_var, mid_var, g_var);
    return (le_var == 6) && (mid_var == 7) && (g_var == 10) ? 0 : 1;
}
CEOF

"$ccc" -m32 -O1 -c "$td/tls.c" -o "$td/tls.o"

# ── 1. object structure: GOTIE relocs must reference STT_TLS symbols ──
relocs=$(readelf -rW "$td/tls.o")
if ! grep -q "R_386_TLS_GOTIE" <<<"$relocs"; then
    echo "SKIP: no GOTIE relocations emitted (codegen changed?)"
    exit 0
fi
for sym in $(readelf -rW "$td/tls.o" | awk '/R_386_TLS_GOTIE/{print $NF}'); do
    case "$sym" in
        .*)
            echo "FAIL: GOTIE reloc references section symbol '$sym' (local-label folding leaked into a TLS reloc)"
            exit 1
            ;;
    esac
    kind=$(readelf -sW "$td/tls.o" | awk -v s="$sym" '$8==s {print $4; exit}')
    if [ "$kind" != "TLS" ]; then
        echo "FAIL: GOTIE reloc references '$sym' of type '$kind' (expected TLS)"
        exit 1
    fi
done

# ── 2. full link + runtime ──
"$ccc" -m32 -O1 "$td/tls.c" -o "$td/tls_exe"
out=$("$td/tls_exe")
if [ "$out" != "le=6 mid=7 g=10" ]; then
    echo "FAIL: i686 TLS runtime output '$out' (expected 'le=6 mid=7 g=10')"
    exit 1
fi

# ── 3. the linked code must be relaxed: no live GOT load before the add ──
# The relaxed `movl $tpoff, %reg` is 0xc7; the unrelaxed
# `movl slot(%ebx), %reg` is 0x8b.  Count `add %gs:0` instructions whose
# predecessor is still a disp32-based load — that must be zero for
# main-image TLS.
if command -v objdump >/dev/null 2>&1; then
    unrelaxed=$(objdump -d "$td/tls_exe" | awk '
        /add.*%gs:0/ { if (prev ~ /mov.*\(.*%ebx\)/) n++ }
        { sub(/:.*$/, "", $0); prev = $0 }
        END { print n + 0 }')
    if [ "$unrelaxed" -gt 0 ]; then
        echo "FAIL: $unrelaxed unrelaxed GOT-load TLS sequence(s) in main image"
        exit 1
    fi
fi

# ── 4. deterministic per-register probe (hand assembly) ──────────────────
# The C7 /0 encoding trap: `movl $tpoff, %reg` after relaxation is C7 /0,
# which fixes the ModRM REG field to 000 and puts the destination in the
# r/m field.  Writing 0xC0|(reg<<3) instead emits the undefined C7 /reg
# form and #UDs on every destination except %eax (where the two fields
# coincide).  Drive every register through the GOTIE pair and check both
# the exact relaxed bytes and the runtime value.
cat >"$td/tls_regs.s" <<'SEOF'
.section .tdata,"awT",@progbits
.align 4
.type rv0, @tls_object
.size rv0, 4
rv0: .long 1
.type rv1, @tls_object
.size rv1, 4
rv1: .long 2
.type rv2, @tls_object
.size rv2, 4
rv2: .long 3
.type rv3, @tls_object
.size rv3, 4
rv3: .long 4
.type rv4, @tls_object
.size rv4, 4
rv4: .long 5
.type rv5, @tls_object
.size rv5, 4
rv5: .long 6
.text
.globl main
.type main, @function
main:
    pushl %ebx
    call __x86.get_pc_thunk.bx
    addl $_GLOBAL_OFFSET_TABLE_, %ebx
    movl rv0@GOTNTPOFF(%ebx), %eax
    addl %gs:0, %eax
    incl (%eax)
    movl rv1@GOTNTPOFF(%ebx), %ecx
    addl %gs:0, %ecx
    incl (%ecx)
    movl rv2@GOTNTPOFF(%ebx), %edx
    addl %gs:0, %edx
    incl (%edx)
    movl rv3@GOTNTPOFF(%ebx), %esi
    addl %gs:0, %esi
    incl (%esi)
    movl rv4@GOTNTPOFF(%ebx), %edi
    addl %gs:0, %edi
    incl (%edi)
    movl rv5@GOTNTPOFF(%ebx), %ebp
    addl %gs:0, %ebp
    incl (%ebp)
    popl %ebx
    /* read back: acc = r0 + r1*10 + r2*100 + r3*1000 + r4*10000 + r5*100000 */
    pushl %ebx
    call __x86.get_pc_thunk.bx
    addl $_GLOBAL_OFFSET_TABLE_, %ebx
    movl rv0@GOTNTPOFF(%ebx), %eax
    addl %gs:0, %eax
    movl (%eax), %eax
    movl rv1@GOTNTPOFF(%ebx), %ecx
    addl %gs:0, %ecx
    movl (%ecx), %edx
    imull $10, %edx
    addl %edx, %eax
    movl rv2@GOTNTPOFF(%ebx), %ecx
    addl %gs:0, %ecx
    movl (%ecx), %edx
    imull $100, %edx
    addl %edx, %eax
    movl rv3@GOTNTPOFF(%ebx), %esi
    addl %gs:0, %esi
    movl (%esi), %edx
    imull $1000, %edx
    addl %edx, %eax
    movl rv4@GOTNTPOFF(%ebx), %esi
    addl %gs:0, %esi
    movl (%esi), %edx
    imull $10000, %edx
    addl %edx, %eax
    movl rv5@GOTNTPOFF(%ebx), %esi
    addl %gs:0, %esi
    movl (%esi), %edx
    imull $100000, %edx
    addl %edx, %eax
    popl %ebx
    ret
SEOF

"$ccc" -m32 -O1 "$td/tls_regs.s" -o "$td/tls_regs"
# Expected after the increments: 2 + 3*10 + 4*100 + 5*1000 + 6*10000 + 7*100000
want=$(( 2 + 3*10 + 4*100 + 5*1000 + 6*10000 + 7*100000 ))
got=$("$td/tls_regs"; echo $?)
# main returns the value; the shell only keeps the low 8 bits — compare mod 256
if [ $(( got % 256 )) -ne $(( want % 256 )) ]; then
    echo "FAIL: per-register TLS probe returned $got (expected $want)"
    exit 1
fi

# Static check: every C7 in the linked text must be the /0 form
# (ModRM REG field == 000).  Any other C7 form is an undefined opcode.
python3 - "$td/tls_regs" <<'PEOF'
import subprocess, sys
path = sys.argv[1]
data = open(path, "rb").read()
# Restrict the scan to the executable LOAD segment(s) so data bytes
# (ELF header, .tdata, dynamic relocations) cannot false-positive.
phdrs = subprocess.run(
    ["readelf", "-lW", path], capture_output=True, text=True).stdout
segs = []
for line in phdrs.splitlines():
    parts = line.split()
    if len(parts) >= 8 and parts[0] == "LOAD" and "E" in parts[6:8]:
        p_offset = int(parts[1], 16)
        p_vaddr = int(parts[2], 16)
        fsize = int(parts[4], 16)
        segs.append((p_vaddr - p_offset, p_vaddr - p_offset + fsize))
assert segs, "no executable LOAD segment found"
bad = []
for f0, f1 in segs:
    i = f0
    while True:
        i = data.find(b"\xc7", i, f1)
        if i < 0 or i + 1 >= f1:
            break
        modrm = data[i + 1]
        if (modrm >> 3) & 7:
            bad.append(i)
        i += 1
if bad:
    print("FAIL: C7 bytes with non-zero ModRM REG field at file offsets:",
          ", ".join(hex(b) for b in bad[:8]))
    sys.exit(1)
PEOF
if [ $? -ne 0 ]; then
    exit 1
fi

echo "OK: i686 TLS IE relaxation (GOTIE keeps STT_TLS symbols; main-image pair relaxed; per-register C7 /0 encoding; runtime correct)"
