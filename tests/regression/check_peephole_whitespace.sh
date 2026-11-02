#!/usr/bin/env bash
# Trailing whitespace is semantically neutral in every assembly syntax GAS
# accepts, so a compiler whose output depends on it is wrong.  The x86 peephole
# was wrong in three separate ways before this gate existed, and a review of the
# patch that fixed the first of them estimated "~46 unfixed sites" of the same
# class.  The answer here is not a site list but a property, checked four ways:
#
#   * a load with a single trailing space stopped forwarding the constant store
#     above it -- in lccc's own sha256 output, `movq $0, 152(%rsp)` followed by
#     `movq 152(%rsp), %rax ` re-read memory on every LCG iteration;
#   * the `%rcx` address-copy fold was lost when the CONSUMER line was padded,
#     because passes slice operands out of the raw line text;
#   * `movq $3, %xmm0` and `movq $1, %st` panicked the compiler outright: the
#     immediate-load/ALU pair indexed the 16-entry GP register-name table before
#     validating the family id, so an XMM family (24) or REG_NONE (255) aborted
#     the process.  That one needs no whitespace at all -- the padded corpus
#     merely made it reachable often enough to notice.
#
# All three are now impossible by construction (LineStore trims trailing blanks
# from every span it hands out and from every replacement a pass writes, and the
# family is validated before it is used as an index), and this gate is what keeps
# them impossible: it re-derives the property from real assembly instead of
# trusting the code that asserts it.
#
# Phase 1  in-tree corpus        fixtures + the repository's committed .s, 7 paddings
# Phase 2  operand totality      the exhaustive matrix (~150k pipeline runs, #[ignore]d
#                                in the default suite because it costs minutes)
# Phase 3  generated corpus      real C -> real .s with the just-built lccc, then padded
# Phase 4  assembler path        padded .s must assemble to the same object bytes
#
# Usage: tests/regression/check_peephole_whitespace.sh [--phase N]
#   CCC=... path to the lccc binary (default target/fastbuild/lccc)
#   PROFILE / JOBS  cargo profile and -j (default fastbuild / 2)
#   CORPUS_EVERY=15 sample the first-party C selection after corpus exclusion
#   --list [root]  print that exact sample without running compilers
set -uo pipefail

repo_root=$(CDPATH= cd -P -- "$(dirname -- "$(readlink -f -- "$0")")/../.." && pwd -P)
cd "$repo_root"

# --- argument parsing -------------------------------------------------------
# `ONLY_PHASE` is the phase NUMBER, never the literal `--phase`: it used to be
# `${1:-}`, so the documented `--phase 1` stored the flag itself, `want_phase`
# matched nothing, every phase was skipped -- and the gate still printed PASS
# for all four of them.  A selector that silently selects nothing is worse than
# no selector, so malformed input is REJECTED rather than reinterpreted:
#
#   * `--phase=` is an error, not "run every phase" (an empty selector that
#     widens scope is exactly the failure this gate exists to prevent);
#   * a repeated selector is an error, not silently last-wins;
#   * anything after the selector is an error, not silently ignored.
#
# Factored into a function so --selftest drives it with cases instead of trust,
# and needs no compiler, no cargo and no corpus to do it.
PHASE_USAGE="usage: $0 [--phase {1|2|3|4}]"

parse_phase_args() { # parse_phase_args "$@" -> sets ONLY_PHASE; rc 0 ok, 2 usage
    ONLY_PHASE=''
    local seen=0 pending=0 arg
    for arg in "$@"; do
        case "$arg" in
            --phase | --phase=*)
                if [ "$seen" -ne 0 ]; then
                    echo "$PHASE_USAGE (repeated --phase)" >&2
                    return 2
                fi
                seen=1
                if [ "$arg" = '--phase' ]; then
                    pending=1
                else
                    ONLY_PHASE=${arg#--phase=}
                    if [ -z "$ONLY_PHASE" ]; then
                        echo "$PHASE_USAGE (empty --phase=)" >&2
                        return 2
                    fi
                fi
                ;;
            *)
                if [ "$pending" -eq 1 ]; then
                    pending=0
                    ONLY_PHASE=$arg
                else
                    echo "$PHASE_USAGE (unexpected argument '$arg')" >&2
                    return 2
                fi
                ;;
        esac
    done
    if [ "$pending" -eq 1 ]; then
        echo "$PHASE_USAGE (--phase needs a value)" >&2
        return 2
    fi
    case "$ONLY_PHASE" in
        '' | 1 | 2 | 3 | 4) return 0 ;;
        *) echo "$PHASE_USAGE (no phase '$ONLY_PHASE')" >&2; return 2 ;;
    esac
}

