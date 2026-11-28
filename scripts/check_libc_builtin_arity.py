#!/usr/bin/env python3
"""Generate GCC 16.2 hard arity errors for LCCC's implicit and explicit libc aliases.

Measure the two spellings separately: GCC rejects `__builtin_puts()` but not
`puts()` in a legacy dialect, for example. Unlisted names have no hard error
from this table; that says nothing about warnings or actual ABI prototypes.
"""
from __future__ import annotations

import argparse
from pathlib import Path
import os
import re
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parent.parent
BUILTINS = ROOT / "src/frontend/sema/builtins.rs"
TABLE = BUILTINS.with_name("libc_builtin_arity.rs")
VERSION = "16.2.0"
MAX_ARGS = 10
BATCH_SIZE = 32
IDENT = r"([A-Za-z_][A-Za-z_0-9]*)"
PAIR = rf'"{IDENT}"\s*,\s*"{IDENT}"'
DIRECT = re.compile(rf'm\.insert\(\s*"{IDENT}"\s*,\s*BuiltinInfo::simple\("{IDENT}"\)\s*,?\s*\)')
LOOP = re.compile(
    r"for &\(builtin, libc_name\) in &\[(.*?)\]\s*\{\s*"
    r"m\.insert\(builtin, BuiltinInfo::simple\(libc_name\)\);\s*\}", re.S,
)
PROTOTYPE = re.compile(r'^    \("(__builtin_[A-Za-z_0-9]+)", LibcPrototype ', re.M)
PROTOTYPES = BUILTINS.with_name("libc_builtin_types.rs")
DIAGNOSTIC = re.compile(
    rf":\d+:\d+: error: too (few|many) arguments to function '{IDENT}'; "
    r"expected (at least )?(\d+), have (\d+)$",
)


def registered(source: str) -> list[str]:
    """Both spellings of every alias builtins.rs writes out by name."""
    direct = DIRECT.findall(source)
    loops = LOOP.findall(source)
    if len(direct) + len(loops) != source.count("BuiltinInfo::simple("):
        raise ValueError("unrecognized libc alias registration in builtins.rs")
    records = direct + [pair for loop in loops for pair in re.findall(PAIR, loop)]
    if not records:
        raise ValueError("no libc alias registrations found")
    return sorted({name for pair in records for name in pair})


def candidates(source: str, prototypes: str) -> list[str]:
    """The aliases written out by name and those registered from the table of
    GCC's prototypes (`BuiltinInfo::library`): every builtin of that table is
    one, and its library function is the name without the prefix."""
    if "BuiltinInfo::library(name)" not in source:
        raise ValueError("builtins.rs no longer registers the prototype table")
    listed = PROTOTYPE.findall(prototypes)
    if not listed:
        raise ValueError("no prototype in libc_builtin_types.rs")
    return sorted(set(registered(source)) | {
        name for builtin in listed for name in (builtin, builtin.removeprefix("__builtin_"))})


def probe_source(names: list[str]) -> str:
    calls = (f"    {name}({', '.join(['1'] * argc)});"
             for name in names for argc in range(MAX_ARGS + 1))
    return "void probe(void) {\n" + "\n".join(calls) + "\n}\n"


def parse_arity(stderr: str, batch: list[str]) -> dict[str, tuple[int, bool]]:
    facts: dict[str, tuple[int, bool]] = {}
    observations: dict[str, set[int]] = {}
    for line in stderr.splitlines():
        match = DIAGNOSTIC.search(line)
        if not match:
            continue
        direction, name, variadic, minimum, passed = match.groups()
        arity, actual = int(minimum), int(passed)
        fact = (arity, bool(variadic))
        if name not in batch or not 0 <= arity < MAX_ARGS or not 0 <= actual <= MAX_ARGS:
            raise ValueError(f"unexpected arity diagnostic: {line}")
        if (direction == "few" and actual >= arity) or (
            direction == "many" and (variadic or actual <= arity)
        ) or facts.setdefault(name, fact) != fact:
            raise ValueError(f"inconsistent arity diagnostic: {line}")
        observations.setdefault(name, set()).add(actual)
    for name, (arity, variadic) in facts.items():
        counts = set(range(arity))
        if not variadic:
            counts.update(range(arity + 1, MAX_ARGS + 1))
        if observations[name] != counts:
            raise ValueError(f"{name}: incomplete hard-error coverage: {sorted(observations[name])}")
    return facts


def measure(gcc: str, names: list[str]) -> dict[str, tuple[int, bool]]:
    env = dict(os.environ, LC_ALL="C")
    version = subprocess.check_output([gcc, "-dumpfullversion"], text=True, env=env, timeout=30).strip()
    if version != VERSION:
        raise ValueError(f"expected GCC {VERSION}, found {version} ({gcc})")
    table = {}
    with tempfile.TemporaryDirectory(prefix="lccc-libc-arity.") as tmp:
        source = Path(tmp) / "probe.c"
        for start in range(0, len(names), BATCH_SIZE):
            batch = names[start:start + BATCH_SIZE]
            source.write_text(probe_source(batch), encoding="utf-8")
            result = subprocess.run([
                gcc, "-std=gnu90", "-fsyntax-only", "-fmax-errors=0",
                "-fdiagnostics-color=never", "-fno-diagnostics-show-caret", source,
            ], capture_output=True, text=True, env=env, timeout=60)
            if result.returncode not in (0, 1) or any(
                msg in result.stderr for msg in ("fatal error:", "internal compiler error:")
            ):
                raise ValueError(f"GCC probe failed ({result.returncode}): {result.stderr[-1000:]}")
            table.update(parse_arity(result.stderr, batch))
    # Canary cases rule out an incompatible diagnostic format or a silent oracle.
    for name, expected in {
        "memcpy": (3, False), "__builtin_puts": (1, False), "printf": (1, True),
    }.items():
        if table.get(name) != expected:
            raise ValueError(f"GCC arity canary {name} did not yield {expected}")
    return table


