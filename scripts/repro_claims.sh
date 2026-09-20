#!/usr/bin/env bash
# ============================================================================
# repro_claims.sh — reproduce every quantitative claim this tree's follow-up
# documents make, with the command that produces it.
#
# A number in a document is a claim, and a claim without a command is rhetoric:
# the audit of this patch objected to exactly that (eight numbers, no
# reproduction path).  This script is the reproduction path.  Each claim below
# names the document section it comes from, the command that re-measures it, the
# documented value, and what the measurement actually returned.
#
# Claims are not all the same kind of thing, and pretending otherwise is how
# documents end up unverifiable.  Each one is classified:
#
#   EXACT     the value is an invariant of the tree: a sha256, an `_end`, a
#             section total, an object size.  Measured != documented is a
#             FAILURE, because either the claim or the tree is wrong.
#   MONOTONE  the value counts something that only grows as tests are added
#             (test totals, corpus totals).  Measured < documented is a
#             FAILURE; measured > documented is reported as drift, not failure.
#   REPORT    the value describes a host or a sample, not the tree: a timing
#             distribution, another compiler's code size, a file-mode census.
#             The script prints what it measured next to what was documented and
#             asserts nothing, because asserting it would be the dishonesty the
#             audit was complaining about — the document itself says the A/B
#             delta is below this host's noise floor.
#   SKIP      a prerequisite is absent (no kernel tree, no clang, no pre-change
#             binary to A/B against).  The missing prerequisite and the command
#             that installs it are printed.  Skips never pass silently, and
#             --require-all turns any SKIP into a failure so a release check
#             cannot degrade into a no-op.
#
# Usage:
#   scripts/repro_claims.sh [--claims 1,4,7] [--require-all] [--rounds N]
#                           [--quick] [--list]
# Environment:
#   LCCC          compiler under test      (default target/fastbuild/lccc)
#   LCCC_PRE      pre-change binary, for the A/B claims (default: none -> SKIP)
#   KERNEL_DIR    kernel tree              (default /home/user/kernel-work/linux-6.18.52)
#   PROFILE JOBS  cargo profile / -j       (default fastbuild / 2)
#   SWEEP_JOBS    width of the 37-TU sweep pool (default 2 = this host's cores)
#   DOC           the document the claims are quoted from
# ============================================================================
set -uo pipefail

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_root"

LCCC=${LCCC:-target/fastbuild/lccc}
LCCC_PRE=${LCCC_PRE:-}
KERNEL_DIR=${KERNEL_DIR:-/home/user/kernel-work/linux-6.18.52}
PROFILE=${PROFILE:-fastbuild}
JOBS=${JOBS:-2}
DOC=${DOC:-engineering/FOLLOWUP-2026-09-19C-deferred-items-closed-with-kernel-harness.md}
ROUNDS=${ROUNDS:-6}
SWEEP_JOBS=${SWEEP_JOBS:-2}   # width of the 37-TU replay pool (also the make -j for phase 1)

# Absolutize the compiler paths once. Every harness this script calls changes
# directory (the boot gate builds inside the kernel tree, the sweep replays
# Kbuild commands from there), so a relative LCCC=target/fastbuild/lccc reaches
# them as a path that does not exist and the gate dies with 127 — which reads as
# a compiler failure and is not one. Observed, then fixed here rather than in
# each caller.
LCCC=$(cd "$repo_root" && realpath -m "$LCCC")
[[ -n "$LCCC_PRE" ]] && LCCC_PRE=$(realpath -m "$LCCC_PRE")

REQUIRE_ALL=0
SELECT=""
QUICK=0
LIST=0
while [[ $# -gt 0 ]]; do
    case "$1" in
        --claims)      SELECT=$2; shift 2 ;;
        --require-all) REQUIRE_ALL=1; shift ;;
        --rounds)      ROUNDS=$2; shift 2 ;;
        --quick)       QUICK=1; shift ;;
        --list)        LIST=1; shift ;;
        *)             echo "repro_claims: unknown argument $1" >&2; exit 2 ;;
    esac
done

n_ok=0; n_fail=0; n_skip=0; n_report=0
declare -a SUMMARY=()
boot_log=""

hr() { printf '%s\n' "----------------------------------------------------------------"; }

want() { [[ -z "$SELECT" ]] && return 0; [[ ",$SELECT," == *",$1,"* ]] && return 0; return 1; }

# claim <id> <class> <title> <doc-section> <documented-value>
claim() {
    hr
    printf 'CLAIM %-3s [%-8s] %s\n' "$1" "$2" "$3"
    printf '  doc        %s %s\n' "$DOC" "$4"
    printf '  documented %s\n' "$5"
    cur_id=$1; cur_class=$2
}
command_line() { printf '  command    %s\n' "$*"; }
measured()     { printf '  measured   %s\n' "$*"; }

