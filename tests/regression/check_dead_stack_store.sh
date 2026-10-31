#!/usr/bin/env bash
# Stack-slot dead-store contract: the compiler must never leave a store to a
# stack slot that nothing in the same function ever reads.
#
# Why this exists: a dead spill store is pure cost in the hottest block it sits
# in, and the peephole path that creates them is subtle.  `spill_deref` rewrites
# `movq %rax,N(%rsp); movq N(%rsp),%rcx; <deref (%rcx)>` into a direct
# `(%rax)` dereference and drops both memory ops — but only when the
# *never-read* store pass can see the slot as dead.  That pass fails closed for
# a whole function on any frame access it cannot model, and one such access was
# a 32-bit `leal -1(%rbp), %eax` under -fomit-frame-pointer: `%rbp` is a data
# register there, yet the generic `rbp_offset` arm asked for a memory read
# extent, LEA has none, and the bail swallowed the function.  glibc's Horspool
# search loop kept two dead 8-byte spill stores in its hot block (measured
# -1.0 % runtime), and `spill_deref` had already deleted the reloads, so the
# stores were unreachable garbage.
#
# The fixture below reproduces the shape: a 256-byte frame array whose address
# escapes through a computed pointer, a data-register %rbp, and enough live
# values to force spilling.  The checker is structural (store with no read of
# the slot anywhere in the function), so it catches any future regression of
# this class, not just this instance.
#
# The checker is self-tested against a synthetic assembly file with a known dead
# store — a checker that cannot fail is decoration.
set -u
CCC=${CCC:-target/fastbuild/lccc}
GCC=${GCC_BIN:-gcc}
HERE=$(cd "$(dirname "$0")" && pwd)
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
fails=0
note() { printf '%s\n' "$*"; }
bad()  { note "FAIL: $*"; fails=$((fails+1)); }

cat >"$TMP/dead_store.c" <<'C'
#include <stdio.h>
/* Escaped frame array + data-register %rbp + spill pressure: the shape that
   produced the dead spill stores.  Derived from the strstr kernel's search
   loop (bad-character shift table on the frame, computed element pointer). */
static unsigned char hay[4096];
static int search(const unsigned char *haystack, unsigned int hs_len,
                  const unsigned char *needle, unsigned int needle_len)
{
    unsigned int i;
    unsigned char shift_table[256];
    for (i = 0; i < 256; i++)
        shift_table[i] = (unsigned char)needle_len;
    for (i = 0; i < needle_len - 1; i++)
        shift_table[needle[i]] = (unsigned char)(needle_len - 1 - i);
    unsigned int h = 0;
    while (h <= hs_len - needle_len) {
        int j = (int)needle_len - 1;
        while (j >= 0 && needle[j] == haystack[h + j])
            j--;
        if (j < 0)
            return (int)h;
        h += shift_table[haystack[h + needle_len - 1]];
    }
    return -1;
}
int main(void)
{
    unsigned long long sum = 0;
    for (unsigned i = 0; i < sizeof hay; i++)
        hay[i] = (unsigned char)('a' + (i * 7 + i / 13) % 26);
    for (unsigned q = 0; q < 512; q++) {
        unsigned char needle[8];
        unsigned n = 3 + q % 5;
        for (unsigned i = 0; i < n; i++)
            needle[i] = hay[(q * 37 + i * 11) & 4095];
        int r = search(hay, 4096, needle, n);
        sum = sum * 31 + (unsigned)(r + 1);
    }
    printf("%llu\n", sum);
    return 0;
}
C

"$CCC" -O2 -fomit-frame-pointer -S "$TMP/dead_store.c" -o "$TMP/dead_store.s" \
    || { echo "FAIL: fixture compile"; exit 1; }
"$CCC" -O2 -fomit-frame-pointer -o "$TMP/dead_store" "$TMP/dead_store.c" \
    || { echo "FAIL: fixture link"; exit 1; }

