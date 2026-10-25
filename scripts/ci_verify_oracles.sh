#!/usr/bin/env bash
# ============================================================================
# ci_verify_oracles.sh — prove a provisioned lane has the oracles its gates need.
#
# THE DEFECT THIS PREVENTS
# ------------------------
# Provisioning used to end at "the packages installed".  Every lane then ran
# its gates and, if one of them needed a tool the lane did not have, the
# failure surfaced 10-20 minutes later as a *compiler* failure inside an
# unrelated gate:
#
#   * run 39809624659, Fast local contracts (shard b): the FMA contract gate
#     reached its hexadecimal-literal oracle, which refuses to run without
#     valgrind, and reported "LCCC, GCC, Clang and Valgrind are all required".
#     Nothing in the lane had installed valgrind, so the gate could only pass
#     where the tool happened to be present.  A whole 6-minute job was spent to
#     find out that a 2-second probe at the top would have said.
#   * run 39809624659, Fast local contracts (shard a):
#     check_llvm_word_counts.py requires GCC 16.2 EXACTLY (the version the LLVM
#     oracle family was measured against) while provisioning is rolling-major,
#     so the lane could silently receive 16.3 and fail in a *different* gate.
#
# The lesson is not "install more packages".  It is that a lane's toolchain is
# an input contract, and an input contract that is only checked *implicitly*,
# by the first gate that trips over it, is unverifiable: the failure names the
# consumer, not the missing dependency, and it costs the full lane to observe.
#
# WHAT THIS SCRIPT PROVES
# -----------------------
# Every probe below is one a gate actually depends on, and every failure names
# the tool, the expectation and what was found:
#
#   gcc-16 / g++-16     exist, compile C17+C23, and -- only when the lane
#                       passes --gcc-version -- report that version exactly.
#                       The pin is an EXPLICIT per-lane argument: provisioning
#                       is rolling-major by policy, so a lane that does not ask
#                       for 16.2 must not be judged against it.  The first
#                       version of this script defaulted the expectation to
#                       16.2 and turned every unpinned lane red in run
#                       39822157492, where `Build and package compiler` and
#                       `Assertion-enabled units and corpus` had installed the
#                       distribution's 16.0.1 exactly as their gates expect.
#   i686 multilib       links AND runs (qemu-i386 when the kernel refuses)
#   mold, ld.lld        run -- only when a lane asks with --require, because
#   qemu-i386           presence is already provisioning's contract: every lane
#                       runs ci_provision_toolchain.sh, whose ci_toolchain_bundle.sh
#                       verify step refuses to return without /usr/bin/{ld.lld,
#                       qemu-i386,mold}.  Re-probing them here bought nothing and
#                       made the gate unpassable on a host the tag cache cannot
#                       provision (no lld in the cache), which is how a
#                       redundancy becomes a false red.
#   valgrind            runs Memcheck end to end -- not just `--version`:
#                       valgrind aborts at startup when libc6-dbg for the
#                       loader is missing, and that is exactly the state a
#                       bundle without the debug symbols restores
#   python3 yaml        imports (several gates parse the CI plan with it)
#   clang (optional)    when --clang-version is given, $CLANG or `clang` must
#                       report that version EXACTLY (the ext_vector_type and
#                       hex-float oracles pin their reference implementation)
#
# The probe result is emitted as ONE `::notice` annotation, because the raw job
# log is not reachable from the check-run API and "which compiler did this run
# actually use" has to be answerable from the Checks page alone.  It includes
# how the toolchain arrived (bundle restore vs package install), recorded by
# ci_provision_toolchain.sh.
#
# Usage:
#   scripts/ci_verify_oracles.sh [--gcc-version 16.2] [--clang-version 23.1.4]
#                                [--require gcc,multilib,valgrind,pyyaml]
#                                [--valgrind-tool NAME]
#   scripts/ci_verify_oracles.sh --self-test
#
# Exit status 0 means every requested probe passed.  A missing or wrong tool is
# never a skip: the gates it feeds either refuse to run (so the lane would go
# red later, naming the wrong thing) or, worse, run a weaker comparison.
# ============================================================================
set -Eeuo pipefail

# The version a *pinned* lane asks for.  Empty by default: the pin belongs to
# the lane (and to the installer that delivers it), never to the verifier.  The
# lanes that need 16.2 are exactly the ones running the LLVM oracle family
# (tests/regression/check_llvm_*.py), and they install it with
# scripts/ci_install_gcc_oracle.sh before calling this script.
PROVENANCE=$HOME/.cache/lccc-toolchain/last-provision.json