# Compiler-free enumeration is the EXACT phase-3 selection, not a test-only
# replica. It runs before toolchain discovery and before any cargo command.
if [[ ${1:-} == --list ]]; then
    exec python3 scripts/corpus_selection.py "${2:-$repo_root/tests}" \
      --every "${CORPUS_EVERY:-15}"
fi

# Compiler-free proof that the argument parser does what the usage line claims.
# This is the part of the gate that regressed once already -- it accepted
# `--phase 1`, stored the flag, selected nothing and still printed PASS -- so it
# gets a case table rather than a comment.  Runs before toolchain discovery: no
# cargo, no compiler, no corpus.
if [[ ${1:-} == --selftest ]]; then
    st_fail=0
    st_case() { # st_case <want_rc> <want_phase> [args...]
        local want_rc=$1 want_phase=$2
        shift 2
        parse_phase_args "$@" >/dev/null 2>&1
        local rc=$?
        if [ "$rc" -ne "$want_rc" ] ||
            { [ "$rc" -eq 0 ] && [ "$ONLY_PHASE" != "$want_phase" ]; }; then
            printf 'FAIL  args=[%s] rc=%s want=%s phase=[%s] want=[%s]\n' \
                "$*" "$rc" "$want_rc" "$ONLY_PHASE" "$want_phase"
            st_fail=1
        else
            printf 'ok    args=[%s] -> rc=%s phase=[%s]\n' "$*" "$rc" "$ONLY_PHASE"
        fi
    }
    echo "== parser self-test =="
    st_case 0 ''                        # no selector: run every phase
    st_case 0 '1' --phase 1
    st_case 0 '2' --phase 2
    st_case 0 '3' --phase 3
    st_case 0 '4' --phase 4
    st_case 0 '1' --phase=1
    st_case 0 '4' --phase=4
    st_case 2 '' --phase=               # empty selector must NOT widen to all
    st_case 2 '' --phase                # missing value
    st_case 2 '' --phase 5              # out of range
    st_case 2 '' --phase 0
    st_case 2 '' --phase x
    st_case 2 '' --bogus
    st_case 2 '' stray                  # bare positional
    st_case 2 '' --phase 1 unexpected   # trailing argument
    st_case 2 '' --phase 1 --phase 2    # repeated selector
    st_case 2 '' --phase=1 --phase=2
    st_case 2 '' --phase --phase
    st_case 2 '' --phase=1 stray
    if [ "$st_fail" -ne 0 ]; then
        echo "check_peephole_whitespace: parser self-test FAILED" >&2
        exit 1
    fi
    echo "check_peephole_whitespace: parser self-test PASS (19 cases)"
    exit 0
fi

# Prefer the persisted rustup installation: the phases drive `cargo test`
# directly, and a bare environment (cron, CI shards, fresh shells) does not
# have ~/.cargo/bin on PATH — the gate then reads "cargo: command not found"
# as three test failures.
if [[ -x "${CARGO_HOME:-$HOME/.cargo}/bin/cargo" ]]; then
    export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"
fi

CCC=${CCC:-target/fastbuild/lccc}
PROFILE=${PROFILE:-fastbuild}
JOBS=${JOBS:-2}
CORPUS_EVERY=${CORPUS_EVERY:-15}
# `ONLY_PHASE` is the phase NUMBER, never the literal `--phase`: it used to be
# `${1:-}`, so the documented `--phase 1` stored the flag itself, `want_phase`
# matched nothing, every phase was skipped -- and the gate still printed PASS
# for all four of them.  A selector that silently selects nothing is worse than
# no selector, so the argument is parsed and validated here instead.
parse_phase_args "$@" || exit $?

fail=0
declare -a RESULTS=()

note() { printf '%s\n' "$*"; }
ok()   { printf 'ok   %s\n' "$*"; RESULTS+=("ok   $*"); }
bad()  { printf 'FAIL %s\n' "$*" >&2; RESULTS+=("FAIL $*"); fail=1; }
# A phase runs unless --phase N selected a different one.  `$1` is the phase
# number; the second argument is always `$ONLY_PHASE` at the call site, which is
# empty when every phase was requested.
want_phase() { [[ -z "${2:-}" || "${2:-}" == "$1" ]] && return 0; return 1; }

