#!/usr/bin/env python3
"""Structural ratchet for the x86-64 vector store-source contract.

Three separate records describe "this vector value is in a register": a
block-local claim (`vec_live_regs`), a last-store peephole (`*_last_store_*`),
and a deferred store (`pending_vec_store`).  A `VecStore*` arm has to answer two
questions from them, and getting either wrong is a *silent* miscompile -- the
destination receives a register's previous contents:

1. **Which register holds the value** (`vec_store_source_256` / `_128`), and
2. **May the staged register be read instead of reloading the slot**
   (`CodegenState::vec_holds_staging_reg`).

Both were once answered per call site, from whichever record that site happened
to consult.  The laws they now obey are only enforced by code review, so this
gate makes them mechanical.  It is a *static* scan of `src/`: no compiler, no
fixtures, no execution -- which is why it is registered in the pre-build
section and mirrors the hosted `backend-contracts` shard step.

What is pinned, and why each rule can fail independently:

  A. `vec_store_source_*` never returns an unproven register.  Every `return`
     before the final staging default must sit behind a proof: an allocator
     home, a WIDTH-FILTERED claim (`vec_live_ymm`/`vec_live_xmm`), a
     width-matched `pending_vec_store`, or a `*_last_store_reg_name` match on
     the RESERVED SCRATCH PAIR (`ymm0`/`ymm1`/`xmm0`/`xmm1`).  A record naming
     an allocator bank does not license a store that reads the hard-coded
     staging register, so it must not appear in that match.
  B. `vec_holds_staging_reg` compares the REGISTER NAME, not just the value.
     `last_store_val` alone survives the eviction of an allocator bank, and a
     narrow deferred value must not license a wide read.
  C. `vec_claim_live_reg` refuses banks 0 and 1.  The reserved pair is
     overwritten by any 1XX/2XX emitter, so a claim there would outlive its own
     clobber -- and the scratch invalidation intentionally does not clear
     claims, which is what makes rule D sound.
  D. A store to MEMORY retires only the scratch peephole.  `vec_mem_operand`
     and `emit_vec_store_addr` write GPR addressing scratch and never a vector
     bank, so a memory store cannot invalidate an allocator claim.  Stated
     exactly: for every `emit_vec_store_addr(`, the NEAREST preceding
     invalidation must be the scratch one -- no arbitrary look-back window, so
     a wide wipe moved to any distance from the store is still caught.
  E. `vec_arg_mem` consults the register records BEFORE slot validity.  A
     `SlotAddr::Direct` says "a slot exists", never "the slot was written".
  F. A claim is only ever READ through a width-filtered accessor
     (`vec_live_xmm` / `vec_live_ymm` / `vec_live_zmm`), never from the map.
     `vec_live_regs` is width-polyglot: one map, three spellings, recording
     the width the producer wrote. A reader with a fixed width that takes the
     map raw splices the wrong spelling into its operand —
     `vmovdqa %ymm3, %xmm0`, `vbroadcastss %xmm3, %ymm0`,
     `vmovdqa64 %xmm3, %zmm0` are all illegal mixed-width encodings that the
     assembler rejects, so a valid program fails to build. Only readers with
     NO fixed width may look at the map: a bank eviction, a computed-width
     move, a reservation test.

Every rule is proven non-vacuous by `--self-test`, which mutates a synthetic
fixture of each shape and requires the corresponding rule to report it: a
checker that cannot fail is decoration.
"""
from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
INTRINSICS = ROOT / "src" / "backend" / "x86" / "codegen" / "intrinsics.rs"
STATE = ROOT / "src" / "backend" / "state.rs"

# The reserved scratch pair.  A `*_last_store_*` record naming one of these IS
# the whole truth, because the pair can never carry a claim (rule C).
SCRATCH_NAMES = {"ymm0", "ymm1", "xmm0", "xmm1"}

# Proofs a return may stand behind, in `vec_store_source_*`.
PROOF = re.compile(
    r"reg_assignments\.get"                      # allocator home
    r"|vec_live_(ymm|xmm)\("                     # width-filtered claim
    r"|pending_vec_store"                        # deferred store
    r"|_last_store_reg_name"                     # scratch-pair peephole
)


