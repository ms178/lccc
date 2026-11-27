#!/usr/bin/env bash
# C `volatile` access semantics (C11 5.1.2.3): every volatile load and store is
# an observable side effect.  Guards the whole pipeline: IR flags, lowering,
# mem2reg flag propagation, and the optimizer gates (store-load forwarding,
# load CSE/forwarding, GVN, DCE, LICM, loop memory promotion).
#
#  1. store-then-load of a volatile global must re-read memory (no forwarding)
#  2. two volatile loads must not be CSE'd (loop executes N reads)
#  3. a dead-result volatile load must survive DCE
#  4. *p through a pointer-to-volatile parameter must load
#  5. volatile locals keep their RMW shape (no mem2reg promotion)
#  6. a volatile or _Atomic object read-modify-written in a loop keeps its
#     per-iteration access: no pass may collapse N read+write pairs into one
#     accumulated update.  This is the PR #716 review's "gate the policy where
#     volatility is still visible" item: the peephole RMW fold (one read, one
#     write, no other access in between) is sound for a single-threaded
#     observer, but nothing downstream may multiply that into a whole loop.
#     The check is SHAPE + ORACLE, and says so: the loop must still contain a
#     reference to the object and a back edge.
#  6b. an _Atomic object is accessed where the program accesses it, however
#     it is named: through a typedef (`atomic_bool`), by the qualifier on
#     the object, as a member, behind a pointer, behind a cast.  For
#     `_Atomic` item 6 used to hold by accident: the types carry no
#     atomic-ness, the accumulator was plain memory, and it stayed in its
#     loop only because nothing could move a global's store out of a
#     top-tested loop.  Loop rotation can, and did; and without it `while
#     (!flag) ;` already read its flag once and span on a register.  The
#     parser now records `volatile` wherever it records `_Atomic`
#     (`ParsedDeclAttrs::set_atomic`), and the cells below hold each form to
#     it, with the same loops over plain objects as the control: there the
#     load does leave the loop.  (That the qualifier rules do not take that
#     `volatile` for a written one is check_qualifier_conversions.py's.)
#     The cast cells hold one more form of plain `volatile` to the same
#     rule: a cast to a pointer to a typedef name that is volatile, the
#     register idiom `*(vu32 *)ADDR`, whose object used to be read once.
#  7. a pointer object that is itself volatile -- `T *volatile p`, a function
#     pointer `int (*volatile f)(int)`, `volatile fn_t f` with `fn_t` a typedef
#     of a pointer -- is stored and loaded where the program says.  The slot
#     of such a local was an ordinary one (only a volatile POINTEE was looked
#     at when the slot was made), so its initializer was forwarded to its
#     first read, and a function pointer never reached memory at all: with
#     `static int h(int a) { return a + 1; }`, `int (*volatile f)(int) = h;
#     return f(1);` was `movl $2, %eax`, where GCC 16.2 stores `h`, loads it
#     back and jumps through what it loaded.
#     Each cell must write the slot and read it back, the calls must go
#     through what was read, and the same code with a plain pointer is the
#     control: there no slot is left.
set -euo pipefail

CCC=${CCC:-./target/fastbuild/lccc}
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT

cat >"$td/vol.c" <<'EOF'
volatile int counter = 5;
volatile int sink;
int read_after_store(void) { counter = 7; return counter; }   /* want: load */
int reads_in_loop(int n) { int t = 0; for (int i = 0; i < n; i++) t += counter; return t; }
int dead_read(void) { counter; sink = 1; return 0; }          /* load must survive */
int deref_param(volatile int *p) { return *p; }
int volatile_local(void) { volatile int loc = 3; loc = loc + 1; return loc; }
_Atomic int atomic_sum = 0;
volatile int vol_sum = 0;
void atomic_acc(int n) { for (int i = 0; i < n; i++) atomic_sum += 3; }
void vol_acc(int n) { for (int i = 0; i < n; i++) vol_sum += 3; }
EOF

cat >"$td/acc.c" <<'EOF'
#include <stdio.h>
_Atomic int atomic_sum = 0;
volatile int vol_sum = 0;
void atomic_acc(int n) { for (int i = 0; i < n; i++) atomic_sum += 3; }
void vol_acc(int n) { for (int i = 0; i < n; i++) vol_sum += 3; }
int main(void) { atomic_acc(1000); vol_acc(1000); printf("%d %d\n", (int)atomic_sum, vol_sum); return 0; }
EOF

