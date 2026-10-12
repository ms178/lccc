#!/usr/bin/env bash
# Scoped-entity noreturn: the diagnostic face of the fallthrough answer.
#
# `is_noreturn_call` decides three things at once -- whether a function can fall
# out of its body (-Wreturn-type), whether a switch segment falls through, and
# whether a statement expression has a value.  All three read the *entity the call
# resolves to*, and a GNU nested function is a distinct entity that merely shares a
# basename with an outer declaration (C11 6.2.1p4).  Resolving by bare name
# therefore mis-answers every one of them, in both directions; so does an
# attribute the parser drops on a nested *definition*.
#
# Two faces, both asserted here.  Warnings are the fallthrough face, and the
# expectations are absolute as well as differential:
#   * `expect 1` rows fail a compiler that lost the warning machinery;
#   * `expect 0` rows fail one that warns whenever a call ends a body.
# Where GCC accepts the spelling its answer is the law (measured, not quoted);
# where it refuses to parse (the trailing-attribute form on a definition), there
# is no oracle and the row pins our documented superset instead.
#
# The second face is emitted control flow: `ir::lowering::expr_calls` starts a
# dead block after a call to a function in `noreturn_functions`, which is keyed by
# entity name, so a nested declaration that shares a basename with a noreturn
# entity can delete live code -- a miscompile, not a missed warning.  The
# `SHAPES` section at the bottom asserts that on the emitted asm.
#
# The fixtures live in tests/regression/scoped_noreturn/ so each row's source is a
# file a human can compile by hand; they are deliberately *not* run_regression
# cases, because exit codes cannot see either face.
set -uo pipefail

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
LCCC=${CCC:-$ROOT/target/fastbuild/lccc}
CC=${CC:-gcc}
TIMEOUT=${TIMEOUT:-60}
SRC=$ROOT/tests/regression/scoped_noreturn
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

