#!/usr/bin/env python3
"""ISA-wide differential against GNU as, driven by GNU as's own instruction table.

Why another differential
------------------------
`asmdiff.py` and `encdiff.py` compare what somebody thought of writing down:
case files and corpora.  The integrated assembler can only be as complete as
those lists, and a statement nobody listed is checked by nobody.  This tool
takes the list from the reference itself.  Every row of binutils'
`opcodes/i386-opc.tbl` (template blocks expanded) names a mnemonic and the
operand classes it takes, in AT&T order; each row is turned into concrete
statements -- a few registers, memory operands and immediates per operand
class, with and without a size suffix -- the ones GNU as assembles are kept,
and lccc assembles the same statements:

  SAME     identical bytes and identical relocations
  REJECT   GNU as assembles the statement, lccc refuses it
  LONGER   lccc's encoding is longer
  SHORTER  lccc's encoding is shorter
  DIFF     same length, different bytes
  RELOC    same bytes, different relocations

DIFF, RELOC and LONGER are defects by definition: the same statement means
something else, resolves to another address, or wastes bytes.  The first run
of this tool found all three kinds (an operation without a size suffix taking
64 bits where GNU as takes 32, a RIP-relative address off by the bytes of a
trailing immediate, `push %cx` pushing four bytes).  SHORTER is acceptable
only for the audited list in the expectation file: lccc commutes the sources
of a commutative VEX instruction to reach the 2-byte prefix, which GNU as
does under `-O2` only.

The table is GPL-3.0-or-later and is *read*, never copied: it is fetched at a
pinned commit, verified against a pinned SHA-256 (the same pin
`scripts/isa/gas-2.47.manifest.json` records for the cross-reference gate) and
cached outside the repository.

Usage
-----
    # report
    scripts/gas_table_sweep.py --as AS --lccc LCCC [--32] [--only MNEMONIC...]

    # CI gate: nothing but SAME, the audited SHORTER statements, and no more
    # rejected statements per mnemonic than the expectation file records
    scripts/gas_table_sweep.py --as AS --lccc LCCC [--32] --check

    # after a fix: rewrite the expectation file from this run
    scripts/gas_table_sweep.py --as AS --lccc LCCC [--32] --update

Statements are assembled in batches, each between two named labels, so one
`as` run and a handful of lccc runs cover the whole table (about a minute per
mode).  The statement set is a pure function of the table and of this file:
what is sampled is chosen by a SHA-256 of the row and the statement, not by a
generator whose sequence a Python release may change, so the expectation
files mean the same statements on every machine.
"""
from __future__ import annotations

import argparse
import base64
import hashlib
import itertools
import json
import os
import re
import struct
import subprocess
import sys
import tempfile
import time
import urllib.request
from collections import Counter, defaultdict
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
EXPECT_DIR = REPO / "tests" / "gas-table-sweep"

# The table, at an immutable commit of the binutils-gdb mirror whose copy is
# byte-identical to the 2.47 release tarball's.
TABLE_REPO = "RTEMS/sourceware-mirror-binutils-gdb"
TABLE_COMMIT = "fd3cf86c638b7fdc0ccb64e178ec43ccd7e9d3f9"
TABLE_BLOB = "7042ca4512ec190463a6f2652c830bff2dc5a535"
TABLE_PATH = "opcodes/i386-opc.tbl"
TABLE_SHA256 = "a48171c05930cbe120a26fdf802689a4865607b3fe42eca784541dbf7ba81179"

DECL = re.compile(r"^<([A-Za-z_][A-Za-z0-9_]*):")
REF = re.compile(r"<([A-Za-z_][A-Za-z0-9_]*)(?::([A-Za-z_][A-Za-z0-9_]*))?>")
SIZES = ("Byte", "Word", "Dword", "Qword")


# ─── The table ────────────────────────────────────────────────────────────

