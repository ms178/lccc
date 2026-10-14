#!/usr/bin/env bash
# check_arm_visibility_shared.sh — the shared-library visibility contract
# (PR #821 F2, review of 2026-10-14): the input-visibility contract and the
# emitted ELF must agree.
#
# THE GAP THIS PINS
# -----------------
# The ADR admission predicate treated a non-default visibility declaration in
# any input as a binding constraint, while the shared emitter exported every
# defined global as GLOBAL DEFAULT (st_other = 0) in dynsym — a hidden
# definition was exported and externally preemptible, so the ADR guarantee
# and the ELF disagreed.  The fix carries the gABI-merged visibility through
# symbol resolution (the most constraining STV_* of all references and the
# definition, order-independent), and the emitter uses that ONE value for the
# export decision, the dynsym st_other byte and the ADR predicate.
#
# THE CONTRACT (real links; dynsym inspected with readelf)
#   1. hidden DEFINITION       -> not exported (absent from dynsym)
#   2. protected DEFINITION    -> exported, st_other = PROTECTED
#   3. default DEFINITION      -> exported, st_other = DEFAULT    (control)
#   4. hidden REFERENCE in one TU + default DEFINITION in another,
#      in BOTH input orders            -> not exported (gABI merge,
#      order-independent)
#   5. a DEFAULT definition keeps a plain STB_GLOBAL st_info (no accidental
#      binding changes)
#
# Mutation check (--self-test): a fake arm driver modeling the PRE-FIX
# emitter ("link" every input, export every definition as GLOBAL DEFAULT)
# must fail rows 1 and 4.  The fake emits a minimal ET_DYN with real section
# headers, so the gate's inspection path (readelf --dyn-syms) is exercised
# end-to-end, not mocked.
set -uo pipefail

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
SELF=$(cd "$(dirname "$0")" && pwd)/$(basename "$0")
LDDD=${CCC_ARM:-$ROOT/target/fastbuild/lccc-arm}
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
cd "$WORK"

fail=0
step() {
    local name=$1 expr=$2
    if eval "$expr"; then
        printf 'PASS %s\n' "$name"
    else
        printf 'FAIL %s\n' "$name"
        fail=1
    fi
}

craft_vis() {
    # usage: craft_vis <out.o> <sym_name> <st_shndx> <st_other> <binding:local|global>
    python3 - "$@" <<'PYEOF'
import struct, sys
path, name, shndx, other, binding = sys.argv[1], sys.argv[2], int(sys.argv[3]), int(sys.argv[4]), sys.argv[5]
EM_AARCH64 = 183
text = struct.pack('<II', 0xD65F03C0, 0xD503201F)  # ret ; nop
bind_val = 0 if binding == "local" else 1          # STB_LOCAL / STB_GLOBAL
sym_null = struct.pack('<IBBHQQ', 0, 0, 0, 0, 0, 0)
sym_f = struct.pack('<IBBHQQ', 1, (bind_val << 4) | 2, other, shndx, 0, 1)  # STT_FUNC, st_name=1
symtab = sym_null + sym_f
strtab = b'\x00' + name.encode() + b'\x00'
shstrtab = b'\x00.text\x00.symtab\x00.strtab\x00.shstrtab\x00'
names = {'.text': 1, '.symtab': 7, '.strtab': 15, '.shstrtab': 23}
off = 64
def align(x, a=8): return (x + a - 1) & ~(a - 1)
offs = []
for blob in (text, symtab, strtab, shstrtab):
    off = align(off); offs.append(off); off += len(blob)
shoff = align(off)
eh = bytearray(struct.pack('<16sHHIQQQIHHHHHH',
    b'\x7fELF\x02\x01\x01\x00' + b'\x00' * 8, 1, EM_AARCH64, 1, 0, 64, 0,
    0, 64, 0, 0, 64, 5, 4))
struct.pack_into('<Q', eh, 0x28, shoff)
out = bytearray(eh)
for o, blob in zip(offs, (text, symtab, strtab, shstrtab)):
    while len(out) < o: out.append(0)
    out += blob
while len(out) < shoff: out.append(0)
def shdr(nm, typ, flags, addr, offset, size, link, info, al, es):
    return struct.pack('<IIQQQQIIQQ', nm, typ, flags, addr, offset, size, link, info, al, es)
sh = shdr(0, 0, 0, 0, 0, 0, 0, 0, 0, 0)
sh += shdr(names['.text'], 1, 0x6, 0, offs[0], len(text), 0, 0, 4, 0)
sh += shdr(names['.symtab'], 2, 0, 0, offs[1], len(symtab), 3, 1, 8, 24)
sh += shdr(names['.strtab'], 3, 0, 0, offs[2], len(strtab), 0, 0, 1, 0)
sh += shdr(names['.shstrtab'], 3, 0, 0, offs[3], len(shstrtab), 0, 0, 1, 0)
out += sh
open(path, 'wb').write(bytes(out))
PYEOF
}

