#!/usr/bin/env bash
# AUD-CPU-1 follow-up gate: GOT-slot references to file-local symbols.
#
# The defect this pins was found while making the i686 runtime probe helper
# position independent: the assembler folded a local symbol out of GOT-slot
# relocations (`@GOT`/`@GOTPCREL`), so the linker computed the slot address
# from the *section* and added the symbol's offset a second time.  Measured on
# the files below: exit status 145 instead of 25 with GNU ld, and a fault under
# the LCCC linker.  `tests/data/got_slot_local_addend_{x86_64,i686}.s` read two
# locals of nonzero offset through their GOT slots and return 3*5 + 5*2 = 25;
# every pipeline must give exit status 25.
#
# The spelling half (x86-64): GAS 2.47 emits the relaxable
# `R_X86_64_{REX_,}GOTPCRELX` only for instruction forms a linker may rewrite
# (`mov` load, `test`, the eight binops in both directions, indirect
# `call/jmp/push`) and the plain `R_X86_64_GOTPCREL` for stores, `lea` and
# immediate forms.  LCCC matches that exactly, form by form; when the pinned
# GAS 2.47 oracle is present the gate compares the full (type, symbol) lists
# instead of trusting the table in this comment.  i686 emits the relaxable
# `R_386_GOT32X` for these `movl x@GOT(%reg)` loads exactly as GAS 2.47
# does (I686-GOT32X; the spelling gap this gate used to report is closed),
# and with the oracle present the i686 lists must agree exactly as well.
set -uo pipefail

CCC=$(realpath "${CCC:-./target/fastbuild/lccc}")
REPO=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$REPO"
GAS=${GAS_ORACLE:-$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/as}
GCC=${GCC_BIN:-gcc}
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT
fail=0
ok()   { printf 'ok   %s\n' "$*"; }
bad()  { printf 'FAIL %s\n' "$*"; fail=1; }
skip() { printf 'SKIP %s\n' "$*"; }

[ -x "$CCC" ] || { echo "FAIL: no compiler at $CCC"; exit 1; }

x64_src=tests/data/got_slot_local_addend_x86_64.s
i686_src=tests/data/got_slot_local_addend_i686.s
[ -f "$x64_src" ] && [ -f "$i686_src" ] || { echo "FAIL: missing test data"; exit 1; }

# ------------------------------------------------- 1. the values must be right
run_25() { # description
    local desc=$1
    if [ ! -x "$td/prog" ]; then bad "$desc: no binary produced"; return; fi
    "$td/prog"
    local rc=$?
    if [ "$rc" = 25 ]; then ok "$desc -> exit 25"; else bad "$desc -> exit $rc (want 25)"; fi
}

try_build() { # description, command...
    local desc=$1; shift
    : >"$td/build.log"
    if "$@" >"$td/build.log" 2>&1; then return 0; fi
    bad "$desc: build failed"; sed -n '1,4p' "$td/build.log" | sed 's/^/      /'; return 1
}

rm -f "$td/prog"
if try_build "x86-64 default" "$CCC" "$x64_src" -o "$td/prog"; then run_25 "x86-64 default (LCCC driver)"; fi
rm -f "$td/prog"
if try_build "x86-64 -no-pie" "$CCC" -no-pie "$x64_src" -o "$td/prog"; then run_25 "x86-64 -no-pie (LCCC driver)"; fi

printf 'int main(void) { return 0; }\n' >"$td/m32.c"
if "$GCC" -m32 -no-pie "$td/m32.c" -o "$td/m32probe" >/dev/null 2>&1; then
    has_m32_libc=1
else
    has_m32_libc=0
    cat >"$td/shim32.s" <<'EOF'
	.text
	.globl	_start
_start:
	call	main
	movl	%eax, %ebx
	movl	$1, %eax
	int	$0x80
	.weak	__x86.get_pc_thunk.bx
	.type	__x86.get_pc_thunk.bx, @function
__x86.get_pc_thunk.bx:
	movl	(%esp), %ebx
	ret
EOF
fi

rm -f "$td/prog"
if [ "$has_m32_libc" = 1 ]; then
    if try_build "i686 default" "$CCC" -m32 "$i686_src" -o "$td/prog"; then run_25 "i686 default (LCCC driver)"; fi
    rm -f "$td/prog"
    if try_build "i686 -no-pie" "$CCC" -m32 -no-pie "$i686_src" -o "$td/prog"; then run_25 "i686 -no-pie (LCCC driver)"; fi