rc=0
# Accept register-indirect and direct RIP-relative memory operands, plus the
# signed-widening form selected for an i32 value on x86-64. RA-01 deliberately
# turns ordinary PIE globals into `symbol(%rip)` accesses.
load_pat='mov(l|slq) +[^,]*\(%[re]?[a-z0-9]+'
# A volatile READ is an observable access whatever instruction performs it, so
# the loop check cannot be spelled "there is a mov": `t += counter` selects the
# tighter fused read-modify-write `addl counter(%rip), %esi` (one instruction
# where GCC emits `movl counter(%rip), %ecx` + `addl %ecx, %edx`), and a
# mov-only pattern reported that correct code as "volatile load eliminated".
# What must not happen is the access disappearing from the loop, so the pattern
# accepts any ALU instruction with a trailing memory operand — the memory
# operand is last in AT&T, which is what keeps a *store* (`movl %eax,
# counter(%rip)`) out of the match.
read_pat='(mov[lbwlq]?|movs(bl|bq|wl|wq|lq)|add[lbwlq]?|sub[lbwlq]?|and[lbwlq]?|or[lbwlq]?|xor[lbwlq]?|cmp[lbwlq]?) +[^,]*\(%[re]?[a-z0-9]+'
check() { # check <fn> <grep-pattern> <description>
    local fn=$1 pat=$2 desc=$3
    local body
    body=$(awk -v f="$fn" '$0==f":"{ins=1} ins{print} /^\.size/{if(ins)exit}' "$td/vol.s")
    if [ -z "$body" ]; then echo "FAIL: $fn not found"; rc=1; return; fi
    if echo "$body" | grep -Ec "$pat" >/dev/null; then
        echo "ok: $desc"
    else
        echo "FAIL: $desc (pattern '$pat' not in $fn)"; rc=1
    fi
}

for lvl in O0 O1 O2 Os; do
    echo "== $lvl"
    "$CCC" -$lvl -S "$td/vol.c" -o "$td/vol.s" || { echo "FAIL: compile at -$lvl"; rc=1; continue; }
    # 1. a real load of counter between the store and the return
    body=$(awk '/^read_after_store:/,/^\.size/' "$td/vol.s")
    # `grep -c` exits 1 on a zero count, which under `set -e` would abort the
    # whole script instead of reporting the one failed check.
    loads=$(echo "$body" | grep -Ec "$load_pat" || true)
    case $loads in ''|*[!0-9]*) echo "FAIL: load count unreadable ('$loads')"; rc=1; loads=0 ;; esac
    if [ "$loads" -ge 1 ]; then echo "ok: store-then-load reloads memory"; else echo "FAIL: volatile load forwarded/eliminated"; rc=1; fi
    # 2. the volatile read must happen INSIDE the loop, once per iteration.
    #
    # The property is position, not mnemonic. lccc folds the read into
    # `addl counter(%rip), %esi`, which is ONE instruction instead of a
    # `movl` plus an `addl` -- better code that a regex demanding a separate
    # load reports as a failure. So slice out the loop (from the backward
    # branch's target label to the branch itself) and require the volatile
    # object to be referenced in it. Hoisting it out would leave the loop
    # with no reference at all.
    fn_body=$(awk '/^reads_in_loop:/,/^\.size/' "$td/vol.s")
    tail_ln=$(echo "$fn_body" | grep -nE '^[[:space:]]*j[a-z]+ +\.[A-Za-z]' | tail -1 | cut -d: -f1)
    if [ -z "$tail_ln" ]; then
        echo "FAIL: reads_in_loop has no backward branch (loop unrolled?)"; rc=1
    else
        tail_lbl=$(echo "$fn_body" | sed -n "${tail_ln}p" | grep -oE '\.[A-Za-z][A-Za-z0-9_]*' | tail -1)
        head_ln=$(echo "$fn_body" | grep -n "^${tail_lbl}:" | sed -n '1,1p' | cut -d: -f1)
        loop=$(echo "$fn_body" | sed -n "${head_ln},${tail_ln}p")
        if echo "$loop" | grep -c 'counter' >/dev/null; then
            echo "ok: loop keeps the volatile read in the body"
        else
            echo "FAIL: volatile read hoisted out of the loop body"
            echo "      loop was: $loop"
            rc=1
        fi
        # NEGATIVE CONTROL: the loop must still be a loop. If the read
        # vanished because the whole loop was optimised away, the check
        # above would pass for the wrong reason.
        if ! echo "$loop" | grep -cE '^[[:space:]]*j[a-z]+ +\.[A-Za-z]' >/dev/null; then
            echo "FAIL: no back edge in the extracted loop region"; rc=1
        fi
    fi
    # 3. dead read survives
    check dead_read "$load_pat" "dead-result volatile load survives DCE"
    # 4. deref through pointer param loads
    check deref_param "$load_pat" "*volatile-ptr param loads"
    # 5. volatile local: store;load;store sequence
    body=$(awk '/^volatile_local:/,/^\.size/' "$td/vol.s")
    n=$(echo "$body" | grep -Ec 'mov[a-z]* +[^#]*\(%(rsp|rbp|esp|ebp)' || true)
    case $n in ''|*[!0-9]*) echo "FAIL: mem access count unreadable ('$n')"; rc=1; n=0 ;; esac
    if [ "$n" -ge 3 ]; then echo "ok: volatile local keeps RMW"; else echo "FAIL: volatile local promoted (mem access count $n < 3)"; rc=1; fi