def _blocks(text: str, signature: str) -> list[tuple[int, str]]:
    """Every `fn <signature>` body, with its 1-based start line.

    A brace-matching scan over comment- and string-stripped text: function
    bodies here contain nested braces in closures and `format!` payloads, so a
    line-oriented split would truncate them.
    """
    stripped = _strip(text)
    out: list[tuple[int, str]] = []
    for match in re.finditer(r"\bfn\s+" + re.escape(signature) + r"\b", stripped):
        start = stripped.find("{", match.end())
        if start < 0:
            raise SystemExit(f"check_vec_store_contracts: {signature} has no body")
        depth = 0
        for index in range(start, len(stripped)):
            if stripped[index] == "{":
                depth += 1
            elif stripped[index] == "}":
                depth -= 1
                if depth == 0:
                    break
        else:
            raise SystemExit(
                f"check_vec_store_contracts: {signature} body never closes"
            )
        # Report against the ORIGINAL text so line numbers stay useful.
        out.append((text[:match.start()].count("\n") + 1, text[match.start():index + 1]))
    return out


def _strip(text: str, keep_literals: bool = False) -> str:
    """Replace comments and string/char literals with spaces, length-preserving.

    Offsets and therefore line numbers are preserved, which is what lets the
    rules below report a real file:line.  Character literals are distinguished
    from lifetimes by the closing quote, so `'a` survives.

    `keep_literals` keeps string CONTENTS and blanks only comments.  Rule A
    needs it: the reserved scratch registers are string literals, so an
    `if let Some(name @ ("ymm0" | "ymm1"))` guard is empty in the stripped view.
    Both modes are length-preserving, so offsets from one are valid in the
    other.  The known imprecision of `keep_literals` is a `//` or a brace
    inside a literal -- it can misplace a header boundary, never accept an
    unproven return (the proof still has to be found in the header text).
    """
    out: list[str] = []
    index, size = 0, len(text)
    while index < size:
        two = text[index:index + 2]
        if two == "//":
            end = text.find("\n", index)
            end = size if end < 0 else end
            out.append(" " * (end - index))
            index = end
        elif two == "/*":
            end = text.find("*/", index + 2)
            end = size if end < 0 else end + 2
            out.append("".join(c if c == "\n" else " " for c in text[index:end]))
            index = end
        elif text[index] == '"':
            end = index + 1
            while end < size and text[end] != '"':
                end += 2 if text[end] == "\\" else 1
            end = min(end + 1, size)
            if keep_literals:
                out.append(text[index:end])
            else:
                out.append("".join(c if c == "\n" else " " for c in text[index:end]))
            index = end
        elif text[index] == "'" and index + 2 < size and text[index + 2] == "'":
            out.append("   ")
            index += 3
        else:
            out.append(text[index])
            index += 1
    return "".join(out)


def _enclosing_if_headers(stripped: str, at: int) -> list[str]:
    """The `if` headers whose braces enclose offset `at`, innermost last.

    `vec_store_source_*` prove a register inside an `if`/`if let` chain, so a
    `return` is justified exactly when one of its ENCLOSING headers names a
    record.  A search of the whole function prefix would be wrong in the
    dangerous direction: one early proof would license every later return, and
    that is precisely how a record-free fallback creeps back in.
    """
    stack: list[tuple[int, str]] = []
    headers: list[str] = []
    stmt_start = 0
    for index, ch in enumerate(stripped):
        if ch == "{":
            header = stripped[stmt_start:index].strip()
            if re.match(r"^(?:\}\s*)?(?:else\s+)?if\b", header):
                stack.append((index, header))
            else:
                stack.append((index, ""))
            stmt_start = index + 1
        elif ch == "}":
            if stack:
                stack.pop()
            stmt_start = index + 1
        elif ch == ";" and stack:
            stmt_start = index + 1
        if index == at:
            break
    return [header for _, header in stack if header]


# Records a store-source resolver must consult.  A resolver that silently
# stops consulting one does not fail loudly -- it degrades to the staging
# default, so some value's store reads `%ymm0` and the destination receives a
# different value.  Absence is therefore a defect, not a simplification.
CONSULTED = {
    "the allocator home": r"reg_assignments\s*\.\s*get",
    "a width-filtered claim": r"vec_live_(?:ymm|xmm)\s*\(",
    "the deferred store or the store peephole":
        r"pending_vec_store|_last_store_(?:val|reg\b|reg_name)",
}