gcc_version_expected=""
clang_version_expected=""
# The binary the Memcheck probe looks for.  It is a parameter so the self-test
# can exercise the "absent tool" path on a host that happens to have valgrind
# installed (a verifier that cannot fail is decoration), and so a lane that
# provisions it under another name can say so.
valgrind_tool=${LCCC_VALGRIND_TOOL:-valgrind}
# The presence probes resolve their binary by name; the names are parameters
# for the same two reasons as valgrind's: the self-test has to exercise the
# "required tool is absent" path on a host where the tool IS installed, and a
# lane that provisions it under another name can say so.
mold_tool=${LCCC_MOLD_TOOL:-mold}
lld_tool=${LCCC_LLD_TOOL:-ld.lld}
qemu_tool=${LCCC_QEMU_TOOL:-qemu-i386}
# Capability probes are on by default; the presence probes are opt-in and the
# union is checked against what the lane's gates actually consume by
# tests/regression/check_ci_lane_plan.py.
required="gcc,multilib,valgrind,pyyaml"
self_test=0

usage() {
    # Through the start of the code, not a fixed line count: this header grows
    # whenever a probe is added, and a range that stops short silently prints
    # nothing at all.
    sed -n '2,/^set -Eeuo/p' "$0" | sed -n '/^# Usage:/,$p' >&2
    exit 2
}

while (( $# )); do
    case $1 in
        --gcc-version) gcc_version_expected=${2:?--gcc-version needs a value}; shift 2 ;;
        --clang-version) clang_version_expected=${2:?--clang-version needs a value}; shift 2 ;;
        --valgrind-tool) valgrind_tool=${2:?--valgrind-tool needs a value}; shift 2 ;;
        --require) required=${2:?--require needs a comma-separated probe list}; shift 2 ;;
        --self-test) self_test=1; shift ;;
        -h|--help) usage ;;
        *) echo "ci_verify_oracles: unknown argument: $1" >&2; exit 2 ;;
    esac
done

# ── annotations ─────────────────────────────────────────────────────────────
# GitHub escapes %, CR and LF in command values; a version string never has
# them, but a tool's stderr does, and an unescaped newline truncates the rest
# of the message in the Checks UI.
escape() {
    local s=$*
    s=${s//'%'/%25}; s=${s//$'\r'/%0D}; s=${s//$'\n'/%0A}
    printf '%s' "$s"
}
notice() {
    if [[ -n ${GITHUB_ACTIONS:-} ]]; then printf '::notice title=%s::%s\n' "$(escape "$1")" "$(escape "${*:2}")"; fi
}
error() {
    if [[ -n ${GITHUB_ACTIONS:-} ]]; then printf '::error title=%s::%s\n' "$(escape "$1")" "$(escape "${*:2}")"; fi
    printf 'ci_verify_oracles: %s: %s\n' "$1" "${*:2}" >&2
}

failures=0
failed() {
    error 'Oracle provisioning' "$@"
    failures=$((failures + 1))
}

# The self-test's diagnosis has to survive the run page, because the hosted job
# log is not reachable from the review environment.  An assertion that only
# writes to stderr leaves a red step that names nothing, which is how a
# CI-only failure in this table cost a full pipeline round to bisect.
case_fail() {
    error 'Oracle self-test' "$@"
    printf '%s\n' "$*" >&2
}

probes=()
IFS=, read -r -a probes <<<"$required"
for probe in "${probes[@]}"; do
    case $probe in
        gcc|multilib|valgrind|pyyaml|mold|lld|qemu) ;;
        *) printf 'ci_verify_oracles: --require: unknown probe %s (gcc,multilib,valgrind,pyyaml,mold,lld,qemu)\n' "$probe" >&2
           exit 2 ;;
    esac
done
# ── probes ──────────────────────────────────────────────────────────────────
# report <label> <value>: collected in probe order and printed as one line.
REPORT=()
report() { REPORT+=("$1=$2"); }