done

# 6. the accumulated-RMW loop, for the two spellings whose accesses are
# observable.  `gcc -O0` is the oracle for the VALUE; the pattern is for the
# SHAPE, because access COUNT is not observable from stdout in a
# single-threaded program -- an interpreter of the asm is what would count it,
# and the honest static claim is "the object is still accessed every iteration
# and the loop still loops".
cat >"$td/acc_oracle.c" <<'EOF'
#include <stdio.h>
_Atomic int atomic_sum = 0;
volatile int vol_sum = 0;
void atomic_acc(int n) { for (int i = 0; i < n; i++) atomic_sum += 3; }
void vol_acc(int n) { for (int i = 0; i < n; i++) vol_sum += 3; }
int main(void) { atomic_acc(1000); vol_acc(1000); printf("%d %d\n", (int)atomic_sum, vol_sum); return 0; }
EOF
if command -v gcc >/dev/null 2>&1; then
    gcc -O0 "$td/acc_oracle.c" -o "$td/acc.ref" && ref=$("$td/acc.ref")
    "$CCC" -O2 "$td/acc_oracle.c" -o "$td/acc.got" && got=$("$td/acc.got")
    if [ "$ref" = "$got" ]; then echo "ok: _Atomic/volatile accumulate matches gcc -O0 ($got)"
    else echo "FAIL: _Atomic/volatile accumulate lccc='$got' oracle='$ref'"; rc=1; fi
fi
"$CCC" -O2 -S "$td/acc.c" -o "$td/acc.s" || { echo "FAIL: compile acc.c"; rc=1; }
for fn in atomic_acc vol_acc; do
    obj=$([ "$fn" = atomic_acc ] && echo atomic_sum || echo vol_sum)
    body=$(awk -v f="$fn" '$0==f":"{ins=1} ins{print} /^\.size/{if(ins)exit}' "$td/acc.s")
    if [ -z "$body" ]; then echo "FAIL: $fn not found"; rc=1; continue; fi
    # the loop region: from the backward branch's target to the branch itself
    tail_ln=$(echo "$body" | grep -nE '^[[:space:]]*j[a-z]+ +\.[A-Za-z]' | tail -1 | cut -d: -f1)
    tail_lbl=$(echo "$body" | sed -n "${tail_ln}p" | grep -oE '\.[A-Za-z][A-Za-z0-9_]*' | tail -1)
    head_ln=$(echo "$body" | grep -n "^${tail_lbl}:" | sed -n '1,1p' | cut -d: -f1)
    loop=$(echo "$body" | sed -n "${head_ln},${tail_ln}p")
    # The volatile object is named by the access itself. The atomic one is
    # updated by a locked instruction through a register the loop has no
    # reason to reload (`lock addl $3, (%rsi)`, the `leaq` above the loop):
    # what must stay inside is that instruction.
    if [ "$fn" = atomic_acc ]; then
        n_ref=$(echo "$loop" | grep -cE '^[[:space:]]*lock ' || true)
    else
        n_ref=$(echo "$loop" | grep -c "$obj" || true)
    fi
    case $n_ref in ''|*[!0-9]*) n_ref=0 ;; esac
    # Here-string, not `echo | grep -q`: under `set -o pipefail` the early-exiting