def _check_store_source(
    name: str, text: str, line: int, problems: list[str]
) -> None:
    raw = text
    stripped = _strip(text)
    # Same length, so an offset found in `stripped` is valid here, but the
    # string literals survive: the trusted scratch names live in a guard.
    with_literals = _strip(text, keep_literals=True)
    where = f"{INTRINSICS.relative_to(ROOT)}"

    # -- every `return` stands behind an enclosing proof ---------------------
    for offset, source_line in enumerate(stripped.splitlines()):
        if not re.search(r"\breturn\b", source_line):
            continue
        prefix = stripped.splitlines(keepends=True)[:offset]
        at = sum(len(part) for part in prefix) + source_line.index("return")
        guards = "\n".join(_enclosing_if_headers(with_literals, at))
        if not PROOF.search(guards):
            problems.append(
                f"rule A: {where}:{line + offset} {name} returns without an "
                "enclosing register proof"
            )
            continue
        # A return licensed by the STORE PEEPHOLE (which register the last
        # store moved from) must name an explicit scratch register.  A guard
        # that only CLASSIFIES the name -- `name.starts_with("ymm")` -- is
        # satisfied by an allocator bank whose record proves the VALUE, which
        # is exactly the reading that handed the destination a different
        # vector.
        if "_last_store_reg_name" in guards and not any(
            f'"{scratch}"' in guards for scratch in SCRATCH_NAMES
        ):
            problems.append(
                f"rule A: {where}:{line + offset} {name} licenses a store-"
                "peephole return without naming a reserved scratch register"
            )

    # -- every record is still consulted ------------------------------------
    for what, pattern in CONSULTED.items():
        if not re.search(pattern, stripped):
            problems.append(
                f"rule A: {where}:{line} {name} no longer consults {what}"
            )

    # -- the trusting union is the reserved pair, exactly --------------------
    # Checked on the RAW body: the names are string literals, and a check run
    # over comment- and literal-stripped text sees an empty union and passes.
    for match in re.finditer(r"@\s*\(([^()]*)\)", raw):
        names = set(re.findall(r'"([a-z0-9]+)"', match.group(1)))
        if not names:
            continue
        extra = names - SCRATCH_NAMES
        if extra:
            problems.append(
                f"rule A: {where}:{line + raw[:match.start()].count(chr(10))} "
                f"{name} trusts {sorted(extra)}, not only the reserved scratch "
                "pair (a record naming an allocator bank does not license "
                "reading the staging register)"
            )
    if re.search(r"_last_store_reg_name[^\n]*starts_with", raw):
        problems.append(
            f"rule A: {where}:{line} {name} trusts any register name, not just "
            "the reserved scratch pair"
        )


def rule_a_sources_report_only_proven_registers(
    intrinsics: str, problems: list[str]
) -> None:
    """Every non-default `return` in a store-source resolver is proven."""
    for name in ("vec_store_source_256", "vec_store_source_128"):
        bodies = _blocks(intrinsics, name)
        if not bodies:
            problems.append(f"rule A: {name} is gone (renamed or removed)")
            continue
        if len(bodies) > 1:
            problems.append(f"rule A: {name} is defined {len(bodies)} times")
        line, body = bodies[0]
        _check_store_source(name, body, line, problems)


def rule_b_staging_predicate_compares_the_name(
    state: str, problems: list[str]
) -> None:
    bodies = _blocks(state, "vec_holds_staging_reg")
    if not bodies:
        problems.append("rule B: vec_holds_staging_reg is gone")
        return
    line, body = bodies[0]
    where = f"{STATE.relative_to(ROOT)}:{line}"
    # The predicate deliberately names its operands by destructuring
    # (`let (reg, last_used, last_val, last_name) = ...`), so a general
    # "does this comparison read field X" analysis would need real dataflow.
    # This is a ratchet over our own source, not an analyser for arbitrary
    # Rust: it pins the IDIOM -- the register-name record is read AND some
    # name-carrying local is compared against `Some(...)`. Dropping the name
    # comparison is the mutation this must catch, and it always deletes that
    # comparison, so the idiom is a faithful proxy.
    if "_last_store_reg_name" not in body:
        problems.append(
            f"rule B: {where} vec_holds_staging_reg never reads the "
            "register-name record (only the value would be proven)"
        )
    if not re.search(r"\b\w*name\w*\s*==\s*Some\(", body):
        problems.append(
            f"rule B: {where} vec_holds_staging_reg compares the value but "
            "not the register name (a claim on an allocator bank would license "
            "reading the staging register)"
        )
    if "pending_vec_store" not in body:
        problems.append(
            f"rule B: {where} vec_holds_staging_reg ignores a deferred store, "
            "the only record of a load-elided value"
        )
    else:
        pending_arm = body.split("pending_vec_store")[1]
        # The width must be compared against the CALLER's `wide`, not merely
        # mentioned: a token search alone is satisfied by `_pending_wide`.
        if not re.search(r"\b\w*wide\w*\s*==\s*wide\b", pending_arm):
            problems.append(
                f"rule B: {where} vec_holds_staging_reg accepts a "
                "width-mismatched deferred value (a 128-bit result would feed "
                "a 256-bit read)"
            )