require_gcc() {
    local bin version
    if ! bin=$(command -v gcc-16 2>/dev/null); then
        failed "gcc-16 is not on PATH; the lane cannot run any GCC-oracle gate"
        return
    fi
    if ! version=$("$bin" -dumpfullversion 2>/dev/null) || [[ -z $version ]]; then
        failed "gcc-16 at $bin does not report a version"
        return
    fi
    # The path is part of the report: 16.2 can arrive as the distribution
    # package or as the tag-cache prefix that shadows it, and the two are the
    # same version only by coincidence.  One `::notice` answers which one a run
    # actually used.
    report gcc-16 "$version ($bin)"
    if [[ -n $gcc_version_expected ]]; then
        # A prefix match on the dotted components: the distribution appends its
        # own package revision ("16.2.0" vs "16.2"), and a *minor* movement is
        # what changes the oracle.  16.3 must not pass an expectation of 16.2.
        case $version in
            "$gcc_version_expected"|"$gcc_version_expected".*) ;;
            *) failed "gcc-16 is $version; this lane pins $gcc_version_expected (scripts/ci_install_gcc_oracle.sh --version $gcc_version_expected delivers it)" ;;
        esac
    fi
    if ! command -v g++-16 >/dev/null 2>&1; then
        failed "g++-16 is not on PATH but gcc-16 is; the C++ oracles would fail later"
    fi
    # C17 and C23 both: the corpus and the typed-arithmetic gates compile with
    # -std=c23, and a gcc-16 that cannot parse it is not the oracle.
    local std
    for std in c17 c23; do
        # A here-string, not `printf | gcc`: a producer that loses the race to a
        # consumer which exits early takes SIGPIPE (141) and, under
        # `set -o pipefail`, reports a compiler failure that never happened.
        if ! "$bin" -std="$std" -x c -fsyntax-only - >/dev/null 2>&1 <<<'int main(void) { return 0; }'; then
            failed "gcc-16 cannot compile -std=$std"
        fi
    done
}

require_multilib() {
    # Linking is not running: the i686 rows execute the binary, through
    # qemu-i386 when the kernel refuses 32-bit processes.  Check both halves.
    local work rc=0 run=()
    work=$(mktemp -d)
    if ! gcc-16 -m32 -x c - -o "$work/m32" >/dev/null 2>&1 \
            <<<$'#include <stdio.h>\nint main(void) { puts("ok"); return 0; }'; then
        failed "gcc-16 -m32 cannot link: the i686 multilib is incomplete"
        rm -rf "$work"; return
    fi
    "$work/m32" >"$work/out" 2>"$work/err" || rc=$?
    if (( rc == 126 || rc == 127 )); then
        if command -v qemu-i386 >/dev/null 2>&1; then
            run=(qemu-i386)
        else
            failed "the kernel refuses 32-bit binaries and qemu-i386 is missing; every i686 gate would fail"
            rm -rf "$work"; return
        fi
    fi
    if ! "${run[@]}" "$work/m32" >"$work/out" 2>"$work/err"; then
        failed "the -m32 probe does not run: $(sed -n '1p' "$work/err")"
    elif [[ $(sed -n '1p' "$work/out") != ok ]]; then
        failed "the -m32 probe printed '$(sed -n '1p' "$work/out")' instead of 'ok'"
    else
        report multilib "links and runs${run:+ via ${run[0]}}"
    fi
    rm -rf "$work"
}

require_runner() { # require_runner <label> <binary>
    local label=$1 tool=$2 version
    if ! command -v "$tool" >/dev/null 2>&1; then
        failed "$tool is not on PATH"
        return
    fi
    if ! version=$("$tool" --version 2>&1); then
        failed "$tool does not run"
        return
    fi
    version=${version%%$'\n'*}
    report "$label" "$version"
}

require_valgrind() {
    local bin version work
    if ! bin=$(command -v "$valgrind_tool" 2>/dev/null); then
        failed "$valgrind_tool is not on PATH: the hexadecimal/FP Memcheck oracles refuse to run without it (provision it through ci_provision_toolchain.sh EXTRAS)"
        return
    fi
    version=$("$bin" --version 2>&1) || { failed "valgrind does not run"; return; }
    report valgrind "$version"
    # `--version` proves the binary was linked, not that Memcheck works.  The
    # loader redirection needs libc6-dbg, and without it valgrind aborts before
    # it can match a single symbol -- the exact state a bundle exported without
    # the debug package restores.
    work=$(mktemp -d)
    if ! gcc-16 -g -x c - -o "$work/vg" >/dev/null 2>&1 <<<'int main(void) { return 0; }'; then
        failed "the Memcheck smoke probe did not compile"
    elif ! "$bin" --error-exitcode=99 --leak-check=full -q "$work/vg" >"$work/log" 2>&1; then
        failed "valgrind cannot complete a Memcheck run (missing libc6-dbg for the loader?): $(sed -n '1,3p' "$work/log" | tr '\n' ' ')"
    fi
    rm -rf "$work"
}