# consumer SIGPIPEs the producer and the pipeline reports failure even when the
# pattern matched (the pipefail-sigpipe gate exists for exactly this).
if [ "$n_ref" -ge 1 ] && grep -qE '^[[:space:]]*j[a-z]+ +\.' <<<"$loop"; then
        echo "ok: $fn keeps its per-iteration $obj access inside a real loop"
    else
        echo "FAIL: $fn collapsed to a closed form (refs=$n_ref)"; echo "      loop was: $loop"; rc=1
    fi
done

# 6b. Atomic objects, by every way of naming one.
cat >"$td/atomic.c" <<'EOF'
#include <stdatomic.h>
struct ctl { long pad; _Atomic int stop; long work; };
struct plain_ctl { long pad; int stop; long work; };
atomic_bool a_typedef;
_Atomic int a_named;
int p_named;
atomic_long a_hits;
/* A waiter: the flag is all the loop reads. */
unsigned long spin_typedef(void) { unsigned long n = 0; while (!a_typedef) n++; return n; }
unsigned long spin_named(void) { unsigned long n = 0; while (!a_named) n++; return n; }
unsigned long spin_member(struct ctl *c) { unsigned long n = 0; while (!c->stop) n++; return n; }
unsigned long spin_pointee(_Atomic int *f) { unsigned long n = 0; while (!*f) n++; return n; }
unsigned long spin_trailing(int _Atomic *f) { unsigned long n = 0; while (!*f) n++; return n; }
unsigned long spin_specifier(_Atomic(int) *f) { unsigned long n = 0; while (!*f) n++; return n; }
/* The same loops over plain objects: one load, in front. */
unsigned long spin_plain_named(void) { unsigned long n = 0; while (!p_named) n++; return n; }
unsigned long spin_plain_member(struct plain_ctl *c) { unsigned long n = 0; while (!c->stop) n++; return n; }
/* An accumulator: one read and one write a trip. */
void acc_typedef(int n) { for (int i = 0; i < n; i++) a_hits += 3; }
void acc_member(struct ctl *c, int n) { for (int i = 0; i < n; i++) c->stop += 3; }
void acc_pointee(_Atomic long *p, int n) { for (int i = 0; i < n; i++) *p += 3; }
/* The controls are bottom-tested, so that what leaves them does not wait
 * for a decision of loop rotation. */
void acc_plain_member(struct plain_ctl *c, int n) { int i = 0; do c->stop += 3; while (++i < n); }
/* The neighbour of an atomic member is an ordinary member. */
long neighbour(struct ctl *c, int n) { long s = 0; int i = 0; do s += c->work; while (++i < n); return s; }
/* Behind a cast the object is named by the cast's type alone: `_Atomic`
 * written there, or a typedef name that is atomic or volatile. */
typedef volatile unsigned v_reg;
typedef volatile struct shared { int go; } shared;
unsigned long spin_cast_atomic(void *p) { unsigned long n = 0; while (!*(_Atomic int *)p) n++; return n; }
unsigned long spin_cast_specifier(void *p) { unsigned long n = 0; while (!*(_Atomic(int) *)p) n++; return n; }
unsigned long spin_cast_typedef(void *p) { unsigned long n = 0; while (!*(atomic_int *)p) n++; return n; }
unsigned long spin_cast_index(void *p, long i) { unsigned long n = 0; while (!((atomic_int *)p)[i]) n++; return n; }
unsigned long spin_cast_volatile_typedef(void *p) { unsigned long n = 0; while (!(*(v_reg *)p & 1u)) n++; return n; }
unsigned long spin_cast_volatile_record(void *p) { unsigned long n = 0; while (!((shared *)p)->go) n++; return n; }
/* Not these: a plain pointee, a struct tag that only shares its spelling
 * with a volatile typedef, and an atomic *pointer* to a plain object. */