def rule_c_scratch_pair_is_unclaimable(state: str, problems: list[str]) -> None:
    """`vec_claim_live_reg` must refuse the reserved pair.

    This is what makes rule A's scratch-name match sound: if bank 0 or 1 could
    be claimed, `vec_live_ymm` could return `ymm0` for a value that a later
    emitter has since overwritten, and the claim survives every scratch
    invalidation by design (rule D).
    """
    bodies = _blocks(state, "vec_claim_live_reg")
    if not bodies:
        problems.append("rule C: vec_claim_live_reg is gone")
        return
    line, body = bodies[0]
    if not re.search(r"\bbank\s*<\s*2\b", body):
        problems.append(
            f"rule C: {STATE.relative_to(ROOT)}:{line} "
            "vec_claim_live_reg no longer refuses the reserved scratch pair"
        )


def _enclosing_block_start(stripped: str, at: int) -> int:
    """Offset just past the `{` of the innermost block containing `at`.

    Callers use it to bound a local analysis to one statement block, so a
    property of an arm is not inferred from a neighbouring function.
    """
    depth = 0
    for index in range(at - 1, -1, -1):
        if stripped[index] == "}":
            depth += 1
        elif stripped[index] == "{":
            if depth == 0:
                return index + 1
            depth -= 1
    return 0


def rule_d_memory_stores_use_the_scratch_scope(
    intrinsics: str, problems: list[str]
) -> None:
    """No `emit_vec_store_addr` may be preceded by a wide invalidation."""
    stripped = _strip(intrinsics)
    calls = 0
    for match in re.finditer(r"emit_vec_store_addr\(", stripped):
        # The DEFINITION is not a store: `fn emit_vec_store_addr(` matches the
        # same text, and its enclosing block is the whole impl.
        if re.search(r"\bfn\s+$", stripped[max(0, match.start() - 40): match.start()]):
            continue
        calls += 1
        # Scope matters: the arm that stages the source.  An unscoped "nearest
        # invalidation in the file" crosses function boundaries and reports a
        # store whose own arm is correct, while a look-back cap is a magic
        # number that hides a wide wipe moved a little further away.  So the
        # window is the store's ENCLOSING BLOCK, which is exactly the arm.
        block_start = _enclosing_block_start(stripped, match.start())
        before = stripped[block_start: match.start()]
        wide = before.rfind("invalidate_vec_peephole()")
        scratch = before.rfind("invalidate_vec_scratch_peephole()")
        if wide > scratch:
            line = intrinsics[: match.start()].count("\n") + 1
            problems.append(
                f"rule D: {INTRINSICS.relative_to(ROOT)}:{line} "
                "a memory store retires allocator claims (wide invalidation)"
            )
    if "invalidate_vec_scratch_peephole()" not in stripped:
        problems.append("rule D: the scratch scope is gone entirely")
    if calls == 0:
        problems.append(
            "rule D: no memory vector store found (renamed emitter?) -- the "
            "rule would be vacuous"
        )


def rule_e_fold_consults_registers_first(
    intrinsics: str, problems: list[str]
) -> None:
    bodies = _blocks(intrinsics, "vec_arg_mem")
    if not bodies:
        problems.append("rule E: vec_arg_mem is gone")
        return
    line, body = bodies[0]
    held = body.find("vec_arg_register_held")
    slot = body.find("operand_ptr_mem_operand")
    if held < 0:
        problems.append(
            f"rule E: {INTRINSICS.relative_to(ROOT)}:{line} "
            "vec_arg_mem no longer refuses register-held values"
        )
    elif slot >= 0 and slot < held:
        problems.append(
            f"rule E: {INTRINSICS.relative_to(ROOT)}:{line} "
            "vec_arg_mem consults slot validity before the register records"
        )
    if "pending_vec_store" not in body:
        problems.append(
            f"rule E: {INTRINSICS.relative_to(ROOT)}:{line} "
            "vec_arg_mem ignores a deferred store (its slot may be unwritten)"
        )
    elif "flush_pending_vec_store" not in body:
        problems.append(
            f"rule E: {INTRINSICS.relative_to(ROOT)}:{line} "
            "vec_arg_mem sees a deferred store but never commits it before "
            "folding to the slot"
        )


