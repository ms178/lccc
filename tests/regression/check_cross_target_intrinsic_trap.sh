#!/usr/bin/env bash
# AUD-SILENT-ZERO-1: an intrinsic the target cannot execute must fail LOUDLY.
#
# The AArch64 and RISC-V intrinsic emitters had an "x86-only" arm that zeroed
# the 16-byte destination slot ("zero dest if present") and emitted NOTHING AT
# ALL when the destination was register-allocated, so a program using an x86
# intrinsic on those targets computed a silently wrong value with rc=0 (an
# all-zero vector, or a register's leftovers).
#
# The gate pins the replacement contract, which a runtime test cannot: this
# host cannot execute AArch64/RISC-V code (no qemu), so the assertions are on
# the emitted instruction stream AND on the encoded object, i.e. the target's
# trap must (a) be the only thing the unlowerable intrinsic emits, (b) survive
# the whole pipeline at -O2, (c) assemble -- a trap the assembler rejects
# would be worse than the zeros -- and (d) not appear for anything else.
#
# A pre-fix compiler fails at (2): it emits `stp xzr, xzr, [x9]` / `sd zero,`
# instead of a trap, and no trap encoding anywhere in the object.
set -euo pipefail

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
ccc=${CCC:-$repo/target/fastbuild/lccc}
ccc_arm=${CCC_ARM:-$repo/target/fastbuild/lccc-arm}
ccc_riscv=${CCC_RISCV:-$repo/target/fastbuild/lccc-riscv}
ccc_i686=${CCC_I686:-$repo/target/fastbuild/lccc-i686}
src=$repo/tests/regression/cross_target_intrinsics/x86_only.c
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

for tool in "$ccc_arm" "$ccc_riscv" "$ccc_i686" "$ccc"; do
    [ -x "$tool" ] || { echo "SKIP: $tool not built" >&2; exit 0; }
done

fail() { echo "FAIL: $*" >&2; exit 1; }

function_body() {
    local function=$1 file=$2
    awk -v wanted="$function" '
        $0 == wanted ":" { active=1 }
        active { print }
        active && $0 == ".size " wanted ", .-" wanted { exit }
    ' "$file"
}

# Assert one of the given encodings is present in the object's EXECUTABLE
# sections.  A whole-file byte search would be satisfied by a coincidental
# pattern in a symbol table, so the section table is walked properly, and both
# the full and the compressed (halfword) form of a value are accepted.
trap_in_object() {
    local object=$1
    shift
    python3 - "$object" "$@" <<'PYEOF'
import struct
import sys

path, *wanted = sys.argv[1:]
data = open(path, 'rb').read()
if data[:4] != b'\x7fELF':
    sys.exit(f'{path}: not an ELF file')
is64 = data[4] == 2
little = data[5] == 1
end = '<' if little else '>'
if is64:
    (e_shoff,) = struct.unpack_from(end + 'Q', data, 0x28)
    e_shentsize, e_shnum, e_shstrndx = struct.unpack_from(end + 'HHH', data, 0x3A)
    fmt = end + 'IIQQQQIIQQ'
else:
    (e_shoff,) = struct.unpack_from(end + 'I', data, 0x20)
    e_shentsize, e_shnum, e_shstrndx = struct.unpack_from(end + 'HHH', data, 0x2E)
    fmt = end + 'IIIIIIIIII'


def section(index):
    return struct.unpack_from(fmt, data, e_shoff + index * e_shentsize)


_, _, _, _, str_off, str_size, _, _, _, _ = section(e_shstrndx)
strtab = data[str_off:str_off + str_size]

code = b''
for i in range(e_shnum):
    name_off = section(i)[0]
    sh_flags, sh_offset, sh_size = section(i)[2], section(i)[4], section(i)[5]
    name = strtab[name_off:strtab.index(b'\0', name_off)].decode() if name_off < len(strtab) else ''
    if sh_flags & 0x4 and sh_flags & 0x2:              # SHF_EXECINSTR | SHF_ALLOC
        code += data[sh_offset:sh_offset + sh_size]
        _ = name

for word in wanted:
    value = int(word, 0)
    for width in (4, 2):                               # full and compressed form
        if value >= 1 << (8 * width):
            continue
        packed = struct.pack(end + 'I', value)
        if (packed if little else packed[::-1])[:width] in code or packed[:width] in code:
            sys.exit(0)
print(f'{path}: none of {" ".join(wanted)} is encoded in an executable section', file=sys.stderr)
sys.exit(1)
PYEOF
}