# --- self-test ----------------------------------------------------------------
#
# Every witness here is a fake compiler with a canned answer, so this half of the
# gate runs on a host with no build tree and no 32-bit runtime at all -- which is
# the point: the shape section exists to catch a miscompile that no diagnostic
# reports, and a check nobody can prove bites is a check nobody can trust.
if [[ ${1:-} == --self-test ]]; then  # (witnesses invoke us with --run)
    fake=$WORK/fake
    mkdir -p "$fake"
    printf '%s\n' '#!/usr/bin/env bash' \
        'out=""; mode=""; src=""' \
        'while (( $# )); do case $1 in' \
        '  -o) out=$2; shift 2;;' \
        '  -S) mode=asm; shift;;' \
        '  -c) mode=obj; shift;;' \
        '  *) [[ -z $src && $1 != -* && -f $1 ]] && src=$1; shift;;' \
        'esac; done' \
        'if [[ $mode == asm ]]; then' \
        '  [[ ${ASM_RC:-0} != 0 ]] && { printf "error: canned failure\n" >&2; exit "$ASM_RC"; }' \
        '  cp "$CANNED_ASM" "$out" || exit 1' \
        '  exit 0' \
        'fi' \
        'i=0' \
        'while (( i < ${WARN_COUNT:-0} )); do' \
        '  printf "warning: control reaches end of non-void function [-Wreturn-type]\n"' \
        '  i=$((i + 1))' \
        'done' \
        'exit 0' >"$fake/cc"
    chmod +x "$fake/cc"

    # Two functions, four shapes: f's epilogue dead or kept, h's live or deleted.
    printf '%s\n' '.text' 'f:' '	endbr64' '	call	die' 'h:' '	endbr64' '	movl	$7, %eax' '	ret' \
        >"$fake/ok.s"
    printf '%s\n' '.text' 'f:' '	endbr64' '	call	die' 'h:' '	endbr64' '	jmp	die' >"$fake/leak.s"
    printf '%s\n' '.text' 'f:' '	endbr64' '	call	die' '	movl	$42, %eax' '	ret' 'h:' '	endbr64' '	movl	$7, %eax' '	ret' \
        >"$fake/weak.s"
    printf '%s\n' '.text' 'f:' '.LFB0:' '	.cfi_startproc' 'h:' '.LFB1:' '	.cfi_startproc' >"$fake/nolines.s"

    run_witness() { # $1=name $2=expected FAIL count $3=grep -F pattern $4.. = env
        local name=$1 want=$2 pat=$3; shift 3
        local out rc fails
        out=$(env "$@" CCC="$fake/cc" CC="$fake/cc" \
            CCC_CASE="same_signature_calls_in_both_scopes.c|0|yes" \
            "$SHELL" "$ROOT/tests/regression/check_scoped_noreturn_diagnostics.sh" --run 2>&1)
        rc=$?
        fails=$(printf '%s\n' "$out" | grep -c '^FAIL: ')
        if (( fails != want )); then
            printf 'self-test FAILED: %s reported %s failures, expected %s\n%s\n' \
                "$name" "$fails" "$want" "$out"
            return 1
        fi
        if (( want > 0 )) && ! grep -qF -- "$pat" <<<"$out"; then
            printf 'self-test FAILED: %s did not name the failure it found (%s)\n%s\n' \
                "$name" "$pat" "$out"
            return 1
        fi
# A witness that passes by *not running* is the failure mode this whole
        # file exists to avoid, so the sweep size is asserted as well: the clean
        # one has to report its full count, and a red one has to say how many of
        # the same seven configurations it failed.
        local sig='checks passed (3 of them on emitted control flow)'
        (( want > 0 )) && sig='of 7 checks FAILED'
        if ! grep -qF -- "$sig" <<<"$out"; then
            printf 'self-test FAILED: %s did not sweep all 7 configurations (%s)\n%s\n' "$name" "$sig" "$out"
            return 1
        fi
        if [[ $name == clean ]] && (( rc != 0 )); then
            printf 'self-test FAILED: the clean witness did not exit 0 (rc %s)\n%s\n' "$rc" "$out"
            return 1
        fi
        if [[ $name != clean ]] && (( rc == 0 )); then
            printf 'self-test FAILED: %s exited 0 while reporting %s failures\n' "$name" "$fails"
            return 1
        fi
        printf 'self-test: %s ok\n' "$name"
    }

    bad=0
    run_witness clean   0 "self-test: no pattern" \
        CANNED_ASM=$fake/ok.s WARN_COUNT=0 ASM_RC=0 || bad=1
    # The noreturn fact leaked into the nested scope: h's live `return 7` is gone.
    # No warning row can see this -- both compilers answer 0 either way.
    run_witness leak    2 "epilogue must be live after the call" \
        CANNED_ASM=$fake/leak.s WARN_COUNT=0 ASM_RC=0 || bad=1
    # Our own ratchet: a dead epilogue must be deleted, lccc only, so one failure.
    run_witness weak    1 "dead code that must be gone" \
        CANNED_ASM=$fake/weak.s WARN_COUNT=0 ASM_RC=0 || bad=1
    # If the label match stopped finding instructions, every shape would "pass"
    # by having nothing to say -- so an empty body is a failure, not a skip.
    run_witness empty   3 "no instructions emitted" \
        CANNED_ASM=$fake/nolines.s WARN_COUNT=0 ASM_RC=0 || bad=1
    run_witness asmfail 3 "could not emit asm" \
        CANNED_ASM=$fake/ok.s WARN_COUNT=0 ASM_RC=1 || bad=1
    # The warning channel still bites: two answers where the row says none.
    run_witness warnrow 4 "warnings, expected 0" \
        CANNED_ASM=$fake/ok.s WARN_COUNT=2 ASM_RC=0 || bad=1

    if (( bad )); then
        echo "self-test: FAILED"
        exit 1
    fi
    echo "self-test: ok (6 witnesses, fakes only -- no build tree needed)"
    exit 0
fi

[[ -x $LCCC ]] || { echo "FAIL: no lccc at $LCCC (set CCC=)"; exit 1; }
command -v "$CC" >/dev/null || { echo "FAIL: no $CC for the oracle"; exit 1; }
[[ -d $SRC ]] || { echo "FAIL: no fixtures in $SRC"; exit 1; }