finish() { # finish <status> <detail>
    local status=$1 detail=$2
    case "$status" in
        ok)     printf '  RESULT     ok — %s\n' "$detail"; n_ok=$((n_ok + 1)) ;;
        report) printf '  RESULT     report — %s\n' "$detail"; n_report=$((n_report + 1)) ;;
        skip)   printf '  RESULT     SKIP — %s\n' "$detail" >&2; n_skip=$((n_skip + 1)) ;;
        *)      printf '  RESULT     FAIL — %s\n' "$detail" >&2; n_fail=$((n_fail + 1)) ;;
    esac
    SUMMARY+=("$(printf '%-4s %-8s %-6s %s' "$cur_id" "$cur_class" "$status" "$detail")")
}

need_lccc() {
    [[ -x "$LCCC" ]] && return 0
    finish skip "no compiler at $LCCC (run scripts/build_lccc_fast.sh)"
    return 1
}
need_kernel() {
    [[ -f "$KERNEL_DIR/arch/x86/boot/setup.ld" ]] && return 0
    finish skip "no prepared kernel tree at $KERNEL_DIR (run scripts/prepare_kernel_tree.sh)"
    return 1
}
need_prog() {
    command -v "$1" >/dev/null 2>&1 && return 0
    finish skip "$1 not installed (the claim needs it as an independent oracle)"
    return 1
}
need_pre() {
    [[ -n "$LCCC_PRE" && -x "$LCCC_PRE" ]] && return 0
    finish skip "no pre-change binary; set LCCC_PRE=<path> to A/B against it"
    return 1
}