cargo_test() { # cargo_test <filter> [extra cargo-test args...]
    local filter=$1; shift
    # Low-memory discipline (mirrors ci_local.sh's cargo_test_repeated): the
    # fastbuild profile's incremental session state plus line-tables-only
    # debuginfo OOMs a 4 GB host during the lib-test compile, and the gate
    # then reads a SIGKILLed compiler as three test failures. Drop
    # incremental and debuginfo, give the linker --no-keep-memory, and pin
    # one compile job on small hosts.
    local jobs="$JOBS"
    if [[ -z "${JOBS:-}" ]]; then
        local total_mb
        total_mb=$(free -m 2>/dev/null | awk '/^Mem:/{print $2}')
        [[ -n "$total_mb" && "$total_mb" -lt 6000 ]] && jobs=1
    fi
    # --no-keep-memory is a GNU-ld flag; non-GNU linkers (e.g. macOS ld)
    # reject it and the gate would read a linker error as a test failure.
    # The repository targets Linux (BUILDING_LINUX.txt), so this guard is
    # for portability hygiene, not a supported configuration.
    local ld_flags=""
    if [[ "$(uname -s)" == "Linux" ]]; then
        ld_flags="-C link-arg=-Wl,--no-keep-memory"
    fi
    CARGO_PROFILE_FASTBUILD_DEBUG=0 CARGO_INCREMENTAL=0 \
    RUSTFLAGS="${RUSTFLAGS:-} ${ld_flags}" \
        timeout 1800 cargo test --profile "$PROFILE" --lib --locked -j "$jobs" \
        "$filter" -- "$@" 2>&1
}

# --------------------------------------------------------------- phase 1 ----
# The in-tree corpus: hand-written fixtures for the operand shapes the passes
# slice, plus every .s the repository already commits.  Runs in the default
# suite too; this phase is here so the gate reports it and so --phase 1 is a
# two-second smoke check.
if want_phase 1 "$ONLY_PHASE"; then
    note "== phase 1: in-tree corpus, seven paddings"
    out=$(cargo_test peephole_output_is_invariant_to_trailing_whitespace 2>&1)
    if grep -q "test result: ok" <<<"$out"; then
        runs=$(grep -oE "[0-9]+ passed" <<<"$out" | sed -n '1,1p')
        ok "peephole output is invariant to trailing whitespace ($runs)"
    else
        bad "peephole output is NOT whitespace-invariant"
        grep -E "every failure:|^  [a-z]|PANIC|first difference" <<<"$out" | sed -n '1,30p' >&2
    fi
fi

# --------------------------------------------------------------- phase 2 ----
# Totality over the operand space: every mnemonic whose destination is looked up
# in a register-name table, against every spelling a destination can take (all
# four GP widths, high bytes, xmm/ymm/mm, x87, segment, control, and junk), with
# every source and every padding.  This is the phase that finds an unguarded
# `REG_NAMES[..][fam as usize]` nobody thought to audit.
if want_phase 2 "$ONLY_PHASE"; then
    note "== phase 2: exhaustive operand matrix (ignored in the default suite)"
    out=$(cargo_test the_pipeline_never_panics_on_the_full_operand_matrix --ignored --nocapture 2>&1)
    if grep -q "test result: ok" <<<"$out"; then
        ok "the pipeline never panics on any operand spelling (exhaustive matrix)"
    else
        bad "the pipeline panics on some operand spelling"
        grep -E "PANIC|on input:|panicked" <<<"$out" | sed -n '1,20p' >&2
    fi
fi

# --------------------------------------------------------------- phase 3 ----
# Real compiler output.  The committed .s files are a fixed, small corpus; this
# phase generates assembly from the repository's C tests with the binary under
# test, at four optimization levels, and runs the same invariance property over
# it.  A gate that cannot generate its corpus fails loudly rather than passing
# over nothing.
corpus_dir=""
if want_phase 3 "$ONLY_PHASE" || want_phase 4 "$ONLY_PHASE"; then
    if [[ ! -x "$CCC" ]]; then
        bad "$CCC is missing or not executable: phases 3 and 4 would be vacuous"
    else
        corpus_dir=$(mktemp -d)
        if ! python3 scripts/corpus_selection.py "$repo_root/tests" \
            --every "$CORPUS_EVERY" > "$corpus_dir/sources.list"; then
            bad "phase-3 source enumeration failed"
            sources=()
        else
            mapfile -t sources < "$corpus_dir/sources.list"
        fi
        note "== phase 3: generating assembly from ${#sources[@]} C sources with $CCC"
        generated=0
        compiled_out=0
        for source_index in "${!sources[@]}"; do
            src=${sources[source_index]}
            # Source identity is the position in a unique, stable sorted list;
            # equal basenames cannot overwrite another source's assembly.
            stem="source${source_index}_$(basename "$src" .c)"
            for opt in -O0 -O1 -O2 -O3; do
                dest="$corpus_dir/${stem}${opt}.s"
                if timeout 120 "$CCC" $opt -S "$src" -o "$dest" >/dev/null 2>&1 &&
                    [[ -s "$dest" ]]; then
                    generated=$((generated + 1))
                else
                    compiled_out=$((compiled_out + 1))
                    rm -f "$dest"
                fi
            done
        done
        note "   generated $generated assembly files ($compiled_out source/level pairs did not compile)"
    fi
fi

