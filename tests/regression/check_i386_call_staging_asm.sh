#!/usr/bin/env bash
# i386 register-argument staging, asserted on the emitted assembly.
#
# Why this file exists: `check_i386_regparm_small_struct_abi.sh` measures the
# *answer* a program gives, and it is now a latch on a closed class (budget 0).  An
# answer cannot see two things, and both of them are the reason the staging path is
# delicate:
#
#   1. A capture that overwrites an ABI source register before every parameter has
#      been read from it.  The value can still come out right -- and the wrong-value
#      face is exactly what regparm miscompiles have historically hidden behind,
#      because a caller that stages through %eax while another parameter is still
#      waiting in %eax produces a *plausible* number, not a crash.
#      `i686/codegen/prologue.rs` records the rule this checks: params are walked in
#      descending ABI-register order because "%ecx/%edx are themselves allocatable
#      (caller-saved phase), so a pre-store target could be a later param's
#      still-unread source".  This gate asserts the *property* that rule protects,
#      not the order: an order that keeps sources intact until their last read is
#      correct, and pinning the sequence would make a legal reordering a failure.
#   2. The size of the moves.  A 12-byte packed {unsigned long long; float} argument
#      has to be staged as three integer words on -m32 (regparm=3 has exactly three
#      of them, which is why gcc emits no SSE move at all there) and as
#      [%rdi | %xmm0] on -m64, where the float half must be a 4-byte `movss`.  A
#      16-byte `movaps` over a 12-byte object is a read past the end of the argument
#      -- legal-looking, wrong, and invisible to an exit status.
#
# Both fixtures are compiled with `-S` only, so the assembly half of this gate needs
# no 32-bit runtime; the one arm that *executes* (values must match the oracle) goes
# through tests/regression/i386_exec.sh and reports a host without multilib as a
# capability skip, never as a compiler failure.  `LCCC_REQUIRE_I686=1` turns that
# skip into a failure, for CI on a host that is supposed to have it.
set -uo pipefail
cd "$(dirname "$0")/../.." || exit 2

ROOT=$(pwd)
LCCC=${LCCC:-$ROOT/target/fastbuild/lccc}
CC=${CC:-gcc}
TIMEOUT=${TIMEOUT:-60}
source "$ROOT/tests/regression/i386_exec.sh"



TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