else
    if try_build "i686 default" "$CCC" -m32 -nostdlib "$i686_src" "$td/shim32.s" -o "$td/prog"; then run_25 "i686 default (LCCC driver, freestanding)"; fi
    rm -f "$td/prog"
    if try_build "i686 -no-pie" "$CCC" -m32 -no-pie -nostdlib "$i686_src" "$td/shim32.s" -o "$td/prog"; then run_25 "i686 -no-pie (LCCC driver, freestanding)"; fi
fi

# Cross pipeline: the LCCC object, linked by GNU ld.  A fold defect shows up
# here as 145 (GNU ld doubles the offset) rather than as a fault, so it is the
# pipeline that isolates the assembler from the LCCC linker.
rm -f "$td/prog"
if try_build "x86-64 cross link" "$CCC" -c "$x64_src" -o "$td/x64.o" &&
   try_build "x86-64 cross link" "$GCC" -no-pie "$td/x64.o" -o "$td/prog"; then
    run_25 "x86-64 LCCC object + GNU ld"
fi
if [ "$has_m32_libc" = 1 ]; then
    rm -f "$td/prog"
    if try_build "i686 cross link" "$CCC" -m32 -c "$i686_src" -o "$td/i686.o" &&
       try_build "i686 cross link" "$GCC" -m32 -no-pie "$td/i686.o" -o "$td/prog"; then
        run_25 "i686 LCCC object + GNU ld"
    fi
elif command -v ld >/dev/null 2>&1; then
    rm -f "$td/prog"
    if try_build "i686 cross link" "$CCC" -m32 -c "$i686_src" -o "$td/i686.o" &&
       try_build "i686 cross link" "$CCC" -m32 -c "$td/shim32.s" -o "$td/shim32.o" &&
       try_build "i686 cross link" ld -m elf_i386 "$td/i686.o" "$td/shim32.o" -o "$td/prog"; then
        run_25 "i686 LCCC object + GNU ld (freestanding)"
    fi
else
    skip "i686 cross link (no gcc multilib or ld)"
fi

# ------------------------------------------------------ 2. relocation spelling
reloc_pairs() { # object, names... -> "TYPE SYMBOL" lines, sorted
    # `readelf -r --wide` prints the symbol name followed by "+ Addend", or as
    # the last field when the relocation has no addend.
    local obj=$1; shift
    local names
    names=$(printf '%s\n' "$@")
    readelf -r --wide "$obj" 2>/dev/null |
        awk -v want="$names" '
            BEGIN { n = split(want, a, "\n"); for (i = 1; i <= n; i++) wanted[a[i]] = 1 }
            $1 !~ /^[0-9a-f]+$/ { next }
            {
                name = ($(NF - 1) == "+" || $(NF - 1) == "-") ? $(NF - 2) : $NF
                if (name in wanted) print $3, name
            }
        ' | sort
}

if try_build "x86-64 object" "$CCC" -c "$x64_src" -o "$td/x64.o"; then
    want_x64=$(printf 'R_X86_64_REX_GOTPCRELX q1\nR_X86_64_REX_GOTPCRELX q2\n')
    got_x64=$(reloc_pairs "$td/x64.o" q1 q2)
    if [ "$got_x64" = "$want_x64" ]; then
        ok "x86-64: mov-load keeps the symbol with the relaxable spelling"
    else
        bad "x86-64 GOT reloc spelling"; printf '      want: %s\n      got:  %s\n' \
            "$(echo "$want_x64" | tr '\n' ';')" "$(echo "$got_x64" | tr '\n' ';')"
    fi
fi

# The instruction-form contract, form by form, against the pinned oracle.
cat >"$td/forms.s" <<'EOF'
	.text
	.globl f1
f1:	movq q1@GOTPCREL(%rip), %rax
	ret
	.globl f2
f2:	movl q2@GOTPCREL(%rip), %eax
	ret
	.globl f3
f3:	addq q3@GOTPCREL(%rip), %rax
	ret
	.globl f4
f4:	movl %eax, q4@GOTPCREL(%rip)
	ret
	.globl f5