# fixture | expected warnings | gcc accepts it (oracle available)
CASES=(
"outer_noreturn_shadowed_by_returning_nested.c|1|yes"
"no_attribute_anywhere_control.c|1|yes"
"noreturn_decl_specifier_on_nested_definition.c|0|yes"
"block_scope_extern_keeps_file_entity.c|0|yes"
"noreturn_does_not_leak_to_next_nested_definition.c|1|yes"
"trailing_attribute_on_nested_definition.c|0|no"
"same_signature_calls_in_both_scopes.c|0|yes"
)

# The self-test sweeps one row.  A witness that also ran the other six
# fixtures would report the same failures for the wrong reasons, and a
# witness that the gate ignores is a witness that proves nothing.
if [[ -n ${CCC_CASE:-} ]]; then
    CASES=("$CCC_CASE")
fi


# One warning answer, for a compiler, at one tier.  The count is what the row
# asserts; the exit status is checked separately and only for lccc, because a
# row's `oracle` field is precisely the statement that GCC may not compile it.
warn_answer() { # $1=compiler $2=file $3=tier -> prints count
    local out rc
    out=$(timeout "$TIMEOUT" "$1" -std=gnu17 -Wall "$3" -c "$2" -o /dev/null 2>&1)
    rc=$?
    if (( rc != 0 )) && [[ $1 == "$LCCC" ]]; then
        printf 'COMPILE-FAIL %s\n' "$rc"
        return
    fi
    printf '%s' "$out" | grep -c "control reaches end of non-void function"
}

tiers=(-O1 -O2)
executed=0
want_executed=0
failures=0
differential=0

for row in "${CASES[@]}"; do
    IFS='|' read -r name expect oracle <<<"$row"
    file="$SRC/$name"
    [[ -f $file ]] || { echo "FAIL: missing fixture $file"; failures=$((failures + 1)); continue; }
    for tier in "${tiers[@]}"; do
        want_executed=$((want_executed + 1))
        got=$(warn_answer "$LCCC" "$file" "$tier")
        executed=$((executed + 1))
        if [[ $got == COMPILE-FAIL* ]]; then
            echo "FAIL: $name $tier: lccc could not compile its own fixture ($got)"
            failures=$((failures + 1))
        elif [[ $got != "$expect" ]]; then
            echo "FAIL: $name $tier (lccc): $got warnings, expected $expect"
            failures=$((failures + 1))
        fi
        if [[ $oracle == yes ]]; then
            want_executed=$((want_executed + 1))
            got=$(warn_answer "$CC" "$file" "$tier")
            executed=$((executed + 1))
            differential=$((differential + 1))
            if [[ $got != "$expect" ]]; then
                echo "FAIL: $name $tier (gcc): $got warnings, expected $expect -- the law this row cites no longer holds"
                failures=$((failures + 1))
            fi
        else
            # A row that claims there is no oracle must really have one missing:
            # if GCC starts accepting the spelling, the row has to be re-measured
            # rather than left asserting our answer alone.
            if timeout "$TIMEOUT" "$CC" -std=gnu17 -Wall -O1 -c "$file" -o /dev/null >/dev/null 2>&1; then
                echo "FAIL: $name: oracle=no but gcc now compiles it -- re-measure this row against gcc"
                failures=$((failures + 1))
            fi
            executed=$((executed + 1))
            want_executed=$((want_executed + 1))
        fi
    done
done

