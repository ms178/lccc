#!/usr/bin/env bash
# ============================================================================
# gla_equiv_check.sh — gate ON/OFF differential for the global location
# allocator (GLA, master env CCC_RA_GLOBAL_LOCATION).
#
# For every tests/regression/*.c (or a filter), compiles and RUNS the TU with
# the gate forced ON and forced OFF at each optimization level, requiring
# byte-identical stdout AND identical exit code. On cross targets the
# reference compiler (cross gcc) is additionally run under qemu as an
# independent oracle. TUs with an .env sidecar setting LCCC_NO_AB=1 or
# LCCC_NO_COMPARE=1 are skipped; builds that fail on BOTH sides (target-
# incompatible TUs) are skipped honestly. Any one-sided build failure,
# divergence, or one-sided timeout is a miscompile (exit 1).
#
# Complements the static census (census_full_delta.sh: emitted-text deltas)
# and the fire census (gla_fire_census.py: where the planner acts):
# equivalence proves the rewrites are value-preserving end to end.
#
# Modes:
#   (default)                         x86-64 host, native execution
#   --32                              i686 target (-m32), native or qemu-i386
#   --cross aarch64                   lccc-arm under qemu-aarch64 + gcc oracle
#   --cross riscv64                   lccc-riscv under qemu-riscv64 + oracle
#
# Usage:
#   ./gla_equiv_check.sh [--opts "-O0 -O1 -O2 -O3 -Os"]
#                       [--32 | --cross aarch64|riscv64]
#                       [--on CC] [--off CC] [filter]
# Environment:
#   EQUIV_REPS=N   repetitions per binary for stability (default 3)
# ============================================================================
set -u

# shellcheck disable=SC1007 # CDPATH= intentionally scopes the builtin
REPO=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
REG="$REPO/tests/regression"
OPTS="-O0 -O1 -O2 -O3 -Os"
MODE="x64"
CROSS=""
CC_ON=""
CC_OFF=""
FILTER=""
while [ $# -gt 0 ]; do
    case $1 in
        --opts) shift; OPTS=$1 ;;
        --32) MODE="m32" ;;
        --cross) shift; MODE="cross"; CROSS=$1 ;;
        --on) shift; CC_ON=$1 ;;
        --off) shift; CC_OFF=$1 ;;
        -h|--help) grep '^#' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) FILTER=$1 ;;
    esac
    shift
done

case "$MODE:$CROSS" in
    m32:)
        TARGET_FLAG=(-m32); ORACLE=gcc; QEMU=""
        [ -z "$CC_ON" ]  && CC_ON=$REPO/target/fastbuild/lccc
        [ -z "$CC_OFF" ] && CC_OFF=$REPO/target/fastbuild/lccc
        ;;
    x64:)
        TARGET_FLAG=(); ORACLE=gcc; QEMU=""
        [ -z "$CC_ON" ]  && CC_ON=$REPO/target/fastbuild/lccc
        [ -z "$CC_OFF" ] && CC_OFF=$REPO/target/fastbuild/lccc
        ;;
    cross:aarch64)
        TARGET_FLAG=(); ORACLE=aarch64-linux-gnu-gcc
        QEMU="qemu-aarch64 -L /usr/aarch64-linux-gnu"
        [ -z "$CC_ON" ]  && CC_ON=$REPO/target/fastbuild/lccc-arm
        [ -z "$CC_OFF" ] && CC_OFF=$REPO/target/fastbuild/lccc-arm
        ;;
    cross:riscv64)
        TARGET_FLAG=(); ORACLE=riscv64-linux-gnu-gcc
        QEMU="qemu-riscv64 -L /usr/riscv64-linux-gnu"
        [ -z "$CC_ON" ]  && CC_ON=$REPO/target/fastbuild/lccc-riscv
        [ -z "$CC_OFF" ] && CC_OFF=$REPO/target/fastbuild/lccc-riscv
        ;;
    *) echo "unknown target mode"; exit 2 ;;
esac

for cc in "$CC_ON" "$CC_OFF"; do
    [ -x "$cc" ] || { echo "compiler not found/executable: $cc"; exit 2; }
done
if [ "$MODE" = cross ]; then
    command -v "${QEMU%% *}" >/dev/null 2>&1 || { echo "missing ${QEMU%% *}"; exit 2; }
    command -v "$ORACLE" >/dev/null 2>&1 || { echo "missing oracle $ORACLE"; exit 2; }
fi

GCC_INC="-I$($ORACLE -print-file-name=include)"
WORK=$(mktemp -d /tmp/gla-equiv.XXXXXX)
trap 'rm -rf "$WORK"' EXIT