require_python_yaml() {
    if ! python3 -c 'import yaml' >/dev/null 2>&1; then
        failed "python3 cannot import yaml (the CI plan and benchmark gates parse YAML)"
        return
    fi
    report pyyaml "$(python3 -c 'import yaml; print(yaml.__version__)')"
}

require_clang() {
    local wanted=$1 cand=${CLANG:-clang} bin version
    if ! bin=$(command -v "$cand" 2>/dev/null); then
        failed "clang is not on PATH (looked for '$cand'): install the pinned oracle with scripts/ci_install_clang_oracle.sh"
        return
    fi
    if ! version=$("$bin" -dumpversion 2>/dev/null); then
        failed "$bin does not report a version"
        return
    fi
    report clang "$version ($bin)"
    if [[ -n $wanted && $version != "$wanted" ]]; then
        failed "clang at $bin is $version; the oracles pin $wanted exactly (tests/regression/lib/clang_oracle.sh)"
    fi
}

self_test() {
    # The probe logic is exercised against synthetic tools, because the whole
    # point of this gate is to FAIL for a lane that lacks something: a verifier
    # that cannot fail is decoration.  The mock `gcc-16` is a *compiler*: it
    # parses -o, writes an executable that prints "ok" there, and answers
    # -dumpfullversion from CASES_VERSION.  That keeps the multilib and C17/C23
    # probes real (they must actually run a binary) while the version is under
    # the test's control.
    local work status real_python
    real_python=$(command -v python3)
    work=$(mktemp -d)
    mkdir -p "$work/tools" "$work/clang18" "$work/clang23"
    # A python3 that answers the YAML probe: the verifier must not depend on
    # the host's site-packages to test its own logic, and the real interpreter
    # is still used for everything else (the test mocks one probe at a time).
    cat >"$work/tools/python3" <<MOCK
#!/bin/sh
if [ "\$1" = "-c" ]; then
    case "\$2" in
        *"import yaml"*) case "\$2" in *print*) echo 6.0 ;; esac; exit 0 ;;
    esac
fi
exec "$real_python" "\$@"
MOCK
    chmod +x "$work/tools/python3"
    cat >"$work/tools/gcc-16" <<'MOCK'
#!/bin/sh
for a in "$@"; do
    case $a in
        -dumpfullversion) printf '%s\n' "${CASES_VERSION:-16.2.0}"; exit 0 ;;
    esac
done
cat >/dev/null 2>&1 || true
out=""
prev=""
for a in "$@"; do
    case $prev in -o) out=$a ;; esac
    prev=$a
done
if [ -n "$out" ]; then
    printf '#!/bin/sh\necho ok\n' >"$out"
    chmod +x "$out"