# Reads of the polyglot claim map that carry NO width and are therefore sound.
# A LIST, not a dict: several exemptions share a file, and a dict keyed by path
# would silently keep only the last one. Each entry must match its own line.
WIDTHLESS_CLAIM_READS: list[tuple[str, str, str]] = [
    (
        "src/backend/x86/codegen/emit.rs",
        "    for (&v, &reg_name) in &self.state.vec_live_regs",
        "the whole map is walked to retire every claim at a block edge",
    ),
    (
        "src/backend/x86/codegen/intrinsics.rs",
        "        self.state.vec_live_regs.contains_key(&v)",
        "WIDTH-AGNOSTIC on purpose: ANY claim means the slot may be unwritten, "
        "so a fold must be refused whatever width the claim spells "
        "(`vec_arg_register_held`)",
    ),
    (
        "src/backend/x86/codegen/intrinsics.rs",
        "                !self.state.vec_live_regs.contains_key(&v.0)",
        "a debug_assert that REPORTS a malformed claim; it reads the map on "
        "purpose to show the offending name",
    ),
    (
        "src/backend/x86/codegen/intrinsics.rs",
        "                self.state.vec_live_regs.get(&v.0)",
        "the debug_assert body above: it PRINTS the offending spelling, which "
        "is the whole point of the diagnostic",
    ),
    (
        "src/backend/x86/codegen/intrinsics.rs",
        "                    && this.state.vec_live_regs.get(&v.0).copied() == Some(name)",
        "DST-HOMING: the comparison IS the width-exact test -- `name` comes "
        "from `phys_reg_name_256(home)`, so a claim at another width fails it",
    ),
]

# A consumer with a fixed width must not reach into the map.
RAW_CLAIM_READ = re.compile(r"vec_live_regs\s*\.\s*(?:get|contains_key)\s*\(")


def _test_module_start(text: str) -> int:
    """Offset of the file's `#[cfg(test)]` module, or its length.

    Test code is not an emitter and may construct claims freely; excluding it
    structurally (rather than by matching `assert` in the line) keeps the
    ratchet honest about the code that actually runs in the compiler.
    """
    match = re.search(r"^#\[cfg\(test\)\]", text, re.MULTILINE)
    return match.start() if match else len(text)


def rule_f_claims_are_read_at_a_known_width(
    sources: dict[Path, str], problems: list[str]
) -> None:
    """Every non-exempt claim read goes through a width-filtered accessor."""
    reads = 0
    for path, text in sources.items():
        rel = str(path.relative_to(ROOT))
        cutoff = _test_module_start(text)
        emitted = text[:cutoff]
        for line_no, line in enumerate(emitted.splitlines(), 1):
            if not RAW_CLAIM_READ.search(line):
                continue
            if line.strip().startswith("//"):
                continue
            reads += 1
            if any(
                exempt_file == rel and snippet in line
                for exempt_file, snippet, _ in WIDTHLESS_CLAIM_READS
            ):
                continue
            problems.append(
                f"rule F: {rel}:{line_no} reads the width-polyglot claim map "
                "directly; use vec_live_xmm/vec_live_ymm/vec_live_zmm (a raw "
                "read can splice the wrong bank width into an operand, which "
                "is an illegal encoding)"
            )
    if reads == 0:
        problems.append(
            "rule F: no claim read found at all -- the rule would be vacuous"
        )
    # An exemption that no longer matches its line is a stale licence: the code
    # moved, and the read it excused may now be unexcused (or gone).
    for exempt_file, snippet, _reason in WIDTHLESS_CLAIM_READS:
        path = ROOT / exempt_file
        if not path.is_file() or snippet not in path.read_text(encoding="utf-8"):
            problems.append(
                f"rule F: stale exemption in {exempt_file}: {snippet.strip()!r} "
                "no longer appears"
            )


def check(
    intrinsics: str, state: str, sources: dict[Path, str] | None = None
) -> list[str]:
    problems: list[str] = []
    rule_a_sources_report_only_proven_registers(intrinsics, problems)
    rule_b_staging_predicate_compares_the_name(state, problems)
    rule_c_scratch_pair_is_unclaimable(state, problems)
    rule_d_memory_stores_use_the_scratch_scope(intrinsics, problems)
    rule_e_fold_consults_registers_first(intrinsics, problems)
    if sources is not None:
        rule_f_claims_are_read_at_a_known_width(sources, problems)
    return problems


# --------------------------------------------------------------- self-test ---
#
# Each fixture is the SHAPE the rule accepts, and each mutation breaks exactly
# one rule.  A rule that reports nothing on its own break is not a check.