f5:	pushq q5@GOTPCREL(%rip)
	ret
	.globl f6
f6:	cmpl $1, q6@GOTPCREL(%rip)
	ret
	.globl f7
f7:	leaq q7@GOTPCREL(%rip), %rax
	ret
	.data
	.globl q1
q1:	.quad 1
	.globl q2
q2:	.long 1
	.globl q3
q3:	.quad 1
	.globl q4
q4:	.long 1
	.globl q5
q5:	.quad 1
	.globl q6
q6:	.long 1
	.globl q7
q7:	.quad 1
EOF
if try_build "x86-64 form probe" "$CCC" -c "$td/forms.s" -o "$td/forms_l.o"; then
    want_forms='R_X86_64_REX_GOTPCRELX q1
R_X86_64_GOTPCRELX q2
R_X86_64_REX_GOTPCRELX q3
R_X86_64_GOTPCREL q4
R_X86_64_GOTPCRELX q5
R_X86_64_GOTPCREL q6
R_X86_64_GOTPCREL q7'
    got_forms=$(reloc_pairs "$td/forms_l.o" q1 q2 q3 q4 q5 q6 q7)
    # reloc_pairs sorts by "TYPE SYMBOL", so the expectation is sorted the same
    # way before comparing.
    want_forms=$(printf '%s\n' "$want_forms" | sort)
    if [ -n "$got_forms" ] && [ "$got_forms" = "$want_forms" ]; then
        ok "x86-64: all seven instruction forms match the GAS 2.47 spelling"
    else
        bad "x86-64 instruction-form spelling"
        diff <(printf '%s\n' "$want_forms") <(printf '%s\n' "$got_forms") |
            sed 's/^/      /' | sed -n '1,10p'
    fi
    if [ -x "$GAS" ]; then
        if "$GAS" --64 "$td/forms.s" -o "$td/forms_g.o" 2>/dev/null; then
            oracle=$(reloc_pairs "$td/forms_g.o" q1 q2 q3 q4 q5 q6 q7)
            if [ -z "$oracle" ]; then
                bad "the pinned oracle's own relocations could not be read"
            elif [ "$oracle" = "$got_forms" ]; then
                ok "x86-64: exact agreement with the pinned GAS 2.47 oracle (7 rows)"
            else
                bad "x86-64: oracle disagreement (GAS 2.47 vs LCCC)"
                diff <(printf '%s\n' "$oracle") <(printf '%s\n' "$got_forms") |
                    sed 's/^/      /' | sed -n '1,10p'
            fi
        else
            bad "oracle present but could not assemble the form probe"
        fi
    else
        skip "GAS 2.47 oracle not installed (pinned table checked instead)"
    fi
fi

# i686: values are checked above; the relaxable `mov` loads must carry
# GAS's spelling, R_386_GOT32X, with the local symbol kept.
if try_build "i686 object" "$CCC" -m32 -c "$i686_src" -o "$td/i686.o"; then
    if [ "$(reloc_pairs "$td/i686.o" q1 q2)" = "$(printf 'R_386_GOT32X q1\nR_386_GOT32X q2\n')" ]; then
        ok "i686: GOT32X keeps the symbol (values verified above)"
    else
        bad "i686 GOT reloc spelling"; reloc_pairs "$td/i686.o" q1 q2 | sed 's/^/      /'
    fi
    if [ -x "$GAS" ]; then
        if "$GAS" --32 "$i686_src" -o "$td/i686_g.o" 2>/dev/null; then
            oracle=$(reloc_pairs "$td/i686_g.o" q1 q2)
            if [ -n "$oracle" ] && [ "$oracle" = "$(reloc_pairs "$td/i686.o" q1 q2)" ]; then
                ok "i686: exact agreement with the pinned GAS 2.47 oracle"
            else
                bad "i686: oracle disagreement (GAS 2.47 vs LCCC)"
                diff <(printf '%s\n' "$oracle") <(reloc_pairs "$td/i686.o" q1 q2) |
                    sed 's/^/      /' | sed -n '1,10p'
            fi
        else
            bad "oracle present but could not assemble the i686 probe"
        fi
    fi
fi

if [ "$fail" != 0 ]; then
    echo "GOT-slot local-symbol gate: FAIL"
    exit 1
fi
echo "GOT-slot local-symbol gate: PASS"