def table_text(path: str | None) -> str:
    """The pinned table: from `path`, the cache, or the network.

    Nothing but the pinned bytes is ever used or kept: a file given with
    `--tbl` that hashes to something else ends the run, a cached copy that
    does is fetched again, and a download that does is one more failed
    attempt. The cache is written under another name and renamed, so the
    two modes may run side by side.
    """
    if path:
        raw = Path(path).read_bytes()
        digest = hashlib.sha256(raw).hexdigest()
        if digest != TABLE_SHA256:
            sys.exit(f"gas_table_sweep: {path} hashes to {digest}, "
                     f"not the pinned {TABLE_SHA256}")
        return raw.decode("utf-8")
    cache = Path(os.environ.get("XDG_CACHE_HOME") or Path.home() / ".cache") / "lccc-gas-table"
    cached = cache / f"i386-opc-{TABLE_SHA256[:16]}.tbl"
    try:
        raw = cached.read_bytes()
    except OSError:
        raw = b""
    if hashlib.sha256(raw).hexdigest() != TABLE_SHA256:
        raw = fetch_table()
        cache.mkdir(parents=True, exist_ok=True)
        partial = cache / f".{cached.name}.{os.getpid()}.partial"
        partial.write_bytes(raw)
        os.replace(partial, cached)
    return raw.decode("utf-8")


def fetch_table() -> bytes:
    """The pinned bytes from the mirror: the raw file, else the blob.

    Three rounds over both, a pause between rounds. An answer that is not
    the pinned bytes -- an error page, a truncated body -- is a failure
    like a refused connection.
    """
    urls = [
        f"https://raw.githubusercontent.com/{TABLE_REPO}/{TABLE_COMMIT}/{TABLE_PATH}",
        f"https://api.github.com/repos/{TABLE_REPO}/git/blobs/{TABLE_BLOB}",
    ]
    errors = []
    for attempt in range(3):
        if attempt:
            time.sleep(5 * attempt)
        for url in urls:
            try:
                request = urllib.request.Request(
                    url, headers={"User-Agent": "lccc-gas-table-sweep"})
                token = os.environ.get("GITHUB_TOKEN") or os.environ.get("GH_TOKEN")
                if token and "api.github.com" in url:
                    request.add_header("Authorization", f"Bearer {token}")
                with urllib.request.urlopen(request, timeout=60) as reply:
                    body = reply.read()
                if "api.github.com" in url:
                    body = base64.b64decode(json.loads(body)["content"])
            except Exception as error:  # noqa: BLE001 -- every failure means "try the next"
                errors.append(f"{url}: {error}")
                continue
            digest = hashlib.sha256(body).hexdigest()
            if digest == TABLE_SHA256:
                return body
            errors.append(f"{url}: {len(body)} bytes hashing to {digest}")
    sys.exit("gas_table_sweep: cannot fetch the instruction table "
             f"(sha256 {TABLE_SHA256}; pass --tbl):\n  " + "\n  ".join(errors))


def split_top(text: str, sep: str = ",") -> list[str]:
    """Split on `sep` outside `<>`, `{}` and parentheses."""
    out, angle, brace, paren, start = [], 0, 0, 0, 0
    for i, ch in enumerate(text):
        # `<` opens a template reference only in front of a name: the opcode
        # language also has `<<`.
        if ch == "<" and i + 1 < len(text) and (text[i + 1].isalpha() or text[i + 1] == "_"):
            angle += 1
        elif ch == ">" and angle:
            angle -= 1
        elif ch == "{":
            brace += 1
        elif ch == "}":
            brace -= 1
        elif ch == "(":
            paren += 1
        elif ch == ")":
            paren -= 1
        elif ch == sep and not (angle or brace or paren):
            out.append(text[start:i])
            start = i + 1
    out.append(text[start:])
    return out


