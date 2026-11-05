#!/usr/bin/env bash
# Byte-exactness of the `.ascii` string-literal emitter on both ABIs.
#
# WHY THIS EXISTS
# ---------------
# `emit_string_bytes` used to emit one `.byte` line per 32 literal bytes. The
# GCC torture case pr46534.c expands to a 20,000,000-byte literal: 625,000
# `.byte` lines (1.25M at -O2), 171 MB of assembly, 3.7 GiB peak RSS and a
# SIGKILL in a two-worker matrix. The replacement emits bounded `.ascii`
# lines with three-digit octal escapes for every byte outside
# [A-Za-z0-9 space], so no byte can be mistaken for assembly syntax and
# `elf::parse_string_literal` (shared by x86-64, i686, AArch64 and RISC-V)
# decodes it back to exactly one byte per source byte.
#
# A byte-emitter rewrite is a miscompile waiting to happen. This gate proves
# equivalence three independent ways, not by inspection:
#   1. A/B: `CCC_LEGACY_BYTE_STRINGS=1` selects the old `.byte` emitter. Every
#      case is compiled twice on BOTH ABIs at -O0/-O2/-Os and the object files
#      must be byte-identical.
#   2. Chunk boundaries: literal lengths around STRING_ASCII_CHUNK (65536)
#      are exercised, including the NUL terminator landing on a boundary.
#   3. Runtime: a host executable memcmp()s every one of the 256 byte values,
#      embedded NULs included, against an independently generated oracle.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
CCC_I686=${CCC_I686:-./target/fastbuild/lccc-i686}
CCC_X86=${CCC_X86:-./target/fastbuild/lccc-x86}
CCC_ARM=${CCC_ARM:-./target/fastbuild/lccc-arm}
CCC_RISCV=${CCC_RISCV:-./target/fastbuild/lccc-riscv}
[ -x "$CCC" ] || { echo "check_string_literal_emission: SKIP ($CCC not built)"; exit 0; }
[ -x "$CCC_I686" ] || CCC_I686=""

tmp=${TMPDIR:-/tmp}/lccc-string-emission.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"

python3 - "$tmp/allbytes.c" <<'PY'
import sys
out = sys.argv[1]
allb = ''.join('\\x%02x' % b for b in range(256))
# Lengths straddling the 65536-byte .ascii chunk boundary. All spaces keep the
# source compact; the all-bytes case covers the escaping rules.
sizes = [0, 1, 65535, 65536, 65537, 131072, 131075]
literals = ['"%s"' % allb]
for n in sizes:
    literals.append('"%s"' % (' ' * n))
with open(out, 'w') as f:
    f.write('/* generated: exhaustive byte alphabet + chunk boundaries */\n')
    f.write('extern int printf(const char *, ...);\n')
    f.write('extern int memcmp(const void *, const void *, unsigned long);\n')
    for i, lit in enumerate(literals):
        f.write('const char S%d[] = %s;\n' % (i, lit))
    f.write('const unsigned char ORACLE[256] = {')
    f.write(','.join(str(b) for b in range(256)))
    f.write('};\n')
    f.write('int main(void) {\n')
    f.write('  if (sizeof(S0) != 257) { printf("FAIL sizeof(S0)=%%lu\\n", (unsigned long)sizeof(S0)); return 1; }\n')
    f.write('  if (memcmp(S0, ORACLE, 256) != 0) { printf("FAIL alphabet bytes\\n"); return 1; }\n')
    f.write('  if (S0[256] != 0) { printf("FAIL missing NUL\\n"); return 1; }\n')
    f.write('  printf("PASS allbytes\\n"); return 0;\n')
    f.write('}\n')
PY

fail=0
for opt in -O0 -O2 -Os; do
    for arch in x86_64 i686; do
        if [ "$arch" = i686 ]; then
            [ -n "$CCC_I686" ] || continue
            cc="$CCC_I686"
        else
            cc="$CCC"
        fi
        new="$tmp/new-$arch$opt.o"
        old="$tmp/old-$arch$opt.o"
        "$cc" -w "$opt" -c "$tmp/allbytes.c" -o "$new"
        CCC_LEGACY_BYTE_STRINGS=1 "$cc" -w "$opt" -c "$tmp/allbytes.c" -o "$old"
        if ! cmp -s "$new" "$old"; then
            echo "FAIL: .ascii vs .byte objects differ ($arch $opt)"
            cmp -l "$new" "$old" | head -8 || true
            fail=1
        else
            echo "PASS: byte-identical objects ($arch $opt)"
        fi
    done
done

# Host runtime oracle. Only x86_64 is executable here; the i686 object was
# already proven byte-identical above.
if "$CCC" -w -O2 "$tmp/allbytes.c" -o "$tmp/run" && "$tmp/run" >"$tmp/run.log"; then
    cat "$tmp/run.log"
else
    echo "FAIL: host runtime byte oracle"
    cat "$tmp/run.log" 2>/dev/null || true
    fail=1
fi

# 4. Cross-architecture: every supported assembler shares
#    `elf::parse_string_literal`, so the SAME `.ascii` payload must decode to
#    the SAME `.rodata` bytes on x86-64, i686, AArch64, RISC-V and the
#    alternate x86 driver. Parse the ELF section table directly (ELF32/ELF64,
#    little-endian) so this needs no cross-objcopy in CI.
cat >"$tmp/cross.c" <<'C'
extern int puts(const char *);
int main(void) { return puts("hello\x00world\x22q\x5cs"); }
C
python3 - "$tmp" "$CCC" "$CCC_I686" "$CCC_X86" "$CCC_ARM" "$CCC_RISCV" <<'PY'
import pathlib, struct, subprocess, sys
tmp = pathlib.Path(sys.argv[1])
want = b'hello\x00world"q\\s\x00'