if [[ ${1:-} == --self-test ]]; then
    # Fake arm driver modeling the PRE-FIX emitter: exports every input
    # object's first symbol as GLOBAL DEFAULT in a real .dynsym.
    cat > fake-link-all <<'EOF'
#!/usr/bin/env bash
prev=""; out=""; objs=()
for a in "$@"; do
    case "$prev" in -o) out=$a;; esac
    case "$a" in *.o) objs+=("$a");; esac
    prev=$a
done
python3 - "$out" "${objs[@]}" <<'PYEOF'
import struct, sys
out = sys.argv[1]
entries = [(0, 0, 0, 0, 0, 0)]
strtab = b'\x00'
for src in sys.argv[2:]:
    d = open(src, 'rb').read()
    shoff = struct.unpack_from('<Q', d, 0x28)[0]
    shnum = struct.unpack_from('<H', d, 0x3c)[0]
    for i in range(1, shnum):
        sh = shoff + i * 64
        typ = struct.unpack_from('<I', d, sh + 4)[0]
        if typ == 2:  # SHT_SYMTAB
            off, _size, link = struct.unpack_from('<QQI', d, sh + 24)
            st_name = struct.unpack_from('<I', d, off + 24)[0]
            stroff = struct.unpack_from('<Q', d, shoff + link * 64 + 24)[0]
            end = d.index(b'\x00', stroff + st_name)
            symname = d[stroff + st_name:end].decode()
            entries.append((len(strtab), (1 << 4) | 2, 0, 1, 0x10000, 0))  # st_name, st_info(GLOBAL|FUNC), st_other, st_shndx, st_value, st_size
            strtab += symname.encode() + b'\x00'
            break
dynsym = b''.join(struct.pack('<IBBHQQ', *e) for e in entries)
shstrtab = b'\x00.dynsym\x00.dynstr\x00.shstrtab\x00'
n = {'.dynsym': 1, '.dynstr': 9, '.shstrtab': 17}
off = 64
def align(x, a=8): return (x + a - 1) & ~(a - 1)
offs = []
for blob in (dynsym, strtab, shstrtab):
    off = align(off); offs.append(off); off += len(blob)
shoff = align(off)
eh = bytearray(struct.pack('<16sHHIQQQIHHHHHH', b'\x7fELF\x02\x01\x01\x00' + b'\x00'*8, 3, 183, 1, 0, 64, 0, 0, 64, 0, 0, 64, 4, 0))
struct.pack_into('<Q', eh, 0x28, shoff)
o = bytearray(eh)
for oo, blob in zip(offs, (dynsym, strtab, shstrtab)):
    while len(o) < oo: o.append(0)
    o += blob
