#!/usr/bin/env bash
# check_arm_adr_shared_contract.sh — the shared-library ADR relocation
# contract (PR #821 F3, review of 2026-10-14): "non-preemptible" does NOT
# prove "section-relative".
#
# THE COUNTEREXAMPLE (model + linked-object reproducer)
# -----------------------------------------------------
# ADR_PREL_LO21 against the ELF null symbol (index 0), a named local
# UNDEFINED symbol, or a symbol whose defining input section was discarded
# all resolve to a numeric S = 0.  S = 0 passes the signed-21-bit
# displacement check, so the pre-fix linker produced a nominally valid .so
# whose ADR computes B + S at RUNTIME instead of S: with load bias
# B = 0x70000000 the register ends up holding 0x70000000, not 0.  The Review
# AI's model was confirmed here by LINKING (no hand analysis): the pre-fix
# binary linked sym0.craft.o with exit 0.
#
# THE CONTRACT (real link commands against the real linker)
#   1. ADR_PREL_LO21, sym index 0     -> link-time diagnostic, exit != 0
#   2. ADR_PREL_LO21, local undefined -> link-time diagnostic, exit != 0
#   3. ADR_PREL_LO21, valid local     -> links, exit == 0
#   4. NONE relocations (sym 0) stay  -> untouched by the ADR policy
#      legal (padding/alignment)
#
# Mutation check (--self-test): a fake linker that accepts the sym-0 object
# (the pre-fix behavior) must fail row 1.
set -uo pipefail

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
SELF=$(cd "$(dirname "$0")" && pwd)/$(basename "$0")
LDDD=${CCC_ARM_LD:-$ROOT/target/fastbuild/lccc-arm}
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

# --- Object crafting: minimal ELF64 AArch64 ET_REL with one ADR_PREL_LO21 ----
# usage: craft <out.o> <sym_idx> <st_shndx> [reloc_type] [num_syms]
craft() {
    python3 - "$@" <<'PYEOF'
import struct, sys
path, sym_idx, st_shndx = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
rtype = int(sys.argv[4]) if len(sys.argv) > 4 else 274  # R_AARCH64_ADR_PREL_LO21
EM_AARCH64 = 183
text = struct.pack('<II', 0x10000000, 0xD65F03C0)  # adr x0, #0 ; ret
rela = struct.pack('<QQq', 0, (sym_idx << 32) | rtype, 0)
sym_null = struct.pack('<IBBHQQ', 0, 0, 0, 0, 0, 0)
# STB_LOCAL, STT_FUNC if defined else STT_NOTYPE, shndx, value 0
st_info = (0 << 4) | (2 if st_shndx else 0)
sym_f = struct.pack('<IBBHQQ', 1, st_info, 0, st_shndx, 0, 1)
symtab = sym_null + sym_f
strtab = b'\x00f\x00'
shstrtab = b'\x00.text\x00.rela.text\x00.symtab\x00.strtab\x00.shstrtab\x00'
names = {'.text': 1, '.rela.text': 7, '.symtab': 18, '.strtab': 26, '.shstrtab': 34}
off = 64
def align(x, a=8): return (x + a - 1) & ~(a - 1)
offs = []
for blob in (text, rela, symtab, strtab, shstrtab):
    off = align(off); offs.append(off); off += len(blob)
shoff = align(off)
eh = bytearray(struct.pack('<16sHHIQQQIHHHHHH',
    b'\x7fELF\x02\x01\x01\x00' + b'\x00' * 8, 1, EM_AARCH64, 1, 0, 64, 0,
    0, 64, 0, 0, 64, 6, 5))
struct.pack_into('<Q', eh, 0x28, shoff)
out = bytearray(eh)
for o, blob in zip(offs, (text, rela, symtab, strtab, shstrtab)):
    while len(out) < o: out.append(0)
    out += blob
while len(out) < shoff: out.append(0)
def shdr(name, typ, flags, addr, offset, size, link, info, al, es):
    return struct.pack('<IIQQQQIIQQ', name, typ, flags, addr, offset, size, link, info, al, es)
sh = shdr(0, 0, 0, 0, 0, 0, 0, 0, 0, 0)
sh += shdr(names['.text'], 1, 0x6, 0, offs[0], len(text), 0, 0, 4, 0)
sh += shdr(names['.rela.text'], 4, 0x40, 0, offs[1], len(rela), 3, 1, 8, 24)
sh += shdr(names['.symtab'], 2, 0, 0, offs[2], len(symtab), 4, 1, 8, 24)
sh += shdr(names['.strtab'], 3, 0, 0, offs[3], len(strtab), 0, 0, 1, 0)
sh += shdr(names['.shstrtab'], 3, 0, 0, offs[4], len(shstrtab), 0, 0, 1, 0)
out += sh
open(path, 'wb').write(bytes(out))
PYEOF
}