def rodata(path):
    b = pathlib.Path(path).read_bytes()
    if b[:4] != b'\x7fELF':
        raise AssertionError('not an ELF object')
    if b[4] == 2:
        shoff = struct.unpack_from('<Q', b, 0x28)[0]
        shentsize, shnum, shstrndx = struct.unpack_from('<HHH', b, 0x3a)
        fmt = '<IIQQQQIIQQ'
    else:
        shoff = struct.unpack_from('<I', b, 0x20)[0]
        shentsize, shnum, shstrndx = struct.unpack_from('<HHH', b, 0x2e)
        fmt = '<IIIIIIIIII'
    sh = lambda i: struct.unpack_from(fmt, b, shoff + i * shentsize)
    names = sh(shstrndx)
    table = b[names[4]:names[4] + names[5]]
    for i in range(shnum):
        s = sh(i)
        name = table[s[0]:table.index(0, s[0])].decode()
        if name == '.rodata':
            return b[s[4]:s[4] + s[5]]
    raise AssertionError('no .rodata section')

checked = 0
for arg in sys.argv[2:]:
    if not arg or not pathlib.Path(arg).is_file():
        print('SKIP: cross-arch binary not built:', arg or '(unset)')
        continue
    obj = tmp / (pathlib.Path(arg).name + '-cross.o')
    r = subprocess.run([arg, '-w', '-O0', '-c', str(tmp / 'cross.c'), '-o', str(obj)],
                       capture_output=True, text=True)
    assert r.returncode == 0, f'{arg}: compile failed: {r.stderr[:300]}'
    data = rodata(obj)
    assert want in data, f'{arg}: .rodata mismatch: {data[:64]!r}'
    print(f'PASS: exact literal bytes in .rodata ({pathlib.Path(arg).name}, {len(data)} B)')
    checked += 1
assert checked >= 2, 'at least the two primary ABIs must be checked'
PY
[ "$?" -eq 0 ] || fail=1

# 5. End-to-end: the fortify_fold printf("...\n") -> puts("...") transform
#    interns a SECOND, newline-stripped literal through the same emitter.
#    Compare the RUNNING program's bytes against GCC 16.2 at -O0 (no fold)
#    and -O2 (fold), so a wrong escape in either the original or the stripped
#    literal shows up as a runtime difference, not just an object diff.
cat >"$tmp/fold.c" <<'C'
#include <stdio.h>
int main(void) {
    /* Single-backslash OCTAL escapes: the payload really contains 0x21,
       0x22, 0x23, 0x5c, 0x60, 0x80, 0xff, 0xfe and a real trailing \n.
       A doubled backslash here would make this a literal-text string and
       NEITHER compiler would fold printf -> puts, making the test vacuous. */
    printf("fold:\041\042\043\134\140\200\377\376tail\n");
    return 0;
}
C
GCC_BIN=${GCC:-gcc-16}
if command -v "$GCC_BIN" >/dev/null 2>&1; then
    fold_fail=0
    for opt in -O0 -O2; do
        for cc_tag in lccc gcc; do
            cc="$CCC"; [ "$cc_tag" = gcc ] && cc="$GCC_BIN"
            exe="$tmp/fold-$cc_tag$opt"
            if ! "$cc" -w "$opt" "$tmp/fold.c" -o "$exe" 2>"$tmp/fold-$cc_tag$opt.err"; then
                echo "FAIL: fold compile ($cc_tag $opt): $(head -2 "$tmp/fold-$cc_tag$opt.err")"
                fold_fail=1
                continue
            fi
            "$exe" >"$tmp/fold-$cc_tag$opt.out" || { echo "FAIL: fold run ($cc_tag $opt)"; fold_fail=1; }
        done
        if ! cmp -s "$tmp/fold-lccc$opt.out" "$tmp/fold-gcc$opt.out"; then
            echo "FAIL: fold runtime bytes differ at $opt"
            fold_fail=1
        else
            echo "PASS: printf/puts fold runtime bytes equal GCC 16.2 ($opt, $(wc -c <"$tmp/fold-lccc$opt.out") B)"
        fi
    done
    # NON-VACUITY: at -O2 BOTH compilers must really fold printf -> puts.
    # Without this assert a future change to the C fixture (or a fold that
    # stops firing) would silently reduce this to a plain-emitter test.
    if "$CCC" -w -O2 -S "$tmp/fold.c" -o "$tmp/fold-lccc.s" &&
       "$GCC_BIN" -w -O2 -S "$tmp/fold.c" -o "$tmp/fold-gcc.s"; then
        for s in "$tmp/fold-lccc.s" "$tmp/fold-gcc.s"; do
            if ! grep -q 'puts' "$s"; then
                echo "FAIL: printf -> puts fold did not fire in $(basename "$s")"
                fold_fail=1
            fi
        done
        [ "$fold_fail" -eq 0 ] && echo "PASS: printf -> puts fold fires on both compilers (-O2)"
    else
        echo "FAIL: fold -S emission failed"
        fold_fail=1
    fi
    [ "$fold_fail" -eq 0 ] || fail=1
else
    echo "SKIP: GCC 16.2 oracle ($GCC_BIN) not on PATH; fold runtime not compared"
fi

[ "$fail" -eq 0 ] || exit 1
echo "PASS: check_string_literal_emission"