fi
exit 0
MOCK
    cp "$work/tools/gcc-16" "$work/tools/g++-16"
    for tool in mold ld.lld qemu-i386; do
        printf '#!/bin/sh\necho "%s 1.0"\n' "$tool" >"$work/tools/$tool"
    done
    printf '#!/bin/sh\necho "valgrind-3.19.0"\n' >"$work/tools/valgrind"
    printf '#!/bin/sh\necho 18.1.3\n' >"$work/clang18/clang"
    printf '#!/bin/sh\necho 23.1.4\n' >"$work/clang23/clang"
    chmod +x "$work/tools"/* "$work/clang18/clang" "$work/clang23/clang"
    # The dev host this gate also runs on: the tag cache ships no ld.lld, so a
    # host can be fully able to run every gate and still have no linker oracle.
    mkdir -p "$work/tools-min"
    cp "$work/tools/gcc-16" "$work/tools/g++-16" "$work/tools/python3" \
       "$work/tools/valgrind" "$work/tools-min/"

    # 1. A complete, correct tool set passes and reports every version.
    status=0
    PATH="$work/tools:$work/clang23:$PATH" CLANG=clang CASES_VERSION=16.2.0 \
        bash "$0" --gcc-version 16.2 --clang-version 23.1.4 >"$work/out" 2>"$work/err" || status=$?
    if (( status == 0 )) && grep -q 'gcc-16=16.2.0' "$work/out" \
            && grep -q 'clang=23.1.4' "$work/out" && grep -q 'valgrind=valgrind-3.19.0' "$work/out"; then
        echo "  ok   complete tool set: exit 0 with every version reported"
    else
        case_fail "  FAIL complete tool set: exit $status, out=$(sed -n '1,3p' "$work/out") err=$(sed -n '1,5p' "$work/err")"
        rm -rf "$work"; return 1
    fi

    # 2. A minor-version drift is refused, and the message names both versions.
    status=0
    PATH="$work/tools:$PATH" CLANG=clang CASES_VERSION=16.3.0 \
        bash "$0" --gcc-version 16.2 >"$work/out" 2>"$work/err" || status=$?
    if (( status != 0 )) && grep -q 'is 16.3.0; this lane pins 16.2' "$work/err"; then
        echo "  ok   gcc minor drift refused with both versions named"
    else
        case_fail "  FAIL gcc minor drift: exit $status, stderr=$(sed -n '1,5p' "$work/err")"
        rm -rf "$work"; return 1
    fi

    # 3. A missing valgrind is a failure naming the tool, not a skip.
    mv "$work/tools/valgrind" "$work/tools/valgrind.off"
    status=0
    PATH="$work/tools:/usr/bin:/bin" CLANG=clang CASES_VERSION=16.2.0 \
        bash "$0" --valgrind-tool valgrind-case3-absent >"$work/out" 2>"$work/err" || status=$?
    if (( status != 0 )) && grep -qi 'valgrind-case3-absent is not on PATH' "$work/err"; then
        echo "  ok   missing valgrind fails closed and names the tool"
    else
        case_fail "  FAIL missing valgrind: exit $status, stderr=$(sed -n '1,5p' "$work/err")"
        rm -rf "$work"; return 1
    fi
    mv "$work/tools/valgrind.off" "$work/tools/valgrind"

    # 4. A clang that is not the pinned release is refused when a version is
    #    requested, and tolerated when none is.
    status=0
    PATH="$work/tools:$work/clang18:$PATH" CLANG=clang CASES_VERSION=16.2.0 \
        bash "$0" --gcc-version 16.2 --clang-version 23.1.4 >"$work/out" 2>"$work/err" || status=$?
    if (( status != 0 )) && grep -q 'is 18.1.3; the oracles pin 23.1.4' "$work/err"; then
        echo "  ok   unpinned clang refused when the lane asks for the pinned oracle"
    else
        case_fail "  FAIL pinned clang: exit $status, stderr=$(sed -n '1,5p' "$work/err")"
        rm -rf "$work"; return 1
    fi
    status=0
    PATH="$work/tools:$work/clang18:$PATH" CLANG=clang CASES_VERSION=16.2.0 \
        bash "$0" --gcc-version 16.2 >"$work/out" 2>"$work/err" || status=$?
    if (( status == 0 )) && ! grep -q 'clang=' "$work/out"; then
        echo "  ok   a lane that does not compare against Clang does not require it"
    else
        case_fail "  FAIL clang-free lane: exit $status, out=$(sed -n '1,3p' "$work/out") err=$(sed -n '1,5p' "$work/err")"
        rm -rf "$work"; return 1
    fi

    # 5. The regression this script caused, twice, in run 39822157492: an
    #    UNPINNED lane was judged against 16.2.  The distribution's 16.0.1 is
    #    the rolling-major oracle those lanes ask for, so it must pass.
    status=0
    PATH="$work/tools:$PATH" CLANG=clang CASES_VERSION=16.0.1 \
        bash "$0" >"$work/out" 2>"$work/err" || status=$?
    if (( status == 0 )) && grep -q 'gcc-16=16.0.1' "$work/out"; then
        echo "  ok   an unpinned lane is not judged against the 16.2 pin"
    else
        case_fail "  FAIL unpinned lane: exit $status, out=$(sed -n '1,3p' "$work/out") err=$(sed -n '1,5p' "$work/err")"
        rm -rf "$work"; return 1
    fi

    # 6. ...while a lane that DOES ask for 16.2 must refuse that same oracle,
    #    naming both versions so the annotation is the whole diagnosis.
    status=0
    PATH="$work/tools:$PATH" CLANG=clang CASES_VERSION=16.0.1 \
        bash "$0" --gcc-version 16.2 >"$work/out" 2>"$work/err" || status=$?
    if (( status != 0 )) && grep -q 'gcc-16 is 16.0.1; this lane pins 16.2' "$work/err"; then
        echo "  ok   a pinned lane refuses 16.0.1 and names both versions"
    else
        case_fail "  FAIL pinned lane: exit $status, stderr=$(sed -n '1,5p' "$work/err")"
        rm -rf "$work"; return 1
    fi

    # 7. Presence probes are opt-in.  mold/ld.lld/qemu-i386 are proven by the
    #    provisioning step's own bundle verify, so the default set must pass on
    #    a host that has none of them (the research box has mold and qemu but
    #    no lld; making the default require lld turned ci_local.sh red).
    status=0
    PATH="$work/tools-min:/usr/bin:/bin" LCCC_LLD_TOOL=lld-case7-absent CLANG=clang CASES_VERSION=16.2.0 \
        bash "$0" --gcc-version 16.2 >"$work/out" 2>"$work/err" || status=$?
    if (( status == 0 )) && ! grep -q 'lld-case7-absent' "$work/err"; then
        echo "  ok   the default require set does not demand the presence probes"
    else
        case_fail "  FAIL default require set: exit $status, err=$(sed -n '1,5p' "$work/err")"
        rm -rf "$work"; return 1
    fi

    # 8. ...and a lane that DOES ask for one gets a failure that names it.
    status=0
    PATH="$work/tools-min:/usr/bin:/bin" LCCC_LLD_TOOL=lld-case8-absent CLANG=clang CASES_VERSION=16.2.0 \
        bash "$0" --require gcc,lld >"$work/out" 2>"$work/err" || status=$?
    if (( status != 0 )) && grep -q 'lld-case8-absent is not on PATH' "$work/err"; then
        echo "  ok   --require lld fails closed and names the tool"
    else
        case_fail "  FAIL --require lld: exit $status, err=$(sed -n '1,5p' "$work/err")"
        rm -rf "$work"; return 1
    fi

    notice 'Oracle self-test' '9/9 cases pass'
    # 9. The override itself is part of the contract: the clang installer
    #    exports CLANG as an ABSOLUTE path, so every lane after it resolves
    #    that binary.  A fixture that only ever looks at PATH cannot see the
    #    real entry point, and a broken override would go unnoticed until a
    #    lane silently compared against the runner's /usr/bin/clang.
    status=0
    PATH="$work/tools:$PATH" CLANG="$work/clang18/clang" CASES_VERSION=16.2.0 \
        bash "$0" --gcc-version 16.2 --clang-version 23.1.4 >"$work/out" 2>"$work/err" || status=$?
    if (( status != 0 )) && grep -q 'is 18.1.3; the oracles pin 23.1.4' "$work/err"; then
        echo "  ok   the CLANG override is honoured, and the pin applies through it"
    else
        case_fail "  FAIL CLANG override: exit $status, err=$(sed -n '1,5p' "$work/err")"
        rm -rf "$work"; return 1
    fi

    notice 'Oracle self-test' '10/10 cases pass'
    echo "  self-test: 10/10 cases pass"
    rm -rf "$work"
    return 0
}

if (( self_test )); then
    self_test
    exit $?
fi

# ── run ─────────────────────────────────────────────────────────────────────
for probe in "${probes[@]}"; do
    case $probe in
        gcc) require_gcc ;;
        multilib) require_multilib ;;
        valgrind) require_valgrind ;;
        pyyaml) require_python_yaml ;;
        mold) require_runner mold "$mold_tool" ;;
        lld) require_runner ld.lld "$lld_tool" ;;
        qemu) require_runner qemu-i386 "$qemu_tool" ;;
    esac
done
if [[ -n $clang_version_expected ]]; then require_clang "$clang_version_expected"; fi

provenance=unknown
if [[ -r $PROVENANCE ]]; then
    provenance=$(python3 - "$PROVENANCE" <<'EOF' 2>/dev/null || echo unknown
import json, sys
d = json.load(open(sys.argv[1]))
secs = d.get('toolchain_seconds', '?')
# The writer records a display string ("9.6s"), a number, or nothing; the
# notice must not turn "9.6s" into "9.6ss", which is what the Checks page
# showed for every lane.
if isinstance(secs, (int, float)):
    secs = f"{secs:.1f}s"
elif not str(secs).rstrip().endswith("s"):
    secs = f"{secs}s"
print(f"{d.get('source', '?')} ({secs}, {d.get('at', '?')})")
EOF
)
fi

report provenance "$provenance"
notice 'Oracle versions' "${REPORT[*]}"

if (( failures )); then
    printf 'ci_verify_oracles: %d probe(s) failed\n' "$failures" >&2
    exit 1
fi
printf 'ci_verify_oracles: OK (%s)\n' "${REPORT[*]}"