unsigned long spin_cast_plain(void *p) { unsigned long n = 0; while (!*(int *)p) n++; return n; }
unsigned long spin_cast_tag(void *p) { unsigned long n = 0; while (!((struct shared *)p)->go) n++; return n; }
unsigned long spin_cast_atomic_pointer(void *p) { unsigned long n = 0; while (!*(_Atomic(int *))p) n++; return n; }
EOF
"$CCC" -O2 -S "$td/atomic.c" -o "$td/atomic.s" || { echo "FAIL: compile atomic.c"; rc=1; }
# The instructions of <fn> that lie on a cycle of its control flow and name
# memory: a symbol, or a register-indirect operand (frame slots excepted).
memory_in_loop() {
    python3 - "$td/atomic.s" "$1" <<'PY'
import re
import sys

path, fn = sys.argv[1], sys.argv[2]
lines, inside = [], False
for raw in open(path, encoding="utf-8", errors="replace"):
    text = raw.rstrip("\n")
    if text.startswith(fn + ":"):
        inside = True
        continue
    if inside and re.match(r"\s*\.size\s", text):
        break
    if inside and text.strip() and not re.match(r"\s*(#|\.[a-z0-9_]+\b(?!:))", text):
        lines.append(text.strip())
blocks, names = [[]], {}
for text in lines:
    label = re.match(r"(\.L[A-Za-z0-9_.$]*):", text)
    if label:
        if blocks[-1]:
            blocks.append([])
        names[label.group(1)] = len(blocks) - 1
        continue
    blocks[-1].append(text)
    if re.match(r"(j[a-z]+|ret[a-z]*|ud2)\b", text):
        blocks.append([])
succ = []
for i, block in enumerate(blocks):
    out = []
    last = block[-1] if block else ""
    jump = re.match(r"(j[a-z]+)\s+(\S+)", last)
    if jump and jump.group(2) in names:
        out.append(names[jump.group(2)])
    if not re.match(r"(jmp|ret[a-z]*|ud2)\b", last) and i + 1 < len(blocks):
        out.append(i + 1)
    succ.append(out)

def reaches(start):
    seen, work = set(), list(succ[start])
    while work:
        b = work.pop()
        if b not in seen:
            seen.add(b)
            work.extend(succ[b])
    return seen

in_loop = [i for i in range(len(blocks)) if i in reaches(i)]
if not in_loop:
    print("no-loop")
    sys.exit(0)
memory = re.compile(r"\((?!%rsp|%rbp)%r[a-z0-9]+|[A-Za-z_][A-Za-z0-9_]*\(%rip\)")
print(sum(1 for i in in_loop for text in blocks[i]
          if memory.search(text) and not text.startswith("lea")))
PY
}
for probe in spin_typedef:1 spin_named:1 spin_member:1 spin_pointee:1 \
             spin_trailing:1 spin_specifier:1 \
             spin_plain_named:0 spin_plain_member:0 neighbour:0 \
             spin_cast_atomic:1 spin_cast_specifier:1 spin_cast_typedef:1 \
             spin_cast_index:1 spin_cast_volatile_typedef:1 \
             spin_cast_volatile_record:1 \
             spin_cast_plain:0 spin_cast_tag:0 spin_cast_atomic_pointer:0; do
    IFS=: read -r fn want <<<"$probe"
    got=$(memory_in_loop "$fn")
    if [[ $got == "$want" ]]; then
        echo "ok: $fn reads memory $got time(s) a trip"
    else
        echo "FAIL: $fn reads memory $got time(s) a trip, expected $want"; rc=1
    fi
done
# A read and a write a trip: two instructions, or one that does both.
for fn in acc_typedef acc_member acc_pointee; do
    got=$(memory_in_loop "$fn")
    if [[ $got == 1 || $got == 2 ]]; then
        echo "ok: $fn keeps its read and its write in the loop ($got instruction(s))"
    else
        echo "FAIL: $fn has $got memory instruction(s) in its loop, expected 1 or 2"; rc=1
    fi
done
# The control: a plain accumulator behind a pointer is what the memory
# promoter is for, and leaves its loop.
got=$(memory_in_loop acc_plain_member)
if [[ $got == 0 ]]; then
    echo "ok: acc_plain_member keeps its accumulator in a register"