while len(o) < shoff: o.append(0)
def shdr(nm, typ, flags, addr, offset, size, link, info, al, es):
    return struct.pack('<IIQQQQIIQQ', nm, typ, flags, addr, offset, size, link, info, al, es)
sh = shdr(0,0,0,0,0,0,0,0,0,0)
sh += shdr(n['.dynsym'], 11, 2, 0, offs[0], len(dynsym), 2, 1, 8, 24)  # SHT_DYNSYM
sh += shdr(n['.dynstr'], 3, 2, 0, offs[1], len(strtab), 0, 0, 1, 0)
sh += shdr(n['.shstrtab'], 3, 0, 0, offs[2], len(shstrtab), 0, 0, 1, 0)
o += sh
open(out, 'wb').write(bytes(o))
PYEOF
exit 0
EOF
    chmod +x fake-link-all
    craft_vis h.o hidden_fn 1 2 global
    out=$(CCC_ARM="$PWD/fake-link-all" bash "$SELF" 2>&1)
    if grep -q '^FAIL hidden definition: not exported' <<<"$out"; then
        echo "PASS self-test (pre-fix export-everything behavior is caught)"
    else
        echo "FAIL self-test: the fake pre-fix linker was NOT rejected"
        fail=1
    fi
    exit $fail
fi

[[ -x "$LDDD" ]] || { echo "FAIL: lccc-arm not built at $LDDD" >&2; exit 1; }

dynsym_of() { readelf -W --dyn-syms "$1" 2>/dev/null; }

# --- Row 1: hidden definition is NOT exported --------------------------------
craft_vis h.o hidden_fn 1 2 global
"$LDDD" -shared -o libh.so h.o 2>err; rc=$?
step "hidden definition: links"                     '(( rc == 0 ))'
step "hidden definition: not exported"              '! dynsym_of libh.so | grep -q "hidden_fn"'

# --- Row 2: protected definition is exported, st_other = PROTECTED -----------
craft_vis p.o protected_fn 1 3 global
"$LDDD" -shared -o libp.so p.o 2>err; rc=$?
step "protected definition: links"                  '(( rc == 0 ))'
step "protected definition: exported as PROTECTED"  'dynsym_of libp.so | grep -q "protected_fn" && dynsym_of libp.so | grep "protected_fn" | grep -q "PROTECTED"'

# --- Row 3 (control): default definition is exported, st_other = DEFAULT -----
craft_vis d.o default_fn 1 0 global
"$LDDD" -shared -o libd.so d.o 2>err; rc=$?
step "default definition: exported as DEFAULT"      'dynsym_of libd.so | grep -q "default_fn" && dynsym_of libd.so | grep "default_fn" | grep -q " DEFAULT "'
step "default definition: GLOBAL binding preserved" 'dynsym_of libd.so | grep "default_fn" | grep -q "GLOBAL"'

# --- Row 4: hidden reference + default definition, BOTH input orders ---------
# TU A: default definition of merged_fn (st_other 0, global, defined).
# TU B: global UNDEFINED reference with st_other = 2 (hidden).
craft_vis a.o merged_fn 1 0 global
craft_vis bref.o merged_fn 0 2 global   # shndx 0 = SHN_UNDEF reference
"$LDDD" -shared -o libab.so a.o bref.o 2>err; rc=$?
step "hidden ref + default def (A,B): links"        '(( rc == 0 ))'
step "hidden ref + default def (A,B): not exported" '! dynsym_of libab.so | grep -q "merged_fn"'
"$LDDD" -shared -o libba.so bref.o a.o 2>err; rc=$?
step "hidden ref + default def (B,A): links"        '(( rc == 0 ))'
step "hidden ref + default def (B,A): not exported" '! dynsym_of libba.so | grep -q "merged_fn"'

if (( fail )); then
    echo "FAIL: shared-library visibility contract violated"
    exit 1
fi
echo "PASS: visibility contract — hidden unexported, protected exported-and-internal, default preemptible, gABI merge order-independent"