GOOD_INTRINSICS = """
    fn vec_store_source_256(&mut self, arg: &Operand) -> &'static str {
        if let Operand::Value(v) = arg {
            if let Some(&reg) = self.reg_assignments.get(&v.0) {
                if is_xmm_reg(reg) {
                    return phys_reg_name_256(reg);
                }
            }
            if let Some(held) = self.state.vec_live_ymm(v.0) {
                return held;
            }
            if let Some((p, reg, wide)) = self.state.pending_vec_store {
                if p == v.0 {
                    return if wide { reg } else { "ymm0" };
                }
            }
            if self.state.vec_last_store_reg
                && self.state.vec_last_store_val == Some(v.0)
                && let Some(name @ ("ymm0" | "ymm1")) = self.state.vec_last_store_reg_name
            {
                return name;
            }
        }
        "ymm0"
    }
    fn vec_store_source_128(&mut self, arg: &Operand) -> &'static str {
        if let Operand::Value(v) = arg {
            if let Some(&reg) = self.reg_assignments.get(&v.0) {
                if is_xmm_reg(reg) {
                    return phys_reg_name_128(reg);
                }
            }
            if let Some(held) = self.state.vec_live_xmm(v.0) {
                return held;
            }
            if let Some((p, reg, wide)) = self.state.pending_vec_store {
                if p == v.0 {
                    return if wide { "xmm0" } else { reg };
                }
            }
            if self.state.sse_last_store_reg
                && self.state.sse_last_store_val == Some(v.0)
                && let Some(name @ ("xmm0" | "xmm1")) = self.state.sse_last_store_reg_name
            {
                return name;
            }
        }
        "xmm0"
    }
    fn vec_arg_mem(&mut self, arg: &Operand) -> Option<String> {
        if let Some(mem) = self.memfold_operand(arg) {
            return Some(mem);
        }
        match arg {
            Operand::Value(v) => {
                if self.vec_arg_register_held(v.0) {
                    return None;
                }
                if self.state.pending_vec_store.map(|(p, _, _)| p) == Some(v.0) {
                    self.flush_pending_vec_store_impl();
                }
                self.operand_ptr_mem_operand(arg)
            }
        }
    }
    fn arm() {
        self.state.invalidate_vec_scratch_peephole();
        self.emit_vec_store_addr(args, dest_ptr, "vmovups", src);
    }
"""

GOOD_STATE = """
    pub fn vec_holds_staging_reg(&self, v: u32, wide: bool) -> bool {
        let (reg, last_used, last_val, last_name) = if wide {
            (
                "ymm0",
                self.vec_last_store_reg,
                self.vec_last_store_val,
                self.vec_last_store_reg_name,
            )
        } else {
            (
                "xmm0",
                self.sse_last_store_reg,
                self.sse_last_store_val,
                self.sse_last_store_reg_name,
            )
        };
        if last_used && last_val == Some(v) && last_name == Some(reg) {
            return true;
        }
        matches!(
            self.pending_vec_store,
            Some((p, pending_reg, pending_wide)) if p == v && pending_wide == wide && pending_reg == reg
        )
    }
    pub fn vec_claim_live_reg(&mut self, v: u32, reg: &'static str) {
        let bank = vector_reg_bank(reg);
        if bank < 2 {
            self.vec_live_regs.remove(&v);
            return;
        }
        self.vec_live_regs.insert(v, reg);
    }
"""


