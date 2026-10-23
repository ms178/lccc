#!/usr/bin/env bash
# AUD-SILENT-ZERO-1: an intrinsic the target cannot execute must fail LOUDLY.
#
# The AArch64 and RISC-V intrinsic emitters carried an "x86-only" arm that
# answered "zero dest if present": 16 bytes of zeros stored ONLY when the
# destination was a known stack slot, and NOTHING AT ALL when it was
# register-allocated.  Compiling an x86 intrinsic for either target therefore
# produced a wrong value with rc=0 and no diagnostic (measured: `stp xzr, xzr,
# [x9]` / `sd zero,`).  Two layers now stand against that, and both are pinned
# here:
#
#   1. THE COMPILE-TIME CONTRACT.  Lowering rejects an x86-only builtin on a
#      target that is not x86, naming the builtin and the architecture, and the
#      driver fails the translation unit (rc=1) -- GCC rejects the same
#      programs, because the name does not exist there.  Code that guards the
#      call with `#ifdef __x86_64__` still compiles: the diagnostic fires only
#      when the call is actually lowered.
#
#   2. THE BACKEND SAFETY NET.  The emitters no longer contain a silent arm or
#      an `unimplemented!` ICE: an unlowerable op emits the target's own trap
#      (`brk #0` / `ebreak`), so a future frontend gate that leaks one through
#      yields a program that dies at the instruction instead of computing
#      garbage.  This host cannot execute AArch64/RISC-V code (no qemu), so the
#      trap is pinned where a runtime test cannot reach it: the instruction
#      stream AND the encoded object (a trap the assembler rejects would be
#      worse than the zeros it replaced).
#
# A pre-fix compiler fails at (1) with rc=0 and no message, and its object has
# no trap encoding at all.
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