if want_phase 3 "$ONLY_PHASE"; then
    if [[ -n "$corpus_dir" && -x "$CCC" ]]; then
        if (( generated < 40 )); then
            bad "generated corpus holds only $generated files; refusing to pass over a corpus that small"
        else
            out=$(LCCC_ASM_CORPUS="$corpus_dir" cargo_test peephole_output_is_invariant_to_trailing_whitespace 2>&1)
            if grep -q "test result: ok" <<<"$out"; then
                ok "whitespace invariance holds over $generated freshly generated assembly files"
            else
                bad "whitespace invariance FAILS on freshly generated assembly"
                grep -E "every failure:|^  [a-z]|PANIC|first difference" <<<"$out" | sed -n '1,30p' >&2
            fi
        fi
    fi
fi

# --------------------------------------------------------------- phase 4 ----
# The assembler path is a different parser from the peephole: it slices operands
# in src/backend/x86/assembler, where `is_label_like` and the displacement
# grammar live.  Padding a .s file must not change a single byte of the object
# it assembles to.
if want_phase 4 "$ONLY_PHASE"; then
    if [[ -n "$corpus_dir" && -x "$CCC" ]]; then
        note "== phase 4: padded assembly must assemble to identical objects"
        a="$corpus_dir/a"; b="$corpus_dir/b"; c="$corpus_dir/c"
        mkdir -p "$a" "$b" "$c"
        compared=0
        differ=0
        skipped=0
        while IFS= read -r asm; do
            name=$(basename "$asm")
            cp "$asm" "$a/$name"
            sed 's/$/    /' "$asm" > "$b/$name"                    # four trailing spaces
            sed 's/$/\r/' "$asm" > "$c/$name"                        # CRLF endings
            if ! timeout 120 "$CCC" -c "$a/$name" -o "$a/${name%.s}.o" >/dev/null 2>&1; then
                skipped=$((skipped + 1))
                continue
            fi
            for variant in "$b" "$c"; do
                if ! timeout 120 "$CCC" -c "$variant/$name" -o "$variant/${name%.s}.o" >/dev/null 2>&1; then
                    bad "the assembler REJECTED a padded copy of $name in $variant"
                    differ=$((differ + 1))
                    continue
                fi
                if cmp -s "$a/${name%.s}.o" "$variant/${name%.s}.o"; then
                    compared=$((compared + 1))
                    continue
                fi
                # Byte-identical objects are the strong claim; if the container
                # differs (a path or timestamp in a section), fall back to the
                # code itself before calling it a failure.
                if command -v objcopy >/dev/null 2>&1; then
                    objcopy -O binary --only-section=.text "$a/${name%.s}.o" "$a/t" 2>/dev/null
                    objcopy -O binary --only-section=.text "$variant/${name%.s}.o" "$variant/t" 2>/dev/null
                    if cmp -s "$a/t" "$variant/t"; then
                        compared=$((compared + 1))
                        note "   note: $name ($variant) differs outside .text only"
                        continue
                    fi
                fi
                differ=$((differ + 1))
                bad "assembling $name with padding from $variant produced different code"
                if command -v objdump >/dev/null 2>&1; then
                    diff <(objdump -d "$a/${name%.s}.o" 2>/dev/null) \
                         <(objdump -d "$variant/${name%.s}.o" 2>/dev/null) | sed -n '1,20p' >&2
                fi
            done
        done < <(find "$corpus_dir" -maxdepth 1 -name '*.s' | LC_ALL=C sort | sed -n '1,14p')
        note "   compared $compared padded/unpadded object pairs ($skipped sources did not assemble)"
        if (( differ == 0 )); then
            if (( compared < 10 )); then
                bad "only $compared object pairs compared; too few to be evidence"
            else
                ok "the assembler is whitespace-invariant over $compared object pairs"
            fi
        fi
    fi
fi

[[ -n "$corpus_dir" ]] && rm -rf "$corpus_dir"

note ""
# Nothing ran is a failure, not a pass: an empty RESULTS means the phase
# selector matched no phase, and the old banner then claimed all four.
if (( ${#RESULTS[@]} == 0 )); then
    echo "check_peephole_whitespace: FAILED (no phase ran; selector matched nothing)" >&2
    exit 1
fi
for line in "${RESULTS[@]}"; do printf '%s\n' "$line"; done
if (( fail != 0 )); then
    echo "check_peephole_whitespace: FAILED" >&2
    exit 1
fi
# The banner must describe what was actually verified, so a single-phase run
# cannot read as a four-phase pass.
summary='in-tree corpus, operand totality, generated corpus, assembler path'
[[ -n "$ONLY_PHASE" ]] && summary="phase $ONLY_PHASE only"
echo "check_peephole_whitespace: PASS ($summary)"