# One row per mutation: (label, rule it must trip, edits, side).
#   side "intrinsics" -> mutate GOOD_INTRINSICS, keep GOOD_STATE
#   side "state"      -> mutate GOOD_STATE, keep GOOD_INTRINSICS
# A table, not eighteen near-identical call blocks: the rows are the audit
# record, and a reviewer can see at a glance which rule each mutation targets.
MUTATIONS: list[tuple[str, str, list[tuple[str, str]], str]] = [
    # -- rule A: a resolver names only a proven register -------------------
    (
        "unproven early return",
        "A",
        [(
            "if let Some(held) = self.state.vec_live_ymm(v.0) {",
            'if true {\n                    return "ymm1";\n                }\n'
            "                if let Some(held) = self.state.vec_live_ymm(v.0) {",
        )],
        "intrinsics",
    ),
    (
        "trusted union widened to an allocator bank",
        "A",
        [('"ymm0" | "ymm1")', '"ymm0" | "ymm1" | "ymm3")')],
        "intrinsics",
    ),
    (
        "trusted union widened in the 128-bit twin",
        "A",
        [('"xmm0" | "xmm1")', '"xmm0" | "xmm1" | "xmm5")')],
        "intrinsics",
    ),
    (
        "scratch match widened to any name",
        "A",
        [(
            'Some(name @ ("ymm0" | "ymm1"))',
            'Some(name) if name.starts_with("ymm")',
        )],
        "intrinsics",
    ),
    # Totality: dropping a record does not fail loudly, it degrades to the
    # staging default, so absence must be a violation.
    (
        "allocator home no longer consulted",
        "A",
        [("if let Some(&reg) = self.reg_assignments.get(&v.0) {", "if false {")],
        "intrinsics",
    ),
    (
        "width-filtered claim no longer consulted (256)",
        "A",
        [("vec_live_ymm(v.0)", "vec_live_any(v.0)")],
        "intrinsics",
    ),
    (
        "width-filtered claim no longer consulted (128)",
        "A",
        [("vec_live_xmm(v.0)", "vec_live_any(v.0)")],
        "intrinsics",
    ),
    (
        "deferred store no longer consulted",
        "A",
        [("if let Some((p, reg, wide)) = self.state.pending_vec_store {", "if false {")],
        "intrinsics",
    ),
    # -- rule B: the staging predicate compares the register NAME ----------
    (
        "staging predicate ignores the register name",
        "B",
        [(
            "if last_used && last_val == Some(v) && last_name == Some(reg) {",
            "if last_used && last_val == Some(v) {",
        )],
        "state",
    ),
    (
        "staging predicate never reads the name record",
        "B",
        [
            ("self.vec_last_store_reg_name,", "no_name_here,"),
            ("self.sse_last_store_reg_name,", "no_name_here,"),
        ],
        "state",
    ),
    (
        "staging predicate ignores the deferred width",
        "B",
        [(" && pending_wide == wide", "")],
        "state",
    ),
    (
        "staging predicate ignores deferred stores",
        "B",
        [(
            "matches!(\n            self.pending_vec_store,",
            "matches!(\n            None::<u32>,",
        )],
        "state",
    ),
    # -- rule C: the reserved scratch pair stays unclaimable ---------------
    ("reserved scratch pair claimable", "C", [("if bank < 2 {", "if bank < 99 {")], "state"),
    # -- rule D: a memory store retires only the scratch peephole ----------
    (
        "memory store retires allocator claims",
        "D",
        [(
            "invalidate_vec_scratch_peephole();\n        self.emit_vec_store_addr",
            "invalidate_vec_peephole();\n        self.emit_vec_store_addr",
        )],
        "intrinsics",
    ),
    (
        "scratch scope deleted",
        "D",
        [("self.state.invalidate_vec_scratch_peephole();", "")],
        "intrinsics",
    ),
    # -- rule E: registers before slots ------------------------------------
    (
        "fold consults the slot before the registers",
        "E",
        [(
            "if self.vec_arg_register_held(v.0) {",
            "let _probe = self.operand_ptr_mem_operand(arg);\n"
            "                if self.vec_arg_register_held(v.0) {",
        )],
        "intrinsics",
    ),
    (
        "fold ignores a deferred store",
        "E",
        [("self.flush_pending_vec_store_impl();", "")],
        "intrinsics",
    ),
    (
        "fold lost the register records",
        "E",
        [("vec_arg_register_held(v.0)", "false")],
        "intrinsics",
    ),
]


# Rule F is a cross-file scan, so its fixtures are whole synthetic files:
# (label, path, body, must rule F report it?). The two accepting cases are the
# false-positive guards -- a width-filtered read, and a raw read inside a
# `#[cfg(test)]` module, which never runs in the compiler.
SELF_TEST_RULE_F_CASES: list[tuple[str, str, str, bool]] = [
    (
        "raw claim read in an emitter",
        "src/backend/x86/codegen/fake_emitter.rs",
        """
    fn emit(&mut self, v: &Value) {
        if let Some(&held) = self.state.vec_live_regs.get(&v.0) {
            self.emit_fmt(format_args!("    vmovdqa %{held}, %ymm0"));
        }
    }
""",
        True,
    ),
    (
        "width-filtered read",
        "src/backend/x86/codegen/fake_ok.rs",
        """
    fn emit(&mut self, v: &Value) {
        if let Some(held) = self.state.vec_live_ymm(v.0) {
            self.emit_fmt(format_args!("    vmovdqa %{held}, %ymm0"));
        }
    }
""",
        False,
    ),
    (
        "raw read inside a test module",
        "src/backend/x86/codegen/fake_tested.rs",
        """
    fn emit(&mut self) {}

#[cfg(test)]
mod tests {
    fn t() {
        let held = cg.state.vec_live_regs.get(&7);
        assert!(held == Some(&"zmm3"));
    }
}
""",
        False,
    ),
    (
        "raw read under an exemption's snippet",
        "src/backend/x86/codegen/intrinsics.rs",
        """
    fn emit(&mut self) {
        self.state.vec_live_regs.contains_key(&v)
    }
""",
        False,
    ),
]