# 1. The compile-time contract: x86-only builtins are unavailable off x86, and
#    the message says which builtin and which architecture.  The i686/riscv
#    spelling of the target name comes from the same helper the diagnostic uses.
for opt in -O0 -O2; do
    for pair in "arm:$ccc_arm:aarch64" "riscv:$ccc_riscv:riscv64"; do
        label=${pair%%:*}; rest=${pair#*:}; tool=${rest%%:*}; arch=${rest##*:}
        # Capture first: reading the exit status of a pipeline would report the
        # last stage, and `set -o pipefail` makes `producer | grep -q` a
        # self-inflicted red (the repo gates that shape explicitly).
        set +e
        err=$("$tool" "$opt" -S "$src" -o "$tmp/$label$opt.s" 2>&1)
        rc=$?
        set -e
        [ "$rc" -eq 1 ] ||
            fail "lccc-$label $opt: x86-only intrinsic compiled with rc=$rc (expected 1)"
        # The rejection must name the target architecture (GCC's own shape:
        # "'_mm_add_ps' is not available on aarch64") — the wording itself is
        # the compiler's, the contract is that the user learns both which
        # builtin and which target.
        grep -qF "is not available on $arch" <<<"$err" ||
            { printf '%s\n' "$err" >&2;
              fail "lccc-$label $opt: diagnostic does not name the builtin's target ($arch)"; }
        grep -qF "__builtin_ia32_aesenc128" <<<"$err" ||
            { printf '%s\n' "$err" >&2;
              fail "lccc-$label $opt: diagnostic does not name the builtin"; }
        grep -qE "internal error|panicked" <<<"$err" &&
            { printf '%s\n' "$err" >&2; fail "lccc-$label $opt: internal error instead of a diagnostic"; }
        true
    done
done

# 1b. The same contract, family by family: one representative of every x86-only
#     family the frontend accepts (AES-NI, PCLMUL, CRC32, VAES-256, VPCLMULQDQ,
#     AVX float/double, AVX2 broadcast, and the no-operand VEX op the AArch64
#     catch-all used to swallow silently).  Counting the diagnostics is the
#     point: nine functions must produce nine independent rejections, so a gate
#     that accidentally only checks the first one cannot pass.
families=$repo/tests/regression/cross_target_intrinsics/x86_families.c
family_count=$(grep -c '^[a-z].*x86_fam_' "$families")
for opt in -O0 -O2; do
    for pair in "arm:$ccc_arm:aarch64" "riscv:$ccc_riscv:riscv64"; do
        label=${pair%%:*}; rest=${pair#*:}; tool=${rest%%:*}; arch=${rest##*:}
        set +e
        err=$("$tool" "$opt" -S "$families" -o "$tmp/fam-$label$opt.s" 2>&1)
        rc=$?
        set -e
        [ "$rc" -eq 1 ] ||
            fail "lccc-$label $opt: an x86-only intrinsic family compiled with rc=$rc (expected 1)"
        got=$(grep -cF "is not available on $arch" <<<"$err" || true)
        [ "$got" -eq "$family_count" ] ||
            { printf '%s\n' "$err" >&2;
              fail "lccc-$label $opt: $got of $family_count families were rejected with a diagnostic"; }
        grep -qE "internal error|panicked" <<<"$err" &&
            { printf '%s\n' "$err" >&2; fail "lccc-$label $opt: internal error instead of a diagnostic"; }
        true
    done
done

# 2. Portable builtins are NOT collateral damage: the same file's
#    portable_sqrt/portable_fabs must still compile for every target.
for opt in -O0 -O2; do
    tag=${opt#-}
    for tool in "$ccc_arm" "$ccc_riscv" "$ccc_i686" "$ccc"; do
        out=$tmp/portable-$(basename "$tool")$tag.s
        "$tool" "$opt" -S "$repo/tests/regression/cross_target_intrinsics/portable_only.c" -o "$out" 2>"$out.err" ||
            { cat "$out.err" >&2; fail "$(basename "$tool") $opt: rejected a portable builtin"; }
    done
    # Each non-x86 target must still LOWER the portable builtin, not merely
    # accept the file: asserting the arch's own square-root instruction is what
    # separates "the target gate spared this builtin" from "it produced
    # something". Under the default -fmath-errno, `sqrt` of a negative input
    # must set errno, so a call to `sqrt` is part of every correct lowering:
    # the whole function at -O0 (GCC 16.2 emits exactly `bl sqrt`), the
    # never-taken slow path behind the instruction at -O2. With
    # -fno-math-errno the instruction alone is the lowering at every level.
    for spec in "AArch64:$ccc_arm:fsqrt:bl" "RISC-V:$ccc_riscv:fsqrt\\.d:call"; do
        IFS=: read -r arch tool insn call <<<"$spec"
        body=$(function_body portable_sqrt "$tmp/portable-$(basename "$tool")$tag.s")
        grep -qE "^[[:space:]]+$call[[:space:]]+sqrt\$" <<<"$body" ||
            fail "$arch $opt: portable_sqrt lost its errno path (no call to sqrt)"
        if [ "$opt" = -O2 ]; then
            grep -qE "^[[:space:]]+$insn[[:space:]]" <<<"$body" ||
                fail "$arch $opt: portable_sqrt no longer lowers to $insn"
        fi
        out=$tmp/portable-nme-$(basename "$tool")$tag.s
        "$tool" "$opt" -fno-math-errno -S "$repo/tests/regression/cross_target_intrinsics/portable_only.c" \
            -o "$out" 2>"$out.err" ||
            { cat "$out.err" >&2; fail "$arch $opt -fno-math-errno: rejected a portable builtin"; }
        body=$(function_body portable_sqrt "$out")
        grep -qE "^[[:space:]]+$insn[[:space:]]" <<<"$body" ||
            fail "$arch $opt -fno-math-errno: portable_sqrt no longer lowers to $insn"
        ! grep -qE "^[[:space:]]+$call[[:space:]]+sqrt\$" <<<"$body" ||
            fail "$arch $opt -fno-math-errno: portable_sqrt still calls sqrt"
    done
done

# 3. Positive controls: an x86 target that HAS the extension keeps lowering the
#    intrinsic.  AUD-CPU-3 is what makes `-maes` necessary here, and the
#    i686 leg proves the target gate did not turn into "reject on 32-bit".
for spec in "$ccc:-maes:" "$ccc_i686:-maes:"; do
    tool=${spec%%:*}; flag=${spec#*:}; flag=${flag%%:*}
    out=$tmp/$(basename "$tool").s
    "$tool" -O2 "$flag" -S "$src" -o "$out" 2>"$out.err" ||
        { cat "$out.err" >&2; fail "$(basename "$tool"): rejected an intrinsic this target supports"; }
    grep -qE '^[[:space:]]+v?aesenc([[:space:]]|$)' "$out" ||
        { printf '%s\n' "$(function_body x86_only_enc "$out")" >&2;
          fail "$(basename "$tool"): AES-NI intrinsic no longer lowers to aesenc"; }
done

# 4. The safety net itself: the target's trap must assemble and encode.  The
#    intrinsic path is no longer reachable from source (layer 1 rejects it
#    first), so this pins the mechanism the emitters rely on -- an explicit
#    trap, which is the same instruction `emit_unreachable` emits for an
#    unlowerable op.
for spec in "arm:$ccc_arm:0xd4200000" "riscv:$ccc_riscv:0x00100073"; do
    label=${spec%%:*}; rest=${spec#*:}; tool=${rest%%:*}; enc=${rest##*:}
    printf 'void lccc_trap_probe(void) { __builtin_trap(); }\n' >"$tmp/trap.c"
    "$tool" -O1 -S "$tmp/trap.c" -o "$tmp/trap-$label.s"
    body=$(function_body lccc_trap_probe "$tmp/trap-$label.s")
    case $label in
        arm) grep -qE '^[[:space:]]+brk[[:space:]]' <<<"$body" ||
                 { printf '%s\n' "$body" >&2; fail "AArch64: the trap is no longer brk"; } ;;
        riscv) grep -qE '^[[:space:]]+ebreak$' <<<"$body" ||
                 { printf '%s\n' "$body" >&2; fail "RISC-V: the trap is no longer ebreak"; } ;;
    esac
    "$tool" -O1 -c "$tmp/trap.c" -o "$tmp/trap-$label.o"
    trap_in_object "$tmp/trap-$label.o" "$enc" ||
        fail "$label: the trap is not encoded in the object"
done

echo "PASS: x86-only intrinsics are rejected off x86 with a diagnostic, portable ones lower, and the trap safety net encodes"