else
    echo "FAIL: acc_plain_member has $got memory instruction(s) in its loop, expected 0"; rc=1
fi

# 7. An extern declaration still describes a volatile object or pointee.
# Both file- and block-scope declarations used to lose their qualifier before
# IR construction, so even discarded reads disappeared at -O2. The symbol
# references count accesses in these simple straight-line functions; the
# pointer-target cases additionally count the two indirect loads.
cat >"$td/extern.c" <<'EOF'
extern volatile int g_flag;
extern int *volatile g_ptr;
extern volatile int *g_target;
extern volatile int g_arr[4];
struct regs { int x; };
extern volatile struct regs g_regs;
extern _Thread_local volatile int g_tls;
extern int plain;
int global_sum(void) { return g_flag + g_flag; }
void global_discard(void) { g_flag; g_flag; }
void global_stores(void) { g_flag = 1; g_flag = 2; }
void global_ptr(void) { g_ptr; g_ptr; }
void global_target(void) { *g_target; *g_target; }
void global_array(void) { g_arr[0]; g_arr[0]; }
void global_member(void) { g_regs.x; g_regs.x; }
void global_tls(void) { g_tls; g_tls; }
void block_discard(void) { extern volatile int b_flag; b_flag; b_flag; }
void block_ptr(void) { extern int *volatile b_ptr; b_ptr; b_ptr; }
void block_target(void) { extern volatile int *b_target; *b_target; *b_target; }
void block_tls(void) { extern _Thread_local volatile int b_tls; b_tls; b_tls; }
void plain_discard(void) { plain; plain; }
EOF
"$CCC" -O2 -S "$td/extern.c" -o "$td/extern.s" || { echo "FAIL: compile extern.c"; exit 1; }
for probe in global_sum:g_flag:2 global_discard:g_flag:2 global_stores:g_flag:2 \
             global_ptr:g_ptr:2 global_array:g_arr:2 global_member:g_regs:2 \
             block_discard:b_flag:2 block_ptr:b_ptr:2 plain_discard:plain:0; do
    IFS=: read -r fn symbol want <<<"$probe"
    body=$(awk -v f="$fn" '$0==f":"{ins=1} ins{print} ins && /^[[:space:]]*\.size/{exit}' "$td/extern.s")
    n=$(grep -cE "(^|[^[:alnum:]_])${symbol}\\(%rip\\)" <<<"$body" || true)
    if [[ -n $body && $n == "$want" ]]; then
        echo "ok: $fn keeps $want observable accesses"
    else
        echo "FAIL: $fn has $n references to $symbol, expected $want"; rc=1
    fi
done
for fn in global_target block_target; do
    body=$(awk -v f="$fn" '$0==f":"{ins=1} ins{print} ins && /^[[:space:]]*\.size/{exit}' "$td/extern.s")
    n=$(grep -cE '^[[:space:]]*mov[a-z]*[[:space:]]+\(%r[a-z0-9]+\),' <<<"$body" || true)
    if [[ -n $body && $n == 2 ]]; then
        echo "ok: $fn preserves both volatile pointee loads"
    else
        echo "FAIL: $fn has $n indirect loads, expected 2"; rc=1
    fi
done
for fn in global_tls block_tls; do
    body=$(awk -v f="$fn" '$0==f":"{ins=1} ins{print} ins && /^[[:space:]]*\.size/{exit}' "$td/extern.s")
    n=$(grep -cE '%fs:\(' <<<"$body" || true)
    if [[ -n $body && $n == 2 ]]; then
        echo "ok: $fn preserves both volatile TLS loads"
    else
        echo "FAIL: $fn has $n TLS loads, expected 2"; rc=1
    fi
done

# The same extern metadata carries the pointee's constness for `_Generic`.
# Both controls are unevaluated; they need no definitions of the externs.
cat >"$td/extern_generic.c" <<'EOF'
extern const int *g_const;
int global_type(void) { return _Generic(g_const, const int *: 1, int *: 0, default: 2); }
int block_type(void) { extern const int *b_const; return _Generic(b_const, const int *: 1, int *: 0, default: 2); }
int main(void) { return global_type() != 1 || block_type() != 1; }
EOF
if "$CCC" -O2 "$td/extern_generic.c" -o "$td/extern_generic" && "$td/extern_generic"; then
    echo "ok: extern pointee constness selects the correct _Generic arm"