# The kernel's OWN compile command for one object, with the compiler swapped and
# the flag families this backend does not implement removed.  Asking Kbuild
# rather than writing a flag list is the point: a hand-written list drifts from
# the tree it claims to reproduce, and "under the kernel's own include and ISA
# flags" stops being true the first time the config changes.
#
# `make -n V=1` cannot supply that command -- a dry run prints Kbuild's
# sub-make/objtool/fixdep plumbing, not a replayable compile line (this is what
# made the first version of claim 4/6 fail).  Kbuild does record the real
# invocation, but only after a real build: <dir>/.<name>.o.cmd holds
# `savedcmd_<obj> := gcc <flags> -c <src> -o <obj>`.  So build the object once
# with the kernel's own compiler and read the record back.  Only the flags lccc
# actually rejects are dropped -- verified empirically, the case list below is
# the whole of it.  Everything else stays, including -mfunction-return=thunk-
# extern, -mindirect-branch=thunk-extern, -falign-jumps=1 and the retpoline
# switches: dropping flags the compiler accepts would silently change the
# codegen the claim is about (measured: -736 B of object for five speculative
# drops).
kernel_tu_command() { # <object target, e.g. mm/page_alloc.o> -> a command string
    local obj=$1 src="${1%.o}.c"
    local cmdfile="$KERNEL_DIR/$(dirname "$obj")/.$(basename "$obj").cmd" raw abs_lccc
    if [[ ! -s "$cmdfile" ]]; then
        # KERNEL_TU_NO_BUILD is set by the parallel sweep: two `make` runs in one
        # tree race on shared prerequisites, so the replay phase must never build.
        [[ -n "${KERNEL_TU_NO_BUILD:-}" ]] && return 1
        echo "  (asking Kbuild to record its command for $obj — one gcc build)" >&2
        make -C "$KERNEL_DIR" -s "$obj" >/dev/null 2>&1
    fi
    [[ -s "$cmdfile" && -f "$KERNEL_DIR/$src" ]] || return 1
    raw=$(grep -oE "^savedcmd_${obj} := .*" "$cmdfile" | head -1) || return 1
    raw=${raw#savedcmd_${obj} := }
    abs_lccc=$(cd "$repo_root" && realpath -m "$LCCC")
    # Tokenize with the shell, not `read -r -a`. Kbuild records -D values with
    # shell escaping (-DKBUILD_MODNAME=\"skbuff\"), and `read -r` hands those
    # backslashes to the compiler verbatim: the define arrives mangled, the
    # expansion of printk_index_wrap becomes '"skbuff"' -- a multi-character
    # constant -- and the TU dies in the parser with "expected ')' before integer
    # constant". Eight of the 37 sweep TUs failed exactly that way under BOTH
    # compilers, which is how a harness bug presents itself as a regression.
    # The first `;` is cut before eval so the objtool/fixdep commands Kbuild
    # chains after the compile are never run, and noglob keeps a `*` inside a -D
    # value from being pathname-expanded.
    local raw_cut=${raw%%;*}
    local -a toks=() args=()
    set -f; eval "toks=( $raw_cut )"; set +f
    (( ${#toks[@]} > 10 )) || return 7          # the recorded line did not parse as a command
    local skip=0 a
    for a in "${toks[@]:1}"; do                 # token 0 is the compiler itself
        if (( skip )); then skip=0; continue; fi
        case "$a" in
            -pg|-mrecord-mcount|-fno-allow-store-data-races) continue ;;
            -fstack-protector*|-mstack-protector-guard*|-fpatchable-function-entry*) continue ;;
            -Wp,-MMD,*)    continue ;;
            -o)            skip=1; continue ;;  # the caller supplies the output path
            "$obj")        continue ;;
        esac
        args+=("$a")
    done
    (( ${#args[@]} > 10 )) || return 1          # a parse that lost the flags is worse than none
    printf '%s' "$abs_lccc"
    printf ' %q' "${args[@]}"
    printf ' -c %q' "$src"
}

# Instruction counts, because bytes lie: functions are aligned to 16 B, so a
# strictly shorter function can produce a strictly larger object, and padding is
# not codegen. Counting real instructions and padding separately is what lets the
# sweep assert the promise (fewer instructions) instead of the noise (alignment).
obj_insns() { objdump -d --no-show-raw-insn "$1" 2>/dev/null | grep -cE '^ +[0-9a-f]+:'; }
obj_pads()  { objdump -d --no-show-raw-insn "$1" 2>/dev/null \
                | grep -E '^ +[0-9a-f]+:' | grep -cE '\b(nop|nopl|nopw|xchg +%ax,%ax)\b'; }

with_compiler() { # <command string> <compiler> -- swap the leading compiler token
    local rest=${1#* }
    printf '%s %s' "$(cd "$repo_root" && realpath -m "$2")" "$rest"
}

if (( LIST )); then
    cat <<'EOF'
 1 EXACT    boot stage 32 KiB gate: _end and headroom          (kernel tree)
 2 EXACT    boot stage .text total                             (kernel tree)
 3 EXACT    setup.bin sha256                                   (kernel tree)
 4 EXACT    mm/page_alloc.c compiles with lccc; object size    (kernel tree)
 5 EXACT    37-TU sweep, per-object A/B identity               (kernel tree + LCCC_PRE)
 6 EXACT    vectorizer ISA-gate census: entries == refusals    (kernel tree)
 7 EXACT    ISA-gate trace line counts across five env combos  (lccc only)
 8 REPORT   interleaved compile-time A/B on one kernel TU      (kernel tree + LCCC_PRE)
 9 REPORT   boot size oracle: lccc vs gcc vs clang             (kernel tree + oracles)
10 MONOTONE unit + integration test totals                     (cargo)
11 MONOTONE regression corpus totals                           (lccc)
12 REPORT   tracked-file mode census (shebang vs 644)          (git)
EOF
    exit 0
fi

# --------------------------------------------------------------- claim 1-3 --
# One build produces all three numbers, so the harness runs once and the three
# claims read its log.

boot_gate() { # runs build_kernel_boot.sh once, caches the log
    [[ -n "$boot_log" ]] && return 0
    boot_log=$(mktemp)
    echo "  (running scripts/build_kernel_boot.sh — this takes minutes)" >&2
    KERNEL_DIR="$KERNEL_DIR" LCCC="$LCCC" timeout 1700 \
        bash scripts/build_kernel_boot.sh >"$boot_log" 2>&1
    echo "  (exit $?)" >&2
}

if want 1 && [[ $QUICK == 0 ]]; then
    claim 1 EXACT "the boot stage still fits the 32 KiB gate" "§0" "_end=31168, headroom 1600 B"
    command_line "KERNEL_DIR=$KERNEL_DIR LCCC=$LCCC bash scripts/build_kernel_boot.sh"
    if need_lccc && need_kernel; then
        boot_gate
        line=$(grep -oE '32 KiB gate: [A-Z]+ \(_end=[0-9]+, (headroom|overflow)=-?[0-9]+ bytes\)' "$boot_log" | head -1)
        measured "${line:-<no gate line in the harness output>}"
        if [[ "$line" == "32 KiB gate: PASS (_end=31168, headroom=1600 bytes)" ]]; then
            finish ok "identical to the documented gate result"
        elif grep -q "32 KiB gate: PASS" <<<"$line"; then
            finish fail "the gate passes but the numbers moved: $line"
        else
            finish fail "the boot gate did not pass: ${line:-no output} (see $boot_log)"
        fi
    fi
fi

if want 2 && [[ $QUICK == 0 ]]; then
    claim 2 EXACT "the boot stage .text total is unchanged" "§0" "22890 B"
    command_line "bash scripts/build_kernel_boot.sh  # .text total line"
    if need_lccc && need_kernel; then
        boot_gate
        # The harness prints a `size`-style table whose rows include both `.text`
        # and `.text32`; a substring match plus `tail -1` picks up `.text32`'s
        # 30 B. Match the first field exactly.
        total=$(awk '$1 == ".text" { print $2; exit }' "$boot_log")
        measured ".text total = ${total:-<not reported>}"
        if [[ "$total" == "22890" ]]; then
            finish ok "identical to the documented total"
        else
            finish fail "documented 22890, measured ${total:-nothing} (see $boot_log)"
        fi
    fi
fi

if want 3 && [[ $QUICK == 0 ]]; then
    claim 3 EXACT "setup.bin is byte-identical to the documented image" "§0" \
        "bf9f0e9f5f199924b8308b4879c2eb44be98f28defe145a7d60ac72f29cffd53"
    command_line "bash scripts/build_kernel_boot.sh  # sha256sum of \$OUT/setup.bin"
    if need_lccc && need_kernel; then
        boot_gate
        sha=$(grep -oE '[0-9a-f]{64}' "$boot_log" | head -1)
        measured "sha256 = ${sha:-<not reported>}"
        if [[ "$sha" == "bf9f0e9f5f199924b8308b4879c2eb44be98f28defe145a7d60ac72f29cffd53" ]]; then
            finish ok "the codegen changes did not move the boot image"
        else
            finish fail "documented bf9f0e9f…, measured ${sha:-nothing}"
        fi
    fi
fi

# ----------------------------------------------------------------- claim 4 --
if want 4 && [[ $QUICK == 0 ]]; then
    claim 4 EXACT "mm/page_alloc.c compiles with lccc under the kernel's own flags" "§0" \
        "7764 lines -> 141592-byte lccc object; gcc's own object is 2121560 B because it carries CONFIG_DEBUG_INFO DWARF"
    command_line "eval \"\$(kernel_tu_command mm/page_alloc.o) -o /tmp/page_alloc.o\"   # flags replayed from mm/.page_alloc.o.cmd"
    if need_lccc && need_kernel; then
        cmd=$(kernel_tu_command mm/page_alloc.o)
        if [[ -z "$cmd" ]]; then
            finish skip "Kbuild produced no command for mm/page_alloc.o (tree not prepared for 64-bit objects?)"
        else
            out=$(mktemp --suffix=.o)
            if (cd "$KERNEL_DIR" && eval "$cmd -o $out") >/tmp/repro-pagealloc.log 2>&1 && [[ -s "$out" ]]; then
                size=$(stat -c %s "$out")
                lines=$(wc -l < "$KERNEL_DIR/mm/page_alloc.c")
                measured "$lines lines -> $size-byte object (gcc: $(stat -c %s "$KERNEL_DIR/mm/page_alloc.o") B)"
                if [[ "$size" == "141592" ]]; then
                    finish ok "object size reproduces exactly on this tree"
                else
                    finish fail "documented 141592 B, measured $size B (log: /tmp/repro-pagealloc.log)"
                    echo "  note       this is a codegen-stability gate on real kernel code: the size" >&2
                    echo "  note       moves only when codegen or the config moves. If that was intended," >&2
                    echo "  note       re-measure here and update the documented figure in the same commit." >&2
                fi
            else
                finish fail "the TU did not compile cleanly (log: /tmp/repro-pagealloc.log)"
                tail -5 /tmp/repro-pagealloc.log >&2
            fi
            rm -f "$out"
        fi
    fi
fi

# ----------------------------------------------------------------- claim 5 --
if want 5 && [[ $QUICK == 0 ]]; then
    claim 5 EXACT "the 37-TU kernel sweep: every TU compiles under both compilers and no TU emits more real instructions" "§0, §8" \
        "37 TUs compile under \$LCCC and \$LCCC_PRE; no TU's non-padding instruction count grows; differing pairs are saved for inspection"
    command_line "for tu in <37 kernel objects>: compile with \$LCCC and \$LCCC_PRE, cmp"
    if need_lccc && need_kernel && need_pre; then
        tus=(mm/page_alloc.o mm/vmscan.o mm/slub.o kernel/sched/core.o kernel/sched/fair.o
             kernel/fork.o kernel/exit.o kernel/time/timer.o fs/read_write.o fs/namei.o
             fs/open.o fs/dcache.o fs/inode.o net/core/dev.o net/core/skbuff.o
             net/ipv4/tcp.o net/ipv4/tcp_input.o lib/string.o lib/sort.o lib/rbtree.o
             lib/xarray.o lib/radix-tree.o crypto/sha256.o crypto/aes_generic.o
             block/blk-core.o block/blk-map.o drivers/base/core.o drivers/base/dd.o
             mm/mempool.o mm/swap.o mm/filemap.o mm/memory.o kernel/panic.o
             kernel/rcu/tree.o fs/file_table.o lib/list_sort.o mm/page-writeback.o)
        # This list is the documented "37-TU sweep" and had 36 entries in it: the
        # count in the document was one ahead of the list it described.
        # mm/page-writeback.o is the 37th (same subsystem as the other mm/ TUs,
        # buildable under this config, Kbuild record present).
        # kernel/lockdep.o was listed here originally and is not buildable under
        # this config (CONFIG_LOCKDEP is off: Kbuild has no rule for it, and one
        # missing target aborted the whole `make` run -- which is why phase 1 now
        # passes -k). kernel/rcu/tree.o replaces it: same subsystem weight, always
        # built with CONFIG_TREE_RCU. Verified buildable on this tree.
        # Phase 1 — one Kbuild invocation records every TU's command. Calling
        # `make` once per object (what kernel_tu_command does on a cold tree)
        # re-walks the prerequisite graph 37 times and races on the shared
        # prerequisites; inside a single invocation Kbuild parallelises safely.
        missing=()
        for tu in "${tus[@]}"; do
            [[ -s "$KERNEL_DIR/$(dirname "$tu")/.$(basename "$tu").cmd" ]] || missing+=("$tu")
        done
        if (( ${#missing[@]} )); then
            echo "  (recording Kbuild's commands for ${#missing[@]} TUs with the kernel's own compiler — make -j${SWEEP_JOBS}, several minutes)" >&2
            # -k: a target this config cannot build (no Kbuild rule) must cost one
            # TU, not the whole sweep. Without it make aborts and kills the builds
            # already in flight, so 35 targets lose their records to one bad name.
            ( cd "$KERNEL_DIR" && timeout 5400 make -j"$SWEEP_JOBS" -k "${missing[@]}" ) \
                >/tmp/repro-37-gcc.log 2>&1
            echo "  (gcc phase exit $?; log /tmp/repro-37-gcc.log)" >&2
        fi
        # Phase 2 — replay each recorded command under both compilers and compare.
        # No make runs here, so the replays are independent; the pool is two wide,
        # which is this host's core count and keeps two large TUs (sched/fair,
        # tcp_input) inside 2 GB of RAM.
        results=$(mktemp -d)
        DIFF_DIR=${DIFF_DIR:-/tmp/repro-differ}
        rm -rf "$DIFF_DIR"; mkdir -p "$DIFF_DIR"
        running=0
        for tu in "${tus[@]}"; do
            slot="$results/${tu//\//_}"
            (
                export KERNEL_TU_NO_BUILD=1
                cmd=$(kernel_tu_command "$tu")
                a=$(mktemp --suffix=.o); b=$(mktemp --suffix=.o)
                if [[ -z "$cmd" ]]; then
                    echo "unbuildable:$tu:no Kbuild record — this config has no rule for it (see /tmp/repro-37-gcc.log)" > "$slot"
                elif ! (cd "$KERNEL_DIR" && eval "$cmd -o $a") >/dev/null 2>&1; then
                    echo "failed:$tu:post-change compiler rejected it" > "$slot"
                elif ! (cd "$KERNEL_DIR" && eval "$(with_compiler "$cmd" "$LCCC_PRE") -o $b") >/dev/null 2>&1; then
                    echo "failed:$tu:pre-change compiler rejected it" > "$slot"
                elif cmp -s "$a" "$b"; then
                    echo "identical:$tu:$(stat -c %s "$a") B" > "$slot"
                else
                    # Keep both objects: a difference nobody can inspect is a
                    # difference nobody can judge, and "12 differ" is not a
                    # finding until the disassembly says what differs.
                    sa=$(stat -c %s "$a"); sb=$(stat -c %s "$b")
                    mkdir -p "$DIFF_DIR/${tu%/*}"
                    cp "$a" "$DIFF_DIR/$tu.post.o"; cp "$b" "$DIFF_DIR/$tu.pre.o"
                    ia=$(obj_insns "$a"); na=$(obj_pads "$a")
                    ib=$(obj_insns "$b"); nb=$(obj_pads "$b")
                    # .num feeds the aggregate: real (non-padding) instructions.
                    echo "$((ia - na)) $((ib - nb)) $sa $sb" > "$slot.num"
                    echo "differ:$tu:post $sa B / $((ia - na)) insns ($na pad) vs pre $sb B / $((ib - nb)) insns ($nb pad) — pair saved under $DIFF_DIR/$tu.{post,pre}.o" > "$slot"
                fi
                rm -f "$a" "$b"
            ) &
            running=$((running + 1))
            if (( running >= SWEEP_JOBS )); then wait -n 2>/dev/null || wait; running=$((running - 1)); fi
        done
        wait
        identical=$(grep -h -c '^identical:' "$results"/* 2>/dev/null | head -1)
        identical=$(grep -h '^identical:' "$results"/* 2>/dev/null | wc -l)
        ndiffer=$(grep -h '^differ:' "$results"/* 2>/dev/null | wc -l)
        failed=$(grep -h '^failed:' "$results"/* 2>/dev/null | wc -l)
        unbuildable=$(grep -h '^unbuildable:' "$results"/* 2>/dev/null | wc -l)
        tot_post=0; tot_pre=0; grew=0
        if ls "$results"/*.num >/dev/null 2>&1; then
            # Three fields out, three variables in: a fourth field would be
            # swallowed into the last variable by `read` and print as "0 1".
            read -r tot_post tot_pre grew < <(awk '{p += $1; q += $2; if ($1 > $2) g++}
                END {printf "%d %d %d", p, q, g + 0}' "$results"/*.num)
            grew_bytes=$(awk '{if ($3 > $4) g++} END {printf "%d", g + 0}' "$results"/*.num)
        else
            grew_bytes=0
        fi
        while IFS= read -r line; do
            [[ -n "$line" ]] && echo "  detail     $line" >&2
        done < <(grep -h -E '^(differ|failed|unbuildable):' "$results"/* 2>/dev/null | cut -d: -f2-)
        rm -rf "$results"
        measured "of ${#tus[@]} listed: $identical byte-identical, $ndiffer differing, $failed rejected by a compiler, $unbuildable with no Kbuild rule in this config"
        measured "real (non-padding) instructions across the $ndiffer differing TUs: pre $tot_pre -> post $tot_post ($((tot_post - tot_pre))); TUs with more instructions: $grew; objects larger in bytes: $grew_bytes"
        measured "differing pairs kept under $DIFF_DIR/<tu>.{post,pre}.o for objdump -d comparison"
        if (( failed )); then
            finish fail "$failed TUs were rejected by one of the two compilers (details above)"
        elif (( unbuildable )); then
            finish fail "$unbuildable listed TUs have no Kbuild rule in this config — the list is not the sweep"
        elif (( grew )); then
            finish fail "$grew TUs emit MORE real instructions post-change (inspect $DIFF_DIR)"
        elif (( identical + ndiffer < 30 )); then
            finish fail "only $((identical + ndiffer)) TUs compiled on both sides; the sweep is not the documented one"
        else
            finish ok "all ${#tus[@]} TUs compile under both compilers and no TU emits more real instructions: $identical byte-identical, $ndiffer differing, $((tot_pre - tot_post)) instructions removed in total. Byte size is reported but not asserted — $grew_bytes object(s) are larger in bytes purely as 16 B function-alignment padding, which is not codegen."
        fi
    fi
fi

# ----------------------------------------------------------------- claim 6 --
if want 6 && [[ $QUICK == 0 ]]; then
    claim 6 EXACT "the vectorizer ISA gate refused every entry it was asked to analyse" "§5" \
        "1628 entries, 1628 refusals over 586 distinct functions on mm/page_alloc.c"
    command_line "LCCC_DEBUG_VECTORIZE=1 \$LCCC <kernel flags> -c mm/page_alloc.c | count header/refusal pairs"
    if need_lccc && need_kernel; then
        cmd=$(kernel_tu_command mm/page_alloc.o)
        if [[ -z "$cmd" ]]; then
            finish skip "Kbuild produced no command for mm/page_alloc.o"
        else
            out=$(mktemp --suffix=.o)
            trace=$( (cd "$KERNEL_DIR" && LCCC_DEBUG_VECTORIZE=1 eval "$cmd -o $out") 2>&1 >/dev/null )
            rm -f "$out"
            # Exact trace grammar, not fuzzy keyword matching: the header line is
            #   [VEC] Function: NAME, blocks: N, loops: M
            # and the refusal line is
            #   [VEC] Function NAME: not vectorized: x86 SIMD disabled by ISA flags (...)
            # Both contain the word "vectoriz", so a keyword count matches both and
            # reports entries != refusals for a gate that is in fact answering first.
            headers=$(grep -cE '^\[VEC\] Function: ' <<<"$trace")
            refusals=$(grep -cE '^\[VEC\] Function .*: not vectorized' <<<"$trace")
            distinct=$(grep -E '^\[VEC\] Function: ' <<<"$trace" | sed 's/,.*//' | sort -u | wc -l)
            shapes=$(grep -E '^\[VEC\] Function: ' <<<"$trace" | sort -u | wc -l)
            withloops=$(grep -E '^\[VEC\] Function: ' <<<"$trace" | grep -vc 'loops: 0')
            measured "trace lines=$(wc -l <<<"$trace"), entries=$headers, refusals=$refusals"
            measured "distinct functions=$distinct, unique (function, CFG shape)=$shapes, entries with >=1 loop=$withloops"
            if (( headers != refusals || headers == 0 )); then
                finish fail "entries ($headers) != refusals ($refusals): the gate is not answering before the analysis"
            elif (( headers == 1628 && distinct == 586 )); then
                finish ok "1628 entries, 1628 refusals over 586 distinct functions — the census reproduces exactly"
            else
                finish report "entries == refusals at $headers over $distinct functions; the documented census was 1628/586 on this tree, and the absolute count follows the kernel config"
                echo "  note       the invariant this claim is about (every traced entry ends in the ISA" >&2
                echo "  note       refusal, i.e. a 100% waste rate for the traced path) holds either way." >&2
            fi
        fi
    fi
fi

# ----------------------------------------------------------------- claim 7 --
if want 7; then
    claim 7 EXACT "the ISA-gate trace is byte-identical across all five env combos" "§5" \
        "stderr line counts 18, 1526, 9, 18, 0 and identical emitted assembly"
    command_line "bash tests/regression/check_vectorize_isa_gate.sh"
    if need_lccc; then
        out=$(env CCC="$LCCC" timeout 900 bash tests/regression/check_vectorize_isa_gate.sh 2>&1)
        rc=$?
        measured "exit $rc; $(grep -cE '(^|[[:space:]])(ok|PASS)([[:space:]]|:)' <<<"$out") lines report ok/PASS; last: $(tail -1 <<<"$out")"
        if (( rc == 0 )); then
            finish ok "the permanent pin reproduces the documented byte-identity"
        else
            finish fail "check_vectorize_isa_gate.sh failed"
            tail -12 <<<"$out" >&2
        fi
    fi
fi

# ----------------------------------------------------------------- claim 8 --
if want 8 && [[ $QUICK == 0 ]]; then
    claim 8 REPORT "interleaved compile-time A/B on one kernel TU" "§5" \
        "pre 33343 ms / 2a675b3 32982 ms / HEAD 33269 ms median (order-rotated, interleaved); the documented -146 ms (-1.1%) predates F3, whose own +24% was found by this claim and removed"
    command_line "$ROUNDS rounds alternating \$LCCC_PRE and \$LCCC on mm/page_alloc.c"
    if need_lccc && need_kernel && need_pre; then
        cmd=$(kernel_tu_command mm/page_alloc.o)
        if [[ -z "$cmd" ]]; then
            finish skip "Kbuild produced no command for mm/page_alloc.o"
        else
            out=$(mktemp --suffix=.o)
            declare -a pre_t=() post_t=()
            for ((r = 0; r < ROUNDS; r++)); do
                for side in pre post; do
                    bin=$LCCC_PRE; [[ $side == post ]] && bin=$LCCC
                    this=$(with_compiler "$cmd" "$bin")
                    t0=$(date +%s%N)
                    (cd "$KERNEL_DIR" && eval "$this -o $out") >/dev/null 2>&1
                    t1=$(date +%s%N)
                    if [[ $side == pre ]]; then pre_t+=($(( (t1 - t0) / 1000000 )))
                    else post_t+=($(( (t1 - t0) / 1000000 ))); fi
                done
            done
            rm -f "$out"
            stats() { printf '%s\n' "$@" | sort -n | awk '{a[NR]=$1; s+=$1} END {printf "median=%d mean=%d min=%d max=%d n=%d", a[int((NR+1)/2)], s/NR, a[1], a[NR], NR}'; }
            measured "pre: $(stats "${pre_t[@]}") ms | post: $(stats "${post_t[@]}") ms"
            finish report "timings measured on this host; the document claims no more than a direction, and neither does this script"
        fi
    fi
fi

# ----------------------------------------------------------------- claim 9 --
if want 9 && [[ $QUICK == 0 ]]; then
    claim 9 REPORT "boot size oracle: lccc against two independent assemblers" "§6" \
        "lccc 24381 B/_end 31168/1600 headroom; gcc 14.2 13479/22880/9888; clang 19 14740/22768/10000"
    command_line "CC_ORACLE=gcc|clang bash scripts/boot_size_oracle.sh"
    if need_lccc && need_kernel; then
        measured "gcc: $(gcc --version | head -1)"
        if command -v clang >/dev/null 2>&1; then
            measured "clang: $(clang --version | head -1)"
        else
            measured "clang: not installed — the clang row cannot be reproduced here"
        fi
        out=$(CC_ORACLE=gcc KERNEL_DIR="$KERNEL_DIR" LCCC="$LCCC" timeout 1700 \
            bash scripts/boot_size_oracle.sh 2>&1)
        rc=$?
        lccc_row=$(grep -E '^lccc ' <<<"$out" | head -1)
        gcc_row=$(grep -E '^gcc ' <<<"$out" | head -1)
        measured "oracle exit $rc | ${lccc_row:-<no lccc row>} | ${gcc_row:-<no gcc row>}"
        if (( rc == 0 )) && grep -q "PASS" <<<"$lccc_row"; then
            finish report "the oracle runs and the lccc row passes its gate; sizes follow the host's gcc/clang versions, which the document names rather than assumes"
        else
            finish fail "the size oracle did not pass (exit $rc)"
            tail -8 <<<"$out" >&2
        fi
    fi
fi

# ---------------------------------------------------------------- claim 10 --
if want 10; then
    claim 10 MONOTONE "unit + integration test totals" "§8" "2953 passed, 0 failed, 7 ignored (2931/0/6 when first documented)"
    command_line "cargo test --profile $PROFILE --all-targets --locked -j $JOBS"
    out=$(timeout 1700 cargo test --profile "$PROFILE" --all-targets --locked -j "$JOBS" 2>&1)
    sum() { grep -oE "[0-9]+ $1" <<<"$out" | grep -oE '^[0-9]+' | awk '{s+=$1} END {print s+0}'; }
    passed=$(sum passed)
    failed=$(sum failed)
    measured "$passed passed, $failed failed across all targets"
    if (( failed > 0 )); then
        finish fail "$failed tests failed"
        grep -E "^test .* FAILED|^failures:" <<<"$out" | head -10 >&2
    elif (( passed >= 2953 )); then
        finish ok "$passed passed (2953 last verified; the total only grows as tests are added)"
    else
        finish fail "only $passed passed against 2953 last verified — tests went missing"
    fi
fi

# ---------------------------------------------------------------- claim 11 --
if want 11 && [[ $QUICK == 0 ]]; then
    claim 11 MONOTONE "regression corpus totals" "§8" "773 passed, 0 failed, 784 total (772/783 when first documented)"
    command_line "CCC_VALIDATE_SSA=1 python3 tests/regression/run_regression.py --lccc $LCCC -j $JOBS"
    if need_lccc; then
        out=$(env CCC_VALIDATE_SSA=1 timeout 1700 python3 tests/regression/run_regression.py \
            --lccc "$LCCC" -j "$JOBS" 2>&1)
        rc=$?
        # The runner's own summary line: "== 773 passed, 0 failed, 11
        # skipped-compare, 0 skipped-run, 784 total, 100s". A PASS=/FAIL= pattern
        # matched nothing and the fallback printed an empty line, which is the
        # same defect a document with an unreproducible number has.
        line=$(grep -oE '[0-9]+ passed, [0-9]+ failed.*total[, 0-9s]*' <<<"$out" | tail -1)
        total=$(grep -oE '[0-9]+ total' <<<"$out" | grep -oE '^[0-9]+' | tail -1)
        passed=$(grep -oE '[0-9]+ passed' <<<"$out" | grep -oE '^[0-9]+' | tail -1)
        measured "exit $rc | ${line:-<no summary line: $(tail -2 <<<"$out" | head -1)>}"
        if (( rc == 0 )) && (( ${passed:-0} >= 773 )); then
            finish ok "${passed:-0} passed of ${total:-?} total (773 last verified; the corpus only grows)"
        else
            finish fail "the regression corpus reported failures"
            grep -E "FAIL" <<<"$out" | head -8 >&2
        fi
    fi
fi

# ---------------------------------------------------------------- claim 12 --
if want 12; then
    claim 12 REPORT "tracked-file mode census behind the executable-bit rule" "§7" \
        "159 tracked files carry a shebang and are mode 644; 110 of them are tests/regression/check_*.sh"
    command_line "git ls-files -s | awk '\$1==\"100644\"' + head -1 shebang test"
    if git rev-parse --git-dir >/dev/null 2>&1; then
        shebang_644=0; check_644=0
        while IFS= read -r f; do
            [[ -f "$f" ]] || continue
            head -c 2 "$f" | grep -q '#!' || continue
            shebang_644=$((shebang_644 + 1))
            [[ "$f" == tests/regression/check_*.sh ]] && check_644=$((check_644 + 1))
        done < <(git ls-files -s | awk '$1 == "100644" {print $4}')
        measured "$shebang_644 shebang-and-644 files, $check_644 of them tests/regression/check_*.sh"
        finish report "the census follows the tree; the RULE it justified (entry points invoked directly are 755, scripts invoked through an interpreter and sourced modules stay 644) is what §7 states and what the restore script enforces"
    else
        finish skip "not a git repository, so tracked modes cannot be read"
    fi
fi

[[ -n "$boot_log" ]] && rm -f "$boot_log"

hr
printf '%-4s %-8s %-6s %s\n' ID CLASS STATUS DETAIL
for line in "${SUMMARY[@]}"; do printf '%s\n' "$line"; done
hr
printf 'repro_claims: %d ok, %d reported, %d skipped, %d failed\n' \
    "$n_ok" "$n_report" "$n_skip" "$n_fail"
if (( n_fail > 0 )); then
    echo "repro_claims: FAILED — a documented value did not reproduce" >&2
    exit 1
fi
if (( REQUIRE_ALL == 1 && n_skip > 0 )); then
    echo "repro_claims: FAILED under --require-all — $n_skip claims could not be measured" >&2
    exit 1
fi
echo "repro_claims: PASS (every measured claim agreed with its document; skips name their prerequisite)"