def logical_lines(text: str) -> list[str]:
    """The table's lines, with those continued by a trailing `+` joined."""
    out, buf = [], ""
    for raw in text.splitlines():
        line = raw.strip()
        if not line or line.startswith("//") or line.startswith("#"):
            continue
        continued = line.endswith("+")
        if continued:
            line = line[:-1].rstrip()
        buf = f"{buf} {line}".strip() if buf else line
        if not continued:
            out.append(buf)
            buf = ""
    if buf:
        out.append(buf)
    return out


def expand_rows(text: str):
    """Yield (mnemonic, opcode, cpu, attributes, operand class sets), templates expanded."""
    templates: dict[str, list[dict]] = {}
    rows = []
    for line in logical_lines(text):
        match = DECL.match(line)
        if match and line.endswith(">"):
            body = line[line.index(":") + 1: line.rindex(">")]
            parts = split_top(body)
            fields = ["name"] + [f.strip() for f in parts[0].split(":") if f.strip()]
            instances = []
            for item in parts[1:]:
                if not item.strip():
                    continue
                values = [v.strip() for v in item.split(":")]
                instances.append({f: (values[i] if i < len(values) else "")
                                  for i, f in enumerate(fields)})
            templates[match.group(1)] = instances
            continue
        if line.startswith("{"):
            continue
        rows.append(line)

    def substitute(text: str, chosen: dict[str, dict]) -> str:
        def replace(match: re.Match) -> str:
            instance = chosen[match.group(1)]
            if match.group(2) is None:
                # A first field that starts with `$` selects between
                # spellings and contributes no text.
                value = instance.get("name", "")
                return "" if value.startswith("$") else value
            return instance.get(match.group(2), "")
        return REF.sub(replace, text)

    for line in rows:
        refs = list(dict.fromkeys(name for name, _ in REF.findall(line)))
        if any(name not in templates for name in refs):
            continue
        for combo in (itertools.product(*[templates[r] for r in refs]) if refs else [()]):
            row = substitute(line, dict(zip(refs, combo))) if refs else line
            parts = split_top(row)
            if len(parts) < 4:
                continue
            mnemonic = parts[0].strip()
            if not re.fullmatch(r"[a-z][a-z0-9_.]*", mnemonic):
                continue
            attributes = {a.strip() for a in parts[3].split("|") if a.strip()}
            operands_text = ",".join(parts[4:]).strip()
            operands = []
            if operands_text.startswith("{"):
                inner = operands_text[1: operands_text.rindex("}")].strip()
                if inner:
                    operands = [{t.strip() for t in o.split("|") if t.strip()}
                                for o in split_top(inner)]
            if "SSE2AVX" in attributes:
                mnemonic = "v" + mnemonic
            yield mnemonic, parts[1].strip(), parts[2].strip(), attributes, operands


# ─── Statements ───────────────────────────────────────────────────────────