def _self_test_rule_f() -> list[str]:
    """Prove rule F accepts the sound shapes and rejects the raw one.

    Rule F is called DIRECTLY rather than through `check`: the other rules need
    the real tree, and feeding them empty strings would manufacture unrelated
    "X is gone" findings that have to be filtered out -- a filter that would
    also hide a real rule-F verdict if the label ever drifted.
    """
    failures: list[str] = []
    for label, rel, body, must_report in SELF_TEST_RULE_F_CASES:
        problems: list[str] = []
        rule_f_claims_are_read_at_a_known_width({ROOT / rel: body}, problems)
        # A fixture may also trip the STALE-EXEMPTION audit (it consults the
        # real tree); only findings about the fixture's own file are in scope.
        reported = [p for p in problems if rel in p]
        if must_report and not reported:
            failures.append(f"self-test: rule F missed {label}")
        if not must_report and reported:
            failures.append(f"self-test: rule F wrongly flagged {label}: {reported[0]}")
    return failures


def self_test() -> int:
    """Prove every rule can fail, on fixtures shaped like the real source.

    Two ways a self-test rots into decoration, both reported as failures:
    a mutation that no longer applies (a fixture edit made it a silent no-op),
    and a mutation that trips a DIFFERENT rule than the one it names (the rule
    it was proving stopped being exercised).  Rules are attributed by their
    `rule X:` prefix, so the table's second column is a real assertion.
    """
    failures: list[str] = []

    # A rule that fires on correct code is worse than no rule: it trains the
    # reader to ignore the gate.  So the accepting direction runs first.
    for problem in check(GOOD_INTRINSICS, GOOD_STATE):
        failures.append(f"self-test: reference fixture wrongly rejected: {problem}")

    for label, rule, edits, side in MUTATIONS:
        mutated = GOOD_STATE if side == "state" else GOOD_INTRINSICS
        for old, new in edits:
            if old not in mutated:
                failures.append(f"self-test: {label} does not apply (stale fixture)")
                break
            candidate = mutated.replace(old, new, 1)
            if candidate == mutated:
                failures.append(f"self-test: {label} is a no-op")
                break
            mutated = candidate
        else:
            intrinsics, state = (
                (GOOD_INTRINSICS, mutated) if side == "state"
                else (mutated, GOOD_STATE)
            )
            findings = check(intrinsics, state)
            if not findings:
                failures.append(f"self-test: {label} produced no verdict")
                continue
            expected = f"rule {rule}:"
            if not any(finding.startswith(expected) for finding in findings):
                failures.append(
                    f"self-test: {label} tripped no {expected} rule: {findings[0]}"
                )

    failures.extend(_self_test_rule_f())

    for failure in failures:
        print(f"  FAIL {failure}")
    if failures:
        print(f"  self-test: {len(failures)} case(s) failed")
        return 1
    print(
        f"  self-test: {len(MUTATIONS)}/{len(MUTATIONS)} mutations rejected by "
        f"their own rule, {len(SELF_TEST_RULE_F_CASES)} rule-F fixtures "
        "classified correctly, reference fixture accepted"
    )
    return 0


def _backend_sources() -> dict[Path, str]:
    """Every non-test Rust source under `src/backend`, keyed by path.

    Rule F is a cross-file ratchet: the polyglot claim map is read in several
    modules, and a width-blind reader added to any of them is the same defect.
    Scanning the whole backend (rather than the two files the other rules own)
    is what makes the ratchet complete; `src/` is small enough that a full
    read costs ~1.3 s including the rules above.
    """
    return {
        path: path.read_text(encoding="utf-8")
        for path in sorted((ROOT / "src" / "backend").rglob("*.rs"))
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return self_test()
    for path in (INTRINSICS, STATE):
        if not path.is_file():
            print(f"check_vec_store_contracts: missing {path}", file=sys.stderr)
            return 1
    problems = check(
        INTRINSICS.read_text(encoding="utf-8"),
        STATE.read_text(encoding="utf-8"),
        _backend_sources(),
    )
    if problems:
        for problem in problems:
            print(f"  FAIL {problem}")
        print(f"VEC_STORE_CONTRACTS_FAIL: {len(problems)} violation(s)")
        return 1
    print("vec_store_contracts: all six laws hold (sources proven, name "
          "compared, scratch unclaimable, memory stores scratch-only, "
          "registers before slots, claims read at a known width)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