for opt in -O0 -O2; do
    tag=${opt#-}

    # 1. AArch64: the x86-only intrinsic traps, and nothing else does.
    "$ccc_arm" "$opt" -S "$src" -o "$tmp/arm$tag.s"
    body=$(function_body x86_only_enc "$tmp/arm$tag.s")
    grep -qE '^[[:space:]]+brk[[:space:]]' <<<"$body" ||
        { printf '%s\n' "$body" >&2; fail "AArch64 $opt: x86-only intrinsic did not trap"; }
    grep -qE '^[[:space:]]+(stp|xor)[[:space:]]+.*\bzr\b' <<<"$body" &&
        fail "AArch64 $opt: x86-only intrinsic still zero-fills its destination"
    [ "$(grep -cE '^[[:space:]]+brk[[:space:]]' "$tmp/arm$tag.s")" = 1 ] ||
        fail "AArch64 $opt: the trap leaked outside the unlowerable intrinsic"
    for fn in portable_sqrt portable_fabs; do
        # Capture, then match a here-string: `producer | grep -q` would let
        # grep exit early and SIGPIPE the producer, which `set -o pipefail`
        # turns into a failure of the gate itself (the repo's own
        # pipefail-sigpipe gate rejects that shape).
        body=$(function_body "$fn" "$tmp/arm$tag.s")
        grep -qE '^[[:space:]]+brk[[:space:]]' <<<"$body" &&
            fail "AArch64 $opt: $fn was trapped although AArch64 can compute it"
    done
    body=$(function_body portable_sqrt "$tmp/arm$tag.s")
    grep -qE '^[[:space:]]+fsqrt[[:space:]]' <<<"$body" ||
        fail "AArch64 $opt: portable_sqrt no longer lowers to fsqrt"
    "$ccc_arm" "$opt" -c "$src" -o "$tmp/arm$tag.o"
    trap_in_object "$tmp/arm$tag.o" 0xd4200000 ||
        fail "AArch64 $opt: brk #0 is not encoded in the object"

    # 2. RISC-V: the same contract with its own trap and zero-fill spelling.
    "$ccc_riscv" "$opt" -S "$src" -o "$tmp/riscv$tag.s"
    body=$(function_body x86_only_enc "$tmp/riscv$tag.s")
    grep -qE '^[[:space:]]+ebreak$' <<<"$body" ||
        { printf '%s\n' "$body" >&2; fail "RISC-V $opt: x86-only intrinsic did not trap"; }
    grep -qE '^[[:space:]]+(sd|sw)[[:space:]]+zero' <<<"$body" &&
        fail "RISC-V $opt: x86-only intrinsic still zero-fills its destination"
    [ "$(grep -cE '^[[:space:]]+ebreak$' "$tmp/riscv$tag.s")" = 1 ] ||
        fail "RISC-V $opt: the trap leaked outside the unlowerable intrinsic"
    body=$(function_body portable_sqrt "$tmp/riscv$tag.s")
    grep -qE '^[[:space:]]+fsqrt' <<<"$body" ||
        fail "RISC-V $opt: portable_sqrt no longer lowers to fsqrt"
    "$ccc_riscv" "$opt" -c "$src" -o "$tmp/riscv$tag.o"
    trap_in_object "$tmp/riscv$tag.o" 0x00100073 0x9002 ||
        fail "RISC-V $opt: ebreak is not encoded in the object"
done

# 3. Positive controls: the x86 family must keep computing the intrinsic, so
#    this cannot be "fixed" by trapping every target.  `-maes` is required:
#    AUD-CPU-3 makes an x86 target without the AES permission reject the
#    intrinsic (GCC does the same), so the control must ask for it.
for tool in "$ccc_i686" "$ccc"; do
    out=$tmp/$(basename "$tool").s
    "$tool" -O2 -maes -S "$src" -o "$out"
    grep -qE '^[[:space:]]+v?aesenc([[:space:]]|$)' "$out" ||
        { printf '%s\n' "$(function_body x86_only_enc "$out")" >&2;
          fail "$(basename "$tool"): AES-NI intrinsic no longer lowers to aesenc"; }
    grep -qE '^[[:space:]]+(brk|ebreak|ud2)' "$out" &&
        fail "$(basename "$tool"): trapped an intrinsic this target can compute"
done

echo "PASS: x86-only intrinsics trap on AArch64/RISC-V and still lower on x86"