def operand_choices(classes: set[str], long: bool, attributes: set[str]) -> list[str]:
    """Concrete AT&T operands for one operand class set: a few of each kind."""
    out: list[str] = []
    if "Acc" in classes or "FloatAcc" in classes:
        if "FloatAcc" in classes:
            out.append("%st")
        for size, name in zip(SIZES, ("%al", "%ax", "%eax", "%rax")):
            if size in classes and (long or size != "Qword"):
                out.append(name)
        if "Acc" in classes and not (classes & set(SIZES)):
            out += ["%al", "%ax", "%eax"] + (["%rax"] if long else [])
    for reg, names in (("RegC", ("%cl", "%cx", "%ecx", "%rcx")),
                       ("RegD", ("%dl", "%dx", "%edx", "%rdx")),
                       ("RegB", ("%bl", "%bx", "%ebx", "%rbx"))):
        if reg in classes:
            for size, name in zip(SIZES, names):
                if size in classes and (long or size != "Qword"):
                    out.append(name)
            if not (classes & set(SIZES)):
                out.append(names[2])
    if "Reg8" in classes:
        out += ["%cl", "%dh"] + (["%sil", "%r9b"] if long else [])
    if "Reg16" in classes:
        out += ["%cx"] + (["%r9w"] if long else [])
    if "Reg32" in classes:
        out += ["%ecx", "%ebp"] + (["%r9d"] if long else [])
    if "Reg64" in classes and long:
        out += ["%rcx", "%r9", "%r13"]
    if "ShiftCount" in classes:
        out.append("%cl")
    if "InOutPortReg" in classes:
        out += ["%dx", "(%dx)"]
    if "SReg" in classes:
        out += ["%fs", "%es", "%ds"]
    if "Control" in classes:
        out += ["%cr3", "%cr0"] + (["%cr8"] if long else [])
    if "Debug" in classes:
        out += ["%db7", "%dr3"]
    if "Test" in classes:
        out.append("%tr3")
    if "FloatReg" in classes:
        out += ["%st(2)", "%st(0)"]
    if "RegMMX" in classes:
        out += ["%mm3", "%mm7"]
    if "RegXMM" in classes:
        out += ["%xmm1"] + (["%xmm9", "%xmm17"] if long else ["%xmm6"])
    if "RegYMM" in classes:
        out += ["%ymm2"] + (["%ymm10", "%ymm18"] if long else ["%ymm6"])
    if "RegZMM" in classes:
        out += ["%zmm3"] + (["%zmm20"] if long else ["%zmm6"])
    if "RegMask" in classes:
        out += ["%k2", "%k5"]
    if "RegBND" in classes:
        out.append("%bnd1")
    if "RegTMM" in classes:
        out.append("%tmm3")
    if "Imm1" in classes:
        out.append("$1")
    if "Imm8" in classes:
        out += ["$5", "$0x90"]
    if "Imm8S" in classes:
        out += ["$-3", "$7"]
    if "Imm16" in classes:
        out.append("$0x1234")
    if "Imm32" in classes:
        out.append("$0x12345678")
    if "Imm32S" in classes:
        out.append("$-0x12345678")
    if "Imm64" in classes and long:
        out.append("$0x123456789abcdef")
    # A symbol where a 16-bit or wider immediate fits (GNU as takes one in an
    # 8-bit field too, as an R_*_8 that nobody writes: not swept).
    if classes & {"Imm16", "Imm32", "Imm32S", "Imm64"}:
        out.append("$ext")
    if "BaseIndex" in classes:
        if long:
            out += ["(%rdx)", "0x12(%rbx,%rsi,4)", "0x1234(%r9,%r10,2)", "(%rsp)", "(%rbp)",
                    "sym(%rip)", "0x1000"]
        else:
            out += ["(%edx)", "0x12(%ebx,%esi,4)", "0x1234(,%edi,2)", "(%esp)", "(%ebp)",
                    "sym", "0x1000"]
    elif classes & {"Disp8", "Disp16", "Disp32", "Disp64"}:
        if any(a.startswith("Jump") for a in attributes):
            out += ["1f", "ext"]
        else:
            out.append("0x1122334455667788" if (long and "Disp64" in classes) else "0x11223344")
    return list(dict.fromkeys(out))


def draw(seed: int, *parts: str) -> int:
    """A number that depends on `seed` and `parts` alone, on any Python."""
    text = "\x1f".join((str(seed),) + parts)
    return int.from_bytes(hashlib.sha256(text.encode()).digest()[:8], "big")