if [[ ${1:-} == --self-test ]]; then
    # A fake arm driver that links everything (the pre-fix acceptance).
    cat > fake-link-all <<'EOF'
#!/usr/bin/env bash
# find -o ARG
prev=""; out=""
for a in "$@"; do case "$prev" in -o) out=$a;; esac; prev=$a; done
[[ -n "$out" ]] && printf 'ELF-fake-so' > "$out"
exit 0
EOF
    chmod +x fake-link-all
    craft sym0.craft.o 0 0 >/dev/null
    out=$(CCC_ARM_LD="$PWD/fake-link-all" bash "$SELF" 2>&1)
    if grep -q '^FAIL null-symbol ADR: rejected with diagnostic' <<<"$out"; then
        echo "PASS self-test (pre-fix acceptance is caught)"
    else
        echo "FAIL self-test: the fake pre-fix linker was NOT rejected"
        fail=1
    fi
    exit $fail
fi

[[ -x "$LDDD" ]] || { echo "FAIL: lccc-arm not built at $LDDD" >&2; exit 1; }

# --- Row 1: the null-symbol counterexample must be rejected ------------------
craft sym0.craft.o 0 0
"$LDDD" -shared -o libsym0.so sym0.craft.o 2>err; rc=$?
step "null-symbol ADR: rejected with diagnostic" '(( rc != 0 )) && grep -q "null symbol" err'

# --- Row 2: named local undefined symbol must be rejected --------------------
craft symu.craft.o 1 0
"$LDDD" -shared -o libsymu.so symu.craft.o 2>err; rc=$?
step "local-undefined ADR: rejected with diagnostic" '(( rc != 0 )) && grep -qE "surviving section|undefined" err'

# --- Row 3: a valid local symbol still links ---------------------------------
craft sym1.craft.o 1 1
"$LDDD" -shared -o libsym1.so sym1.craft.o 2>err; rc=$?
step "valid local ADR: links" '(( rc == 0 )) && [[ -f libsym1.so ]]'

# --- Row 4: R_AARCH64_NONE (0) padding relocations stay untouched ------------
# A NONE relocation carries sym 0 legally (gas padding, --emit-relocs output);
# the ADR policy must not reject it.
python3 - <<'PYEOF'
import struct
d = bytearray(open('sym0.craft.o', 'rb').read())
# rewrite the single .rela.text entry: type 0 (NONE), sym 0
shoff = struct.unpack_from('<Q', d, 0x28)[0]
# section headers: 6 x 64; .rela.text is index 2
rela_sh = shoff + 2 * 64
off = struct.unpack_from('<Q', d, rela_sh + 24)[0]
struct.pack_into('<QQq', d, off, 0, (0 << 32) | 0, 0)
open('symnone.craft.o', 'wb').write(bytes(d))
PYEOF
"$LDDD" -shared -o libnone.so symnone.craft.o 2>err; rc=$?
step "NONE relocation with sym 0: still links" '(( rc == 0 ))'

if (( fail )); then
    echo "FAIL: shared-library ADR contract violated"
    exit 1
fi
echo "PASS: shared ADR contract — null/local-undefined rejected at link time, valid locals link, NONE untouched"