# ---------------------------------------------------------------- the scanner
#
# For one function's asm, check that the window feeding each `call` leaves the three
# regparm sources intact until they have been read for the last time.  Canonical
# register groups, so that `movb %al, ...` counts as a read of the %eax group and a
# write to %ax invalidates a later read of %edx's *neighbour* not at all -- the point
# is that a 32-bit destination spoils every narrower name it contains.
clobber_scan() { # $1 = asm file, $2 = function label -> prints violations
    awk -v fn="$2" '
        function grp(r) {
            if (r ~ /^(eax|ax|al|ah)$/) return "A"
            if (r ~ /^(edx|dx|dl|dh)$/) return "D"
            if (r ~ /^(ecx|cx|cl|ch)$/) return "C"
            return ""
        }
        # operand -> "src dst" in AT&T order; the last field is the destination.
        /^[A-Za-z_.][A-Za-z0-9_.]*:/ {
            if ($0 !~ /^\.L/) infn = ($0 == fn ":")
            next
        }
        !infn { next }
        /^[ \t]+[^ \t.#]/ { nlines++ }               # body-presence guard, reported in END
        /^[ \t]*[#.;]/ { next }                      # comments and directives
        !/^[ \t]+[^ \t]/ { next }
        { nr_now = NR }
        {
            line = $0
            sub(/^[ \t]+/, "", line)
            sub(/[ \t]+#.*$/, "", line)
            m = match(line, /^[a-z][a-z0-9]*[a-z0-9]/)
            op = (m ? substr(line, 1, RLENGTH) : "")
            rest = substr(line, length(op) + 1)
            sub(/^[ \t]+/, "", rest)
            # `op ~ /^\./` for the assembler directives that survive the filters above.
            # This used to be `/^\.*/`, which matches the empty string and therefore
            # *every* opcode: the scanner skipped every instruction, printed nothing,
            # and read as a pass on a host with no violations -- which the canned
            # violation transcript in --self-test is the only reason it got caught.
            if (op == "" || op == "call" || op == "jmp" || op == "ret" || op ~ /^\./) {
                if (op == "call") emit_window()
                next
            }
            n = split(rest, parts, ",")
            if (op == "push" || op == "pushl" || op == "pushw" || op == "pushb") {
                note_read(parts[1]); next
            }
            if (op == "pop" || op == "popl" || op == "popw" || op == "popb") {
                note_write(parts[1]); next
            }
            if (n < 2) { next }
            for (i = 1; i < n; i++) note_read(parts[i])
            note_write(parts[n])
            next
        }
        function note_read(operand,   t, g) {
            t = operand
            # Only a *register* operand reads a source register: `movl 8(%esp), %eax`
            # reads memory, and a displacement through %ebx/%esp is an address, not
            # one of the three argument registers we are protecting.
            g = reg_group(t)
            if (g != "") {
                reads[g] = nr_now
                # Any read of an ABI register makes this window a staging window;
                # without this flag the check below reports nothing anywhere, which
                # is exactly the "silently green" shape the self-test exists to catch.
                seen = 1
            }
        }
        function note_write(operand,   g) {
            g = reg_group(operand)
            if (g != "" && !(g in firstw)) firstw[g] = nr_now
        }
        END {
            if (!nlines) print "EMPTY: no instructions found for " fn " -- a body the scanner" \
                  " cannot locate is not a body that passed"
        }
        function reg_group(operand,   t) {
            t = operand
            gsub(/[ \t]/, "", t)
            if (t !~ /^%/) return ""
            sub(/^%/, "", t)
            # %fs:(%eax) and friends: the segment part is not the register.
            sub(/^.*:/, "", t)
            if (t ~ /^\(/) return ""
            return grp(t)
        }
        function emit_window(   g, bad) {  # see `nlines` for the body-presence guard
            # Only a window that actually reads the ABI registers is a staging
            # window; anything else (a tail call, a call in a block that receives its
            # values from memory) has nothing to say about the law.
            if (!seen) return
            bad = ""
            for (g = 0; g < 3; g++) {
                k = substr("ADC", g + 1, 1)
                if (!(k in reads)) continue
                if ((k in firstw) && firstw[k] < reads[k])
                    bad = bad " " k "(" firstw[k] "<" reads[k] ")"
            }
            if (bad != "") print "CLOBBER@" nr_now ":" bad
            seen = 0
            delete reads
            delete firstw
        }
    ' "$1"
}

# `emit_window`/`reset_window` above are the contract; the awk is deliberately a
# small recognizer rather than a parser: it only ever asks "was this group written
# before its last read", so an instruction it does not understand is skipped, not
# guessed at.  A skipped instruction can only produce a false pass, never a false
# failure, which is the direction that has to be chosen here -- and the self-test
# below asserts the scanner still fires on a real violation.

# ---------------------------------------------------------------- asm-level arms
#
# One law per row, read off the emitted assembly, no runtime needed.
#
# A `class64` row used to sit here asserting that the float half of the 12-byte
# struct has to travel in an SSE register.  It does not, and measurement is what
# said so: `struct P { unsigned long long w; float f; } __attribute__((packed))`
# places the float at offset 8, inside the eight-byte word the u64 already spans, and
# the psABI merges a word to the higher-precedence class -- INTEGER beats SSE -- so a
# *packed* mixed pair is two integer words and `movss` at the call site would be the
# bug.  gcc agrees (measured: `movl $1, %eax; movl $1, %edx; movl $1077936128, %ecx`
# at -m32 -mregparm=3; %rdi and %rsi at -m64).  A check that pins an intuition about
# classification fails on correct code, so the classification is asserted where it can
# only mean one thing: at the boundary, in the cross-compiler arm below.
asm_case() { # $1 = fixture, $2 = flags, $3 = function, $4 = checker -> runs one check
    local fx=$1 flags=$2 fn=$3 check=$4
    out_asm=$TMP/$(basename "$fx").$(printf '%s' "$flags" | tr -c 'A-Za-z0-9' '_').s
    if ! timeout "$TIMEOUT" "$LCCC" -std=gnu17 $flags -S "$fx" -o "$out_asm" 2>"$TMP/err"; then
        head -3 "$TMP/err" >&2
        printf 'COMPILE-FAIL'
        return 1
    fi
    case "$check" in
        clobber) clobber_scan "$out_asm" "$fn" ;;
        class32) class32_scan "$out_asm" "$fn" ;;
        *) printf 'UNKNOWN-CHECK: %s' "$check"; return 1 ;;
    esac
}

class32_scan() { # 12 bytes on -m32 regparm=3: integer words, never a 16-byte move
    awk -v fn="$2" '
        /^[A-Za-z_.][A-Za-z0-9_.]*:/ { if ($0 !~ /^\.L/) infn = ($0 == fn ":"); next }
        infn && /^[ \t]+[^ \t.#]/ { nlines++ }
        !infn { next }
        /^[ \t]+(movaps|movups|movapd|movupd|movsd|movlp[sd]|movhp[sd])/ {
            print "WIDE-MOVE: " $0; next
        }
        # The vacuity guard, in the only direction that can catch it: a function the
        # scanner cannot locate has no wide move either, so without this line an
        # entirely mislabelled fixture would print "ok".
        END { if (!nlines) print "EMPTY: no instructions found for " fn " -- a body the" \
              " scanner cannot locate is not a body that passed" }
    ' "$1"
}

verdict_for() { # $1 = scanner output, $2 = the verdict that counts as clean
    if [[ -z ${1//[$'\n' ]/} ]]; then printf ok; else printf bad; fi
}

# --- self-test ----------------------------------------------------------------
#
# The scanners above are text recognizers, and a recognizer that has only ever been
# shown correct assembly is indistinguishable from one that cannot see a violation.
# Each is therefore driven with canned transcripts -- no compiler, no 32-bit runtime,
# no build tree -- one that must pass, one that must fail in the specific shape the
# law is about, and one that must fail for *being unable to find the function*.
if [[ ${1:-} == --self-test ]]; then
    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT
    # Legal: three captures into callee-saved registers, then the call.  Ascending
    # order, which is correct precisely because no destination is also a source.
    printf '%s\n' 'good:' '	movl %eax, %esi' '	movl %edx, %edi' '	movl %ecx, %ebp' \
        '	call sink@PLT' >"$tmp/good.s"
    # Violation: the staging for the first parameter runs through %ecx while the
    # third parameter is still waiting to be read out of it.
    printf '%s\n' 'bad:' '	movl $12, %ecx' '	movl %eax, %esi' '	movl %edx, %edi' \
        '	movl %ecx, %ebp' '	call sink@PLT' >"$tmp/bad.s"
    # Not a staging window at all: a call in a function whose parameters came from
    # memory has nothing to say about the rule and must not be reported.
    printf '%s\n' 'quiet:' '	movl 8(%ebp), %eax' '	movl 4(%ebp), %edx' '	call sink@PLT' \
        >"$tmp/quiet.s"
    printf '%s\n' 'c32good:' '	movl $1, %eax' '	movl 8(%esp), %edx' '	movl %ecx, 16(%esp)' \
        '	call use@PLT' >"$tmp/class32.s"
    printf '%s\n' 'c32bad:' '	movaps (%eax), %xmm0' '	call use@PLT' >"$tmp/class32bad.s"
    # A body the label match cannot locate: no wide move, and no instructions either.
    printf '%s\n' 'elsewhere:' '	.cfi_startproc' >"$tmp/nobody.s"

    fails=0
    check() { # $1 = name, $2 = want (ok|bad), $3 = scanner, $4 = file, $5 = function
        local out verdict
        out=$("$3" "$4" "$5")
        verdict=$(verdict_for "$out" ok)
        if [[ $verdict == "$2" ]]; then
            printf 'self-test: %s ok (%s)\n' "$1" "$2"
        else
            printf 'self-test: %s FAILED -- wanted %s, got %s: %s\n' "$1" "$2" "$verdict" "$out"
            fails=$((fails + 1))
        fi
    }
    check "clobber: legal ascending capture"          ok  clobber_scan "$tmp/good.s"         good
    check "clobber: staging through an unread source" bad clobber_scan "$tmp/bad.s"          bad
    check "clobber: a non-staging window stays quiet" ok  clobber_scan "$tmp/quiet.s"        quiet
    check "clobber: a body it cannot find"            bad clobber_scan "$tmp/nobody.s"      elsewhere
    check "class32: three integer words"              ok  class32_scan "$tmp/class32.s"     c32good
    check "class32: a 16-byte move over 12 bytes"     bad class32_scan "$tmp/class32bad.s"  c32bad
    check "class32: a body it cannot find"             bad class32_scan "$tmp/nobody.s"      elsewhere
    if (( fails )); then
        echo "self-test: FAILED ($fails witness(es))"
        exit 1
    fi
    echo "self-test: ok (7 transcripts)"
    exit 0
fi

[[ -x "$LCCC" ]] || { echo "FAIL: no compiler to measure at $LCCC (build it, or set LCCC=...)" >&2; exit 2; }
command -v "$CC" >/dev/null || { echo "FAIL: no $CC for the oracle" >&2; exit 1; }

# The fixtures live in `tests/regression/i386_staging/`, one directory per gate, for
# two reasons: the pair `i386_mixed12_{caller,callee}.c` is deliberately *not*
# self-contained (only one TU defines `use`), and `run_regression.py --compile-only`
# requires every top-level `tests/regression/*.c` to link on its own.  The scoped
# noreturn fixtures sit in their own directory for the same reason.
# ---------------------------------------------------------------- the sweep
failures=0
executed=0
want_executed=0

# (fixture | flags | function | checker | what a clean answer means)
ASM_CASES=(
"tests/regression/i386_staging/i386_regparm_staging_three.c|-m32 -mregparm=3 -O2|f|clobber|no ABI source clobbered before its last read"
"tests/regression/i386_staging/i386_regparm_staging_three.c|-m32 -mregparm=3 -O0|f|clobber|no ABI source clobbered before its last read"
"tests/regression/i386_staging/i386_regparm_staging_three.c|-m32 -mregparm=2 -O2|f|clobber|two of three carried, the same rule"
"tests/regression/i386_staging/i386_regparm_staging_three.c|-m32 -mregparm=1 -O2|f|clobber|one of three carried, the same rule"
"tests/regression/i386_staging/i386_regparm_small_mixed12.c|-m32 -mregparm=3 -O2|main|class32|12 bytes staged as integer words, never a 16-byte move"
"tests/regression/i386_staging/i386_regparm_small_mixed12.c|-m32 -mregparm=3 -O0|main|class32|the unoptimised staging, same law"
)

for row in "${ASM_CASES[@]}"; do
    IFS='|' read -r fx flags fn checker meaning <<<"$row"
    want_executed=$((want_executed + 1))
    if [[ ! -f $fx ]]; then
        echo "FAIL: missing fixture $fx"
        failures=$((failures + 1))
        continue
    fi
    got=$(asm_case "$fx" "$flags" "$fn" "$checker")
    rc=$?
    executed=$((executed + 1))
    if (( rc != 0 )); then
        echo "FAIL: $fx $flags: lccc could not emit asm for its own fixture"
        failures=$((failures + 1))
        continue
    fi
    if [[ $(verdict_for "$got" ok) == bad ]]; then
        printf 'FAIL: %s %s (%s): %s\n' "$fx" "$flags" "$fn" "$meaning"
        printf '%s\n' "$got" | sed 's/^/        /'
        failures=$((failures + 1))
    else
        printf '  ok: %-52s %s\n' "$flags $fn" "$checker"
    fi
done

# ---------------------------------------------------------------- boundary arm
#
# A calling convention is only correct at a boundary.  Every combination of the two
# compilers, one on each side of the same link, has to agree with the two
# same-compiler controls.  This is the arm that could have found the class two rounds
# ago: with both halves from one compiler, a caller that staged bytes the callee does
# not read still runs, because both sides make the same mistake -- every single-
# compiler measurement of this type has passed the whole time.
#
# `main` returns 0 when the callee received the object it was handed and 1 otherwise,
# so a mismatch is an exit code rather than a crash the harness has to interpret.  The
# link is always done by the oracle, so the only variable is which compiler produced
# which half.
CROSS_CASES=(
"tests/regression/i386_staging/i386_mixed12_caller.c|tests/regression/i386_staging/i386_mixed12_callee.c|-m64 -O2"
"tests/regression/i386_staging/i386_mixed12_caller.c|tests/regression/i386_staging/i386_mixed12_callee.c|-m64 -O0"
"tests/regression/i386_staging/i386_mixed12_caller.c|tests/regression/i386_staging/i386_mixed12_callee.c|-m32 -mregparm=3 -O2"
"tests/regression/i386_staging/i386_mixed12_caller.c|tests/regression/i386_staging/i386_mixed12_callee.c|-m32 -mregparm=3 -O0"
"tests/regression/i386_staging/i386_mixed12_caller.c|tests/regression/i386_staging/i386_mixed12_callee.c|-m32 -O2"
)

cross_build_run() { # $1=caller cc $2=callee cc $3=flags $4=caller src $5=callee src
    local c1=$1 c2=$2 flags=$3 tag src cc exe=$TMP/x.$1.$2.$(printf '%s' "$3" | tr -c 'A-Za-z0-9' '_')
    local objs=() rc=0
    for pair in "caller $c1" "callee $c2"; do
        tag=${pair%% *}
        cc=${pair##* }
        # The row names the *role*, not the path: `gcc` is the host's compiler and
        # `lccc` is the artifact under test at $LCCC.  Feeding the literal word to
        # timeout is a harness bug, and it looked like a compiler failure until the
        # first run printed `failed to run command 'lccc'`.
        case $cc in
            gcc) cc=$CC ;;
            lccc) cc=$LCCC ;;
        esac
        src=$4
        [[ $tag == callee ]] && src=$5
        if ! timeout "$TIMEOUT" "$cc" -std=gnu17 $flags -c "$src" -o "$exe.$tag.o" >"$exe.$tag.err" 2>&1; then
            printf 'COMPILE-FAIL(%s)' "$tag"
            sed 's/^/        /' "$exe.$tag.err" >&2
            return 1
        fi
        objs+=("$exe.$tag.o")
    done
    if ! timeout "$TIMEOUT" "$CC" $flags "${objs[@]}" -o "$exe.exe" >"$exe.link.err" 2>&1; then
        printf 'LINK-FAIL'
        sed 's/^/        /' "$exe.link.err" >&2
        return 1
    fi
    timeout "$TIMEOUT" "$exe.exe"
    printf '%s' "$?"
}

i386_cap=$(i386_capability "$CC")
if [[ $i386_cap != run ]]; then
    # One cell of bookkeeping for the whole leg, because the count is what separates
    # "unmeasured" from "skipped because nobody looked".  The assembly arms above do
    # not need a 32-bit runtime at all, so they run either way.
    want_executed=$((want_executed + 1))
    executed=$((executed + 1))
    echo "note: $(( ${#CROSS_CASES[@]} * 4 )) boundary cells unmeasured -- $CC can $i386_cap a" \
         " 32-bit program here (install gcc-multilib and libc6-dev-i386, or set" \
         " LCCC_REQUIRE_I686=1 to fail on that); the ${#ASM_CASES[@]} assembly arms ran anyway"
    if [[ ${LCCC_REQUIRE_I686:-0} == 1 ]]; then
        echo "FAIL: LCCC_REQUIRE_I686=1 but this host cannot run -m32 code" >&2
        exit 1
    fi
else
    for row in "${CROSS_CASES[@]}"; do
        IFS='|' read -r caller_src callee_src flags <<<"$row"
        for split in "lccc lccc" "gcc gcc" "lccc gcc" "gcc lccc"; do
            set -- $split
            want_executed=$((want_executed + 1))
            executed=$((executed + 1))
            got=$(cross_build_run "$1" "$2" "$flags" "$caller_src" "$callee_src")
            case $got in
            COMPILE-FAIL* | LINK-FAIL*)
                echo "FAIL: $flags caller=$1 callee=$2: $got -- the fixture pair must build with" \
                     " either compiler on either side; a row that cannot be linked is not a pass"
                failures=$((failures + 1))
                continue ;;
            esac
            if [[ $got != 0 ]]; then
                echo "FAIL: $flags caller=$1 callee=$2: the callee did not receive the object it was" \
                     " handed (exit $got) -- a boundary only these two compilers can show"
                failures=$((failures + 1))
            elif [[ $1 == "$2" ]]; then
                printf '  ok: %-32s %-11s control, one compiler on both sides\n' "$flags" "$1+$2"
            else
                printf '  ok: %-32s %-11s boundary holds across compilers\n' "$flags" "$1+$2"
            fi
        done
    done
fi

if (( executed != want_executed )); then
    echo "FAIL: $executed of $want_executed arms ran -- a partial sweep is not a passed one"
    exit 1
fi
if (( failures )); then
    echo "i386 call staging: $failures of $executed arms FAILED"
    exit 1
fi
echo "i386 call staging: $executed arms passed (asm laws + cross-compiler boundaries)"