# --- emitted control flow -----------------------------------------------------
#
# The warning count cannot see the discriminating program: in
# `same_signature_calls_in_both_scopes.c` neither call produces a warning
# whichever entity it resolves to, because both functions return a value.  What
# *does* change is the emitted code -- the dead block that
# `ir::lowering::expr_calls` puts after a noreturn call either appears or does
# not -- so the fixtures are asserted on the asm that comes out at -O2.
#
# `live` is a correctness claim and is checked against GCC too: a call that
# returns must leave the rest of its function reachable.  A compiler that hands
# that call the *other* scope's noreturn deletes a live epilogue and miscompiles
# the program, and no diagnostic says so.
#
# `dead` is an optimisation claim, so it is asserted about lccc alone: after a
# noreturn call the epilogue must be gone.  A compiler free to keep unreachable
# code is weaker rather than wrong, and pinning GCC's choice here would measure
# GCC instead of us.
SHAPES=(
"same_signature_calls_in_both_scopes.c|h|live|yes"
"same_signature_calls_in_both_scopes.c|f|dead|no"
)

asm_body() { # $1=compiler $2=file $3=function -> indented instruction lines
    local out=$WORK/$(basename "$2").$(basename "$1" | tr -c 'A-Za-z0-9' '_').s
    if ! timeout "$TIMEOUT" "$1" -std=gnu17 -O2 -S "$2" -o "$out" 2>/dev/null; then
        printf 'COMPILE-FAIL'
        return
    fi
    # One function's instructions: after its label, indented, and not an assembler
    # directive.  Both filters matter.  `.cfi_*` lines would otherwise become the
    # "last instruction" of every function, so no shape could ever read as `ret`.
    # And a label only *ends* a body when it names another function: `.LBB3:` and
    # friends are basic-block labels, so treating them as function ends would
    # truncate every body at its second block and quietly empty out the check.
    # lccc's own `# LCCC_RET_*` / `# LCCC_CALL_ARGS` annotations are indented
    # comments rather than instructions; they would otherwise become the "last
    # instruction" of a body and decide a shape that the machine code says
    # nothing about.
    awk -v fn="$3" '
        /^[A-Za-z_.][A-Za-z0-9_.]*:/ { if ($0 !~ /^\.L/) infn = ($0 == fn ":") }
        infn && /^[ \t]+[^ \t.#]/ { print }
    ' "$out"
}

shape_seen=0
for row in "${SHAPES[@]}"; do
    IFS='|' read -r name fn kind oracle <<<"$row"
    file="$SRC/$name"
    people=(lccc)
    if [[ $oracle == yes ]]; then
        people=(lccc gcc)
    fi
    for who in "${people[@]}"; do
        compiler=$LCCC
        [[ $who == gcc ]] && compiler=$CC
        want_executed=$((want_executed + 1))
        body=$(asm_body "$compiler" "$file" "$fn")
        executed=$((executed + 1))
        shape_seen=$((shape_seen + 1))
        if [[ $body == COMPILE-FAIL* ]]; then
            echo "FAIL: $name ($fn, $who): $compiler could not emit asm for its own fixture"
            failures=$((failures + 1))
            continue
        fi
        if [[ -z ${body//[[:space:]]/} ]]; then
            echo "FAIL: $name ($fn, $who): no instructions emitted for \`$fn\` -- the label match is not seeing this function"
            failures=$((failures + 1))
            continue
        fi
        if [[ $kind == live ]]; then
            last=$(printf '%s\n' "$body" | tail -1 | sed 's/^[[:space:]]*//')
            if [[ $last != *ret* ]]; then
                echo "FAIL: $name ($fn, $who): \`$fn\` returns, so its epilogue must be live after the call; last instruction is \`$last\`"
                failures=$((failures + 1))
            fi
        elif [[ $who == lccc ]]; then
            if grep -qE '(^|[^_[:alnum:]])ret([^_[:alnum:]]|$)' <<<"$body"; then
                echo "FAIL: $name ($fn, $who): the call never returns, so \`$fn\`'s epilogue is dead code that must be gone"
                failures=$((failures + 1))
            fi
        fi
    done
done

if (( executed != want_executed )); then
    echo "FAIL: $executed of $want_executed configurations ran -- a partial sweep is not a passed one"
    exit 1
fi
if (( failures )); then
    echo "scoped-noreturn diagnostics: $failures of $executed checks FAILED"
    exit 1
fi
echo "scoped-noreturn diagnostics: $executed checks passed ($shape_seen of them on emitted control flow), $differential of them differential against gcc"