# Structural check: for every function, no store to `N(%rsp)`/`N(%rbp)` whose
# slot is never read in that function.  `(%rsp)` (offset 0, no displacement) is
# excluded only when the next instruction consumes it (retpoline shape); an
# explicit `0(%rsp)` spill inside a loop is checked like any other slot.
# glibc_strstr kept two of those in its Horspool step before the fix.
cat >"$TMP/dead_slots.py" <<'PY'
import re
import sys


def dead_slots(path):
    """Return [(function, slot_text, line)] for stores whose slot is unread."""
    functions = {}
    current = "<file>"
    functions[current] = []
    with open(path, encoding="utf-8") as handle:
        for line in handle:
            text = line.strip()
            if not text:
                continue
            label = re.match(r"^([A-Za-z_.$][\w.$]*):$", text)
            if label:
                current = label.group(1)
                functions.setdefault(current, [])
                continue
            if text.startswith((".", "#")):
                continue
            functions[current].append(text)
    out = []
    for name, lines in functions.items():
        reads, writes = set(), []
        for text in lines:
            if text.startswith("push") or text.startswith("pop"):
                continue
            for match in re.finditer(r"(-?\d+)\(%r(sp|bp)\)", text):
                slot = (match.group(2), match.group(1))
                mnemonic = text.split()[0]
                is_store = (
                    re.match(r"^mov[qlwb] ", text) is not None
                    and text.rstrip().endswith(match.group(0))
                )
                if is_store:
                    writes.append((slot, text))
                else:
                    reads.add(slot)
        for slot, text in writes:
            # An offset-0 store whose value the very next instruction consumes
            # from the stack top is the retpoline shape (`movq %target,(%rsp);
            # ret`), not a spill slot.
            if slot == ("sp", "0"):
                index = lines.index(text)
                following = lines[index + 1] if index + 1 < len(lines) else ""
                if following.startswith(("ret", "pop", "jmp", "leave")):
                    continue
            if slot not in reads:
                out.append((name, f"{slot[1]}(%r{slot[0]})", text))
    return out


dead = dead_slots(sys.argv[1])
for function, slot, text in dead:
    print(f"dead stack slot {slot} in {function}: {text}")
sys.exit(1 if dead else 0)
PY

if ! python3 "$TMP/dead_slots.py" "$TMP/dead_store.s"; then
    bad "fixture keeps stores to stack slots nothing reads"
fi

# Self-test: the checker must fire on an assembly file with a known dead store
# and stay quiet on the same file once the slot is read.
cat >"$TMP/synthetic.s" <<'S'
f:
.cfi_startproc
    subq $40, %rsp
    leal -1(%rbp), %eax
    movq %rax, 8(%rsp)
    addq $40, %rsp
    ret
.cfi_endproc
S
if python3 "$TMP/dead_slots.py" "$TMP/synthetic.s" >/dev/null; then
    bad "checker missed a synthetic dead store (a checker that cannot fail is decoration)"
fi
cat >"$TMP/synthetic_ok.s" <<'S'
f:
.cfi_startproc
    subq $40, %rsp
    movq %rax, 8(%rsp)
    movq 8(%rsp), %rdx
    addq $40, %rsp
    ret
.cfi_endproc
S
if ! python3 "$TMP/dead_slots.py" "$TMP/synthetic_ok.s" >/dev/null; then
    bad "checker fired on a slot that is read"
fi

# The fixture must still compute the same answer as the host compiler.
want=$("$GCC" -O2 -o "$TMP/ref" "$TMP/dead_store.c" && "$TMP/ref")
got=$("$TMP/dead_store")
[ "$want" = "$got" ] || bad "fixture output $got != reference $want"

if [ "$fails" -eq 0 ]; then
    note "dead stack store: PASS"
    exit 0
fi
note "dead stack store: $fails failure(s)"
exit 1