def render(table: dict[str, tuple[int, bool]], total: int) -> str:
    header = f'''//! GCC {VERSION} hard arity errors for implicit libc calls and explicit aliases.
//!
//! Generated by `scripts/check_libc_builtin_arity.py --write` ({len(table)}/{total} spellings).
//! Missing entries are not hard arity errors. This table never invents types.

pub(crate) struct LibcBuiltinArity {{
    pub(crate) min_args: usize,
    pub(crate) variadic: bool,
}}

pub(crate) fn libc_builtin_arity(name: &str) -> Option<LibcBuiltinArity> {{
    let i = ARITY_TABLE.binary_search_by_key(&name, |row| row.0).ok()?;
    let (_, min_args, variadic) = ARITY_TABLE[i];
    Some(LibcBuiltinArity {{
        min_args: usize::from(min_args),
        variadic,
    }})
}}

const ARITY_TABLE: &[(&str, u8, bool)] = &[
'''
    rows = ''.join(f'    ("{name}", {arity}, {str(variadic).lower()}),\n'
                   for name, (arity, variadic) in sorted(table.items()))
    return header + rows + '''];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_unique_and_sorted() {
        assert!(ARITY_TABLE.windows(2).all(|rows| rows[0].0 < rows[1].0));
    }

    #[test]
    fn spellings_are_checked_independently() {
        assert!(libc_builtin_arity("puts").is_none());
        assert_eq!(libc_builtin_arity("__builtin_puts").unwrap().min_args, 1);
        assert_eq!(libc_builtin_arity("memcpy").unwrap().min_args, 3);
        assert!(libc_builtin_arity("printf").unwrap().variadic);
    }
}
'''


def self_test() -> None:
    def diagnostic(name, count, arity, variadic):
        direction = "few" if count < arity else "many"
        expected = "at least " if variadic else ""
        return f"probe.c:1:2: error: too {direction} arguments to function '{name}'; expected {expected}{arity}, have {count}\n"

    names = candidates(BUILTINS.read_text(encoding="utf-8"),
                       PROTOTYPES.read_text(encoding="utf-8"))
    assert {"bzero", "__builtin_bzero", "memcpy", "__builtin_memcpy"} <= set(names)
    assert "__builtin___clear_cache" in names and "__clear_cache" in names
    assert {"tan", "__builtin_tan", "strnlen", "__builtin_strnlen"} <= set(names)
    row = '    ("__builtin_tan", LibcPrototype { result: LibcType::Double, parameters: &[] }),\n'
    assert PROTOTYPE.findall("const P: &[(&str, LibcPrototype)] = &[\n" + row) == ["__builtin_tan"]
    for source, prototypes in (("", row), ("BuiltinInfo::library(name)", "")):
        try:
            candidates(source + 'm.insert("a", BuiltinInfo::simple("b"))', prototypes)
        except ValueError:
            continue
        raise AssertionError("accepted a builtin table without the prototype registration")
    explicit = ''.join(diagnostic("__builtin_abs", i, 1, False)
                       for i in range(MAX_ARGS + 1) if i != 1)
    variadic = diagnostic("printf", 0, 1, True)
    assert parse_arity(explicit + variadic, ["__builtin_abs", "printf", "abs"]) == {
        "__builtin_abs": (1, False), "printf": (1, True),
    }
    assert parse_arity(variadic.replace("error:", "warning:"), ["printf"]) == {}
    try:
        parse_arity(diagnostic("memcpy", 0, 3, False), ["memcpy"])
    except ValueError:
        pass
    else:
        raise AssertionError("accepted truncated fixed-arity evidence")
    assert len(probe_source(["memcpy", "printf"]).splitlines()) == 2 * (MAX_ARGS + 1) + 2
    print(f"PASS: libc arity derivation self-test ({len(names)} candidate spellings)")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--gcc", default=os.environ.get("GCC", "gcc-16"))
    mode = parser.add_mutually_exclusive_group(required=True)
    for option in ("--check", "--write", "--dump", "--self-test"):
        mode.add_argument(option, action="store_true")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return 0
    names = candidates(BUILTINS.read_text(encoding="utf-8"),
                       PROTOTYPES.read_text(encoding="utf-8"))
    table = measure(args.gcc, names)
    data = render(table, len(names))
    if args.dump:
        print(data, end="")
    elif args.write:
        TABLE.write_text(data, encoding="utf-8")
    elif TABLE.read_text(encoding="utf-8") != data:
        raise ValueError("libc_builtin_arity.rs is stale; regenerate with --write")
    else:
        print(f"PASS: libc arity agrees with GCC {VERSION} ({len(table)}/{len(names)} spellings)")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        sys.exit(f"FAIL: {error}")