def statements(rows, long: bool, per_row: int, only: set[str], seed: int):
    """statement -> the table row it was made from, and the classes no choice exists for."""
    out: dict[str, str] = {}
    unknown: Counter = Counter()
    for mnemonic, opcode, cpu, attributes, operands in rows:
        if only and mnemonic not in only:
            continue
        cpus = set(re.split(r"[&|()]", cpu))
        if long and "No64" in cpus:
            continue
        if not long and ("x64" in cpus or "APX_F" in cpus):
            continue
        per_operand = []
        for classes in operands:
            choices = operand_choices(classes, long, attributes)
            if not choices:
                unknown["|".join(sorted(classes))] += 1
                break
            per_operand.append(choices)
        else:
            combos = list(itertools.product(*per_operand)) if per_operand else [()]
            if len(combos) > per_row:
                # The first choice of every operand, then a sample.
                rest = sorted(combos[1:],
                              key=lambda combo: draw(seed, mnemonic, opcode, ", ".join(combo)))
                combos = [combos[0]] + rest[: per_row - 1]
            suffixes = [""]
            if "NoSuf" not in attributes:
                for suffix, flag in (("b", "No_bSuf"), ("w", "No_wSuf"), ("l", "No_lSuf"),
                                     ("q", "No_qSuf"), ("s", "No_sSuf")):
                    if flag not in attributes and (long or suffix != "q"
                                                   or mnemonic.startswith("f")):
                        suffixes.append(suffix)
            for combo in combos:
                has_memory = any(("(" in o and o != "(%dx)") or o in ("sym", "0x1000")
                                 for o in combo)
                has_register = any(o.startswith("%") for o in combo)
                for suffix in suffixes:
                    statement = (mnemonic + suffix + " " + ", ".join(combo)).strip()
                    # A suffix says something new only where no register
                    # fixes the size; sample the rest (about one in seven).
                    if (suffix and has_register and not has_memory
                            and draw(seed, statement) % 100 >= 15):
                        continue
                    out.setdefault(statement, f"{mnemonic} {opcode} [{cpu}]")
    return out, unknown


# ─── Objects ──────────────────────────────────────────────────────────────