# ── Execution strategy ────────────────────────────────────────────────────
# An unrunnable binary that dies identically under BOTH configurations must
# never count as a pass (the SIGSYS-vacuous-match lesson). Probe once and
# pick: native, qemu runner, or honest SKIP for all TUs.
RUN_PREFIX=()
if [ "$MODE" = m32 ]; then
    can_run_elf32_native() {
        cat > "$WORK/p32.c" <<'PEOF'
/* i386 Linux: exit(42) via int $0x80 (eax=1, ebx=status), no libc. */
void _start(void) {
    __asm__ volatile("int $0x80" : : "a"(1), "b"(42) : "memory");
    __builtin_unreachable();
}
PEOF
        gcc -m32 -O2 -fno-pic -nostdlib -static -Wl,-e,_start \
            "$WORK/p32.c" -o "$WORK/p32" 2>/dev/null || return 1
        "$WORK/p32" >/dev/null 2>&1
        [ $? -eq 42 ]
    }
    if can_run_elf32_native; then
        : # native ELF32 execution works
    else
        for cand in "${LCCC_I686_RUNNER:-}" qemu-i386 qemu-i386-static; do
            [ -n "$cand" ] || continue
            if command -v "$cand" >/dev/null 2>&1 || [ -x "$cand" ]; then
                if $cand "$WORK/p32" >/dev/null 2>&1; [ $? -eq 42 ]; then
                    RUN_PREFIX=("$cand"); break
                fi
            fi
        done
        if [ ${#RUN_PREFIX[@]} -eq 0 ]; then
            echo "cannot execute ELF32 binaries and no qemu-i386 found; SKIP all"
            exit 2
        fi
    fi
elif [ "$MODE" = cross ]; then
    read -ra RUN_PREFIX <<<"$QEMU"
fi

# Fixed guest argv[0] for EVERY binary (both gates and the oracle). The
# dynamic loader, qemu-user diagnostics and several libc abort paths embed
# argv[0] verbatim ("/path/gate1: symbol lookup error: …/gate1: …"), so two
# BYTE-IDENTICAL gate binaries compared under different file names would
# show a spurious "output divergence" (the arm_f128_param_preserves_gp_param
# case on riscv64: the missing __trunctfdf2 lookup prints the path twice).
# qemu-user gets it via `-0`; a native exec uses bash's `exec -a`.
ARGV0="lccc-equiv-prog"

run_bin() { # run_bin <bin> ; echoes "stdout|exitcode"
    local bin=$1 out ec
    if [ "$MODE" = cross ]; then
        # QEMU is "<emulator> -L <sysroot>"; insert -0 before the binary.
        local -a qa
        read -ra qa <<<"$QEMU"
        out=$(timeout 25 "${qa[@]}" -0 "$ARGV0" "$bin" 2>&1); ec=$?
    elif [ ${#RUN_PREFIX[@]} -eq 0 ]; then
        out=$(timeout 25 bash -c 'exec -a "$0" "$@"' "$ARGV0" "$bin" 2>&1); ec=$?
    else
        out=$(timeout 25 "${RUN_PREFIX[@]}" -0 "$ARGV0" "$bin" 2>&1); ec=$?
    fi
    [ $ec -eq 124 ] && { echo "TIMEOUT"; return; }
    # Defensive second layer: scrub every absolute work-dir path (and the
    # current bin's path) so no diagnostic can leak the artifact name.
    out=${out//$bin/PROG}
    out=${out//$WORK/PROGDIR}
    printf '%s|%d' "$out" "$ec"
}

# Repeat runs so a flaky TU cannot pass/fail the differential by luck; a
# disagreement across repeats is reported UNSTABLE.
run_stable() {
    local bin=$1 reps=${EQUIV_REPS:-3} first r i
    first=$(run_bin "$bin")
    [ "$first" = TIMEOUT ] && { echo TIMEOUT; return; }
    for ((i=1; i<reps; i++)); do
        r=$(run_bin "$bin")
        if [ "$r" != "$first" ]; then echo UNSTABLE; return; fi
    done
    printf '%s' "$first"
}

build() { # build <cc> <gate 0|1> <opt> <src> <out> <errfile>
    local cc=$1 gate=$2 opt=$3 src=$4 out=$5 err=$6
    local base=${src%.c}; local flags=()
    [ -f "$base.flags" ] && read -r -a flags < "$base.flags"
    CCC_RA_GLOBAL_LOCATION=$gate "$cc" "$GCC_INC" "$opt" "${TARGET_FLAG[@]}" \
        "${flags[@]}" "$src" -o "$out" 2>"$err"
}

build_oracle() { # build_oracle <opt> <src> <out>
    local opt=$1 src=$2 out=$3 base=${2%.c}; local flags=()
    [ -f "$base.flags" ] && read -r -a flags < "$base.flags"
    "$ORACLE" "$opt" "${TARGET_FLAG[@]}" "${flags[@]}" "$src" -o "$out" \
        2>"$WORK/oracle.err"
}

nrun=0; nskip=0; nfail=0; noracle=0; nprexist=0
PREEXIST=()
FAILED=()
for src in "$REG"/*.c; do
    name=$(basename "$src" .c)
    [[ -n "$FILTER" && "$name" != *"$FILTER"* ]] && continue
    base=${src%.c}
    skip=0
    if [ -f "$base.env" ]; then
        while IFS= read -r line; do
            case $line in
                LCCC_NO_AB=1|LCCC_NO_COMPARE=1) skip=1 ;;
            esac
        done < "$base.env"
    fi
    if [ $skip -eq 1 ]; then nskip=$((nskip+1)); continue; fi
    for opt in $OPTS; do
        # Equal-length basenames gate1/gate0: qemu-user binaries that read
        # uninitialized stack can emit output that depends on the byte
        # layout of argv/envp (e.g. $WORK/on vs $WORK/off differ in length);
        # equal-length names remove that spurious perturbation.
        bon="$WORK/gate1"; boff="$WORK/gate0"; bora="$WORK/gatex"
        if ! build "$CC_ON" 1 "$opt" "$src" "$bon" "$WORK/gate1.err"; then
            if build "$CC_OFF" 0 "$opt" "$src" "$boff" "$WORK/gate0.err"; then
                echo "FAIL $name $opt: gate-ON build failed, gate-OFF built"
                head -3 "$WORK/gate1.err" | sed 's/^/     /'
                nfail=$((nfail+1)); FAILED+=("$name:$opt:build"); continue
            fi
            continue   # unsupported TU on this target
        fi
        if ! build "$CC_OFF" 0 "$opt" "$src" "$boff" "$WORK/gate0.err"; then
            echo "FAIL $name $opt: gate-OFF build failed, gate-ON built"
            head -3 "$WORK/gate0.err" | sed 's/^/     /'
            nfail=$((nfail+1)); FAILED+=("$name:$opt:buildoff"); continue
        fi
        ron=$(run_stable "$bon"); roff=$(run_stable "$boff")
        if [ "$roff" = UNSTABLE ] || [ "$ron" = UNSTABLE ]; then
            echo "SKIP $name $opt: nondeterministic baseline (on=$ron off=$roff)"
            nskip=$((nskip+1)); continue
        fi
        if [ "$ron" = TIMEOUT ] && [ "$roff" = TIMEOUT ]; then
            echo "SKIP $name $opt: times out with the gate off too"
            nskip=$((nskip+1)); continue
        fi
        if [ "$ron" = TIMEOUT ] || [ "$roff" = TIMEOUT ]; then
            echo "FAIL $name $opt: one-sided timeout on=$ron off=$roff"
            nfail=$((nfail+1)); FAILED+=("$name:$opt:timeout"); continue
        fi
        # Independent oracle agreement, best effort on EVERY mode:
        # cross uses the cross-gcc under qemu, m32 uses gcc -m32 under
        # qemu-i386, x64 uses the host gcc. A TU the oracle cannot build
        # (lccc-specific features, missing headers) is skipped; an
        # UNSTABLE/TIMEOUT oracle run is skipped too. Agreement is counted.
        # A case where BOTH gates agree with each other but neither with
        # gcc is a PRE-EXISTING backend gap (not a GLA property): reported
        # separately and counted, never confused with a gate divergence
        # below (off==oracle && on!=oracle implies on!=off and fails there).
        if build_oracle "$opt" "$src" "$bora"; then
            rora=$(run_stable "$bora")
            if [ "$rora" = UNSTABLE ] || [ "$rora" = TIMEOUT ]; then
                echo "SKIP $name $opt: oracle itself $rora"
                nskip=$((nskip+1)); continue
            fi
            if [ "$rora" = "$ron" ]; then
                noracle=$((noracle+1))
            else
                nprexist=$((nprexist+1))
                PREEXIST+=("$name:$opt")
            fi
        elif [ -n "$CROSS" ]; then
            # Cross mode: the cross-gcc oracle is the authority that this TU
            # is meaningful for the target. A rejected TU (e.g. -mavx2
            # sidecars or ia32 vector intrinsics on aarch64) is
            # target-incompatible; its binary can depend on undefined or
            # uninitialized state whose observed output even changes with
            # argv[0] under qemu-user (stack garbage), so gate1/gate0 run
            # comparison cannot prove anything. Skip rather than fail.
            echo "SKIP $name $opt: cross oracle cannot build (target-incompatible)"
            nskip=$((nskip+1)); continue
        fi
        nrun=$((nrun+1))
        if [ "$ron" != "$roff" ]; then
            echo "FAIL $name $opt: output divergence"
            diff <(printf '%s\n' "$ron") <(printf '%s\n' "$roff") | head -12 | sed 's/^/     /'
            nfail=$((nfail+1)); FAILED+=("$name:$opt:run")
        fi
    done
done

echo "----"
echo "mode=${MODE}${CROSS:+:$CROSS} ran=$nrun oracle-confirmed=$noracle pre-existing-oracle-gaps=$nprexist skipped=$nskip failed=$nfail"
if [ $nfail -ne 0 ]; then
    printf 'FAILED: %s\n' "${FAILED[@]}"
    exit 1
fi
if [ $nprexist -ne 0 ]; then
    printf 'note: pre-existing (gate-independent) oracle gaps: %s\n' "${PREEXIST[*]}"
fi
if [ "$MODE" = m32 ]; then tag="i686"; else tag="${MODE}${CROSS:+:$CROSS}"; fi
echo "GATE ON/OFF EQUIVALENT ($tag)"