else
    echo "FAIL: extern pointee constness was lost"; rc=1
fi

if ! python3 "$(dirname "$0")/check_typeof_scalar_volatile.py" --compiler "$CCC"; then
    rc=1
fi

i686_cc=${I686_CCC:-"$(dirname "$CCC")/lccc-i686"}
if [[ -x "$i686_cc" ]]; then
    if ! python3 "$(dirname "$0")/check_i686_volatile_access.py" --compiler "$i686_cc"; then
        rc=1
    fi
else
    echo "SKIP: i686 volatile codegen ($i686_cc not built)"
fi

# 7. volatile pointer objects.
cat >"$td/ptr.c" <<'EOF'
static int h(int a) { return a + 1; }
typedef int (*fn_t)(int);
int ptr_init(int *v) { int *volatile p = v; return *p; }
int ptr_assign(int *v) { int *volatile p; p = v; return *p; }
void *void_ptr_assign(void *v) { void *volatile p; p = v; return p; }
int fn_init(void) { int (*volatile f)(int) = h; return f(1); }
int fn_typedef(fn_t v) { volatile fn_t f = v; return f(2); }
int fn_assign(fn_t v) { fn_t volatile f; f = v; return f(3); }
int plain_ptr(int *v) { int *p = v; return *p; }
int plain_fn(void) { int (*f)(int) = h; return f(1); }
EOF
frame_store='mov[a-z]* +[^,(]+, *-?[0-9]*\(%(rsp|rbp)\)'
frame_load='(mov[a-z]* +-?[0-9]*\(%(rsp|rbp)\), *%|(call|jmp)q? +\*-?[0-9]*\(%(rsp|rbp)\))'
for lvl in O1 O2 O3 Os; do
    echo "== volatile pointer objects, $lvl"
    "$CCC" -$lvl -S "$td/ptr.c" -o "$td/ptr.s" || { echo "FAIL: compile ptr.c at -$lvl"; rc=1; continue; }
    for fn in ptr_init ptr_assign void_ptr_assign fn_init fn_typedef fn_assign; do
        body=$(awk -v f="$fn" '$0==f":"{ins=1} ins{print} /^\.size/{if(ins)exit}' "$td/ptr.s")
        stores=$(echo "$body" | grep -Ec "$frame_store" || true)
        loads=$(echo "$body" | grep -Ec "$frame_load" || true)
        if [ "$stores" -ge 1 ] && [ "$loads" -ge 1 ]; then
            echo "ok: $fn writes its volatile pointer and reads it back"
        else
            echo "FAIL: $fn: $stores store(s) and $loads load(s) of the volatile pointer's slot"
            echo "$body" | sed 's/^/      /'
            rc=1
        fi
        case $fn in fn_*)
            if grep -Eq '(call|jmp)q? +\*' <<<"$body"; then
                echo "ok: $fn calls through the pointer it read"
            else
                echo "FAIL: $fn does not call through its volatile pointer"
                echo "$body" | sed 's/^/      /'
                rc=1
            fi ;;
        esac
    done
    # The control: the same code with a plain pointer leaves no slot, so the
    # accesses counted above are the volatile object's.
    for fn in plain_ptr plain_fn; do
        body=$(awk -v f="$fn" '$0==f":"{ins=1} ins{print} /^\.size/{if(ins)exit}' "$td/ptr.s")
        if grep -Eq "$frame_store|$frame_load" <<<"$body"; then
            echo "FAIL: $fn (plain pointer, the control) touches the frame at -$lvl"
            echo "$body" | sed 's/^/      /'
            rc=1
        else
            echo "ok: $fn, the control, keeps its pointer out of memory"
        fi
    done
done

riscv_cc=${CCC_RISCV:-"$(dirname "$CCC")/lccc-riscv"}
if [[ -x "$riscv_cc" ]]; then
    if ! python3 "$(dirname "$0")/check_riscv_volatile_access.py" --compiler "$riscv_cc"; then
        rc=1
    fi
else
    echo "SKIP: RISC-V volatile codegen ($riscv_cc not built)"
fi

exit $rc