class Elf:
    """Sections, symbols and relocations of an ELF32/ELF64 little-endian object."""

    def __init__(self, path: str):
        self.b = b = Path(path).read_bytes()
        if b[:4] != b"\x7fELF":
            raise ValueError(f"{path}: not an ELF object")
        self.wide = b[4] == 2
        if self.wide:
            (shoff,) = struct.unpack_from("<Q", b, 0x28)
            shentsize, shnum, shstrndx = struct.unpack_from("<HHH", b, 0x3A)
        else:
            (shoff,) = struct.unpack_from("<I", b, 0x20)
            shentsize, shnum, shstrndx = struct.unpack_from("<HHH", b, 0x2E)
        self.sections = []
        for i in range(shnum):
            at = shoff + i * shentsize
            if self.wide:
                name, typ, _f, _a, off, size, link, info, _al, entsize = \
                    struct.unpack_from("<IIQQQQIIQQ", b, at)
            else:
                name, typ, _f, _a, off, size, link, info, _al, entsize = \
                    struct.unpack_from("<IIIIIIIIII", b, at)
            self.sections.append(dict(name_off=name, type=typ, off=off, size=size, link=link,
                                      info=info, entsize=entsize, index=i))
        names = self.sections[shstrndx]
        for section in self.sections:
            section["name"] = self._string(names, section["name_off"])
        self.symbols = []
        table = next((s for s in self.sections if s["type"] == 2), None)
        if table:
            strings = self.sections[table["link"]]
            for i in range(table["size"] // table["entsize"]):
                at = table["off"] + i * table["entsize"]
                if self.wide:
                    name, info, _o, shndx, value, _s = struct.unpack_from("<IBBHQQ", b, at)
                else:
                    name, value, _s, info, _o, shndx = struct.unpack_from("<IIIBBH", b, at)
                text = self._string(strings, name)
                if (info & 0xF) == 3 and 0 < shndx < len(self.sections):
                    text = self.sections[shndx]["name"]
                self.symbols.append(dict(name=text, shndx=shndx, value=value))

    def _string(self, section, offset: int) -> str:
        data = self.b[section["off"] + offset:]
        return data[: data.index(b"\0")].decode("latin-1")

    def data(self, section) -> bytes:
        return self.b[section["off"]: section["off"] + section["size"]]

    def relocations(self, index: int):
        """(offset, type, symbol, addend) of the relocations against section `index`."""
        out = []
        for section in self.sections:
            if section["type"] not in (4, 9) or section["info"] != index:
                continue
            rela = section["type"] == 4
            for i in range(section["size"] // section["entsize"]):
                at = section["off"] + i * section["entsize"]
                if self.wide:
                    offset, info = struct.unpack_from("<QQ", self.b, at)
                    addend = struct.unpack_from("<q", self.b, at + 16)[0] if rela else None
                    symbol, typ = info >> 32, info & 0xFFFFFFFF
                else:
                    offset, info = struct.unpack_from("<II", self.b, at)
                    addend = struct.unpack_from("<i", self.b, at + 8)[0] if rela else None
                    symbol, typ = info >> 8, info & 0xFF
                name = self.symbols[symbol]["name"] if symbol < len(self.symbols) else "?"
                out.append((offset, typ, name, addend))
        return sorted(out)


TAG = "q_"


def batch_source(batch: list[str]) -> str:
    lines = ["\t.text"]
    for i, statement in enumerate(batch):
        lines.append(f"{TAG}{i}:")
        lines.append(f"1:\t{statement}")
    lines.append(f"{TAG}{len(batch)}:")
    # The target of a `1f` in the last statement.
    lines.append("1:\tnop")
    return "\n".join(lines) + "\n"


def read_batch(path: str, count: int):
    """(bytes, relocations) of each of the `count` statements, or None."""
    elf = Elf(path)
    text = next((s for s in elf.sections if s["name"] == ".text"), None)
    if text is None:
        return None
    data = elf.data(text)
    at = {}
    for symbol in elf.symbols:
        if symbol["shndx"] == text["index"] and symbol["name"].startswith(TAG):
            rest = symbol["name"][len(TAG):]
            if rest.isdigit():
                at[int(rest)] = symbol["value"]
    if len(at) != count + 1:
        return None
    relocations = elf.relocations(text["index"])
    out = []
    for i in range(count):
        low, high = at[i], at[i + 1]
        mine = tuple((offset - low, typ, "<next>" if name.startswith(TAG) else name, addend)
                     for offset, typ, name, addend in relocations if low <= offset < high)
        out.append((bytes(data[low:high]), mine))
    return out


def run(command: list[str], **kwargs):
    return subprocess.run(command, capture_output=True, text=True, **kwargs)


def assemble_gas(all_statements: list[str], gas: str, flags: list[str], tmp: Path):
    """{index: (bytes, relocations)} of the statements GNU as assembles."""
    alive = list(range(len(all_statements)))
    for _ in range(8):
        source = tmp / "g.s"
        source.write_text(batch_source([all_statements[i] for i in alive]))
        result = run([gas, *flags, "-o", str(tmp / "g.o"), str(source)])
        if result.returncode == 0:
            got = read_batch(str(tmp / "g.o"), len(alive))
            if got is None:
                sys.exit("gas_table_sweep: cannot read GNU as's object back")
            return dict(zip(alive, got))
        bad = set()
        for match in re.finditer(r"g\.s:(\d+): (?:Error|Fatal error)", result.stderr):
            # Statement k is on line 3 + 2k (1-based), its label on the line before.
            k = (int(match.group(1)) - 2) // 2
            if 0 <= k < len(alive):
                bad.add(k)
        if not bad:
            sys.exit("gas_table_sweep: GNU as failed without naming a line:\n"
                     + result.stderr[:600])
        alive = [i for k, i in enumerate(alive) if k not in bad]
    sys.exit("gas_table_sweep: GNU as keeps finding errors")


def assemble_lccc(all_statements: list[str], indexes: list[int], lccc: str,
                  flags: list[str], tmp: Path):
    """({index: (bytes, relocations)}, {index: first error line}), bisecting on failure."""
    good, bad = {}, {}

    def assemble(part: list[int]) -> None:
        if not part:
            return
        source, obj = tmp / "l.s", tmp / "l.o"
        source.write_text(batch_source([all_statements[i] for i in part]))
        obj.unlink(missing_ok=True)
        try:
            result = run([lccc, *flags, "-c", "-o", str(obj), str(source)], timeout=300)
            code, message = result.returncode, (result.stderr or result.stdout)
        except subprocess.TimeoutExpired:
            code, message = 124, "timeout"
        if code == 0 and obj.exists():
            got = read_batch(str(obj), len(part))
            if got is not None:
                good.update(zip(part, got))
                return
            message = "unreadable object"
        if len(part) == 1:
            lines = message.strip().splitlines()
            bad[part[0]] = lines[0][:200] if lines else f"exit status {code}"
            return
        middle = len(part) // 2
        assemble(part[:middle])
        assemble(part[middle:])

    for start in range(0, len(indexes), 400):
        assemble(indexes[start:start + 400])
    return good, bad


# ─── Verdicts and the expectation file ────────────────────────────────────

def sweep(args) -> dict[str, list[tuple[str, str]]]:
    """verdict -> [(statement, detail)]."""
    long = not args.m32
    rows = list(expand_rows(table_text(args.tbl)))
    candidates, unknown = statements(rows, long, args.per_row, set(args.only or []), args.seed)
    ordered = sorted(candidates)
    with tempfile.TemporaryDirectory() as directory:
        tmp = Path(directory)
        reference = assemble_gas(ordered, args.gas, ["--32" if args.m32 else "--64"], tmp)
        indexes = sorted(reference)
        good, bad = assemble_lccc(ordered, indexes, args.lccc, ["-m32"] if args.m32 else [], tmp)
    report: dict[str, list[tuple[str, str]]] = defaultdict(list)
    for i in indexes:
        theirs = reference[i]
        if i in bad:
            report["REJECT"].append((ordered[i], bad[i]))
            continue
        ours = good[i]
        if ours == theirs:
            verdict, detail = "SAME", ""
        elif len(ours[0]) != len(theirs[0]):
            verdict = "LONGER" if len(ours[0]) > len(theirs[0]) else "SHORTER"
            detail = f"lccc {ours[0].hex()} gas {theirs[0].hex()}"
        elif ours[0] != theirs[0]:
            verdict, detail = "DIFF", f"lccc {ours[0].hex()} gas {theirs[0].hex()}"
        else:
            verdict, detail = "RELOC", f"lccc {ours[1]} gas {theirs[1]}"
        report[verdict].append((ordered[i], detail))
    counts = "  ".join(f"{v} {len(report[v])}" for v in sorted(report))
    print(f"{'32' if args.m32 else '64'}-bit: {len(rows)} table rows, {len(ordered)} statements, "
          f"{len(indexes)} assembled by GNU as:  {counts}")
    if unknown and args.verbose:
        print("operand classes without a choice:", dict(unknown.most_common(12)))
    return report


def expectation_path(m32: bool) -> Path:
    return EXPECT_DIR / f"expected-{'32' if m32 else '64'}.txt"


def write_expectation(report, m32: bool) -> None:
    rejected = Counter(statement.split()[0] for statement, _ in report.get("REJECT", []))
    lines = [
        "# What scripts/gas_table_sweep.py may find beside SAME. Written by --update;",
        "# --check fails on anything this file does not allow.",
        "#",
        "# shorter <statement>: lccc's encoding is shorter than GNU as's, and audited.",
        "# reject <mnemonic> <count>: lccc refuses that many swept statements of the",
        "# mnemonic. The count may only go down.",
    ]
    lines += [f"shorter {statement}" for statement, _ in sorted(report.get("SHORTER", []))]
    lines += [f"reject {mnemonic} {count}" for mnemonic, count in sorted(rejected.items())]
    path = expectation_path(m32)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("\n".join(lines) + "\n")
    print(f"wrote {path.relative_to(REPO)}: {len(report.get('SHORTER', []))} shorter, "
          f"{sum(rejected.values())} rejected statements of {len(rejected)} mnemonics")


def check(report, m32: bool) -> int:
    path = expectation_path(m32)
    shorter, rejects = set(), {}
    for line in path.read_text().splitlines():
        if line.startswith("shorter "):
            shorter.add(line[len("shorter "):])
        elif line.startswith("reject "):
            _, mnemonic, count = line.split()
            rejects[mnemonic] = int(count)
    failures = []
    for verdict in ("DIFF", "RELOC", "LONGER"):
        for statement, detail in report.get(verdict, []):
            failures.append(f"{verdict}: {statement}   {detail}")
    for statement, detail in report.get("SHORTER", []):
        if statement not in shorter:
            failures.append(f"SHORTER (not audited): {statement}   {detail}")
    rejected = defaultdict(list)
    for statement, detail in report.get("REJECT", []):
        rejected[statement.split()[0]].append((statement, detail))
    improved = 0
    for mnemonic, items in sorted(rejected.items()):
        allowed = rejects.get(mnemonic, 0)
        if len(items) > allowed:
            failures.append(f"REJECT: `{mnemonic}`: {len(items)} statements refused, "
                            f"{allowed} allowed, e.g. {items[0][0]}   {items[0][1]}")
        improved += max(0, allowed - len(items))
    improved += sum(count for mnemonic, count in rejects.items() if mnemonic not in rejected)
    for failure in failures[:60]:
        print("  " + failure)
    if failures:
        print(f"gas_table_sweep: {len(failures)} finding(s) "
              f"{path.relative_to(REPO)} does not allow")
        return 1
    if improved:
        print(f"gas_table_sweep: ok; {improved} recorded rejection(s) are gone -- "
              "run --update to tighten the ratchet")
    else:
        print("gas_table_sweep: ok")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--tbl", help="opcodes/i386-opc.tbl of binutils 2.47 (default: fetch)")
    parser.add_argument("--as", dest="gas", default="as", help="GNU as 2.47")
    parser.add_argument("--lccc", required=True)
    parser.add_argument("--32", dest="m32", action="store_true", help="32-bit mode")
    parser.add_argument("--per-row", type=int, default=6,
                        help="operand combinations per table row")
    parser.add_argument("--seed", type=int, default=1)
    parser.add_argument("--only", nargs="+", metavar="MNEMONIC",
                        help="sweep these table mnemonics only (no --check/--update)")
    parser.add_argument("--check", action="store_true",
                        help="fail on anything the expectation file does not allow")
    parser.add_argument("--update", action="store_true",
                        help="rewrite the expectation file from this run")
    parser.add_argument("--show", type=int, default=30, help="statements to print per verdict")
    parser.add_argument("--json", help="write every non-SAME statement here")
    parser.add_argument("--verbose", action="store_true")
    args = parser.parse_args()
    if args.only and (args.check or args.update):
        parser.error("--only sweeps a part of the table; --check and --update need all of it")

    report = sweep(args)
    if args.json:
        Path(args.json).write_text(json.dumps(
            {verdict: items for verdict, items in report.items() if verdict != "SAME"}, indent=0))
    if args.update:
        blocking = [v for v in ("DIFF", "RELOC", "LONGER") if report.get(v)]
        if blocking:
            print("gas_table_sweep: not updating: " + ", ".join(blocking)
                  + " findings are defects, not expectations")
            return 1
        write_expectation(report, args.m32)
        return 0
    if args.check:
        return check(report, args.m32)
    for verdict in ("DIFF", "RELOC", "LONGER", "SHORTER", "REJECT"):
        items = report.get(verdict, [])
        if not items:
            continue
        mnemonics = Counter(statement.split()[0] for statement, _ in items)
        print(f"\n== {verdict}: {len(items)} statements, {len(mnemonics)} mnemonics")
        for statement, detail in items[: args.show]:
            print(f"  {statement:<52} {detail}")
        if verdict == "REJECT":
            print("  mnemonics: " + " ".join(f"{m}({c})" for m, c in sorted(mnemonics.items())))
    return 0


if __name__ == "__main__":
    sys.exit(main())
