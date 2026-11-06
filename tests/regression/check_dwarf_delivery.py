#!/usr/bin/env python3
"""DWARF delivery contracts: assembly equivalence and reproducible source paths.

D7: `-S -g` followed by either GAS or lccc's assembler must retain the same
DIE tree as `-c -g`, relocate to the same code, link, run, and remain visible
to GDB.  Comparing section names alone is insufficient: an empty `.debug_info`
would pass such a test while losing every type and object.

D10: `-fdebug-prefix-map` changes debug paths only; `-fmacro-prefix-map`
changes `__FILE__` only; `-ffile-prefix-map` changes both.  Two physical build
directories mapped to one logical directory must produce identical objects.
The first matching command-line mapping wins (GCC 16.2 measured).

The ELF/DWARF decoder is shared with check_dwarf_quality.py, including its
relocation application and compression support.  No readelf/pyelftools is
required.  GDB is optional outside the provisioned CI environment; its skip
is printed and recorded in JSON rather than disguised as a pass.
"""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

sys.path.insert(0, str(Path(__file__).resolve().parent))
import check_dwarf_quality as q

SOURCE = '''typedef unsigned long pd_size_t;
struct pd_pair { int left; long right; };
int pd_global = 42;
static int pd_static = 17;
__attribute__((noinline)) int pd_value(int x, pd_size_t increment) {
    return x + (int)increment + pd_static - 17;
}
int main(void) {
    struct pd_pair pair = {pd_global, 7};
    return pd_value(pair.left, (pd_size_t)pair.right) - 49;
}
'''
FILE_SOURCE = '''const char *pd_file = __FILE__;
const char *pd_base = __BASE_FILE__;
int pd_data = 1;
'''
TREE_SECTIONS = (".debug_info", ".debug_abbrev", ".debug_str",
                 ".debug_aranges", ".debug_ranges", ".debug_rnglists")


def run(argv: list[str], cwd: Path, *, timeout: int = 120) -> subprocess.CompletedProcess:
    env = dict(os.environ)
    # A debugger must never stop on an interactive pagination prompt in CI.
    env["PAGER"] = "cat"
    return subprocess.run(argv, cwd=cwd, env=env, text=True, stdout=subprocess.PIPE,
                          stderr=subprocess.PIPE, timeout=timeout, check=False)


def require(argv: list[str], cwd: Path, *, timeout: int = 120) -> str:
    result = run(argv, cwd, timeout=timeout)
    if result.returncode:
        raise AssertionError(f"{argv!r} exited {result.returncode}\n{result.stdout}\n{result.stderr}")
    return result.stdout


def hierarchy(elf: q.Elf) -> list[tuple]:
    """Tag/name/type/declaration tree with explicit parent-child nesting.

    DWARF offsets and relocation addends legitimately differ between assembly
    paths.  The tree and its source attributes must not.  Type references are
    resolved recursively to structural names rather than compared as offsets.
    """
    def type_shape(die: q.Die, unit: q.Unit, seen: frozenset[int] = frozenset()) -> tuple:
        if die.offset in seen:
            return ("cycle", die.tag)
        seen = seen | {die.offset}
        target = q.type_of(die, unit)
        return (die.tag, q.die_name(die, unit), die.attrs.get(q.DW_AT_BYTE_SIZE),
                die.attrs.get(q.DW_AT_ENCODING),
                type_shape(target, unit, seen) if target is not None else None)

    result = []
    for unit in q.units(elf):
        ancestors: dict[int, tuple] = {}
        for die in unit.dies:
            path = tuple(ancestors[depth] for depth in range(die.depth))
            target = q.type_of(die, unit)
            result.append((path, die.tag, q.die_name(die, unit),
                           die.attrs.get(q.DW_AT_DECL_FILE), die.attrs.get(q.DW_AT_DECL_LINE),
                           die.attrs.get(q.DW_AT_EXTERNAL), die.attrs.get(q.DW_AT_DECLARATION),
                           type_shape(target, unit) if target is not None else None))
            ancestors[die.depth] = (die.tag, q.die_name(die, unit))
    return result


def assert_runtime(path: Path, cwd: Path) -> None:
    result = run([str(path)], cwd)
    if result.returncode != 0:
        raise AssertionError(f"{path.name} exited {result.returncode}, expected 0: {result.stderr}")


def check_assembly(lccc: str, gas: str | None, gcc: str, gdb: str | None,
                   root: Path, results: dict, *, prefix: str = "", bits: int = 64,
                   execute: bool = True) -> None:
    source = root / "delivery.c"
    source.write_text(SOURCE)
    for version in (2, 3, 4, 5):
        for extra in ([], ["-ffunction-sections"]):
            label = f"{prefix}v{version}{'-sections' if extra else ''}"
            flags = ["-O0", "-g", f"-gdwarf-{version}", *extra]
            direct = root / f"{label}-direct.o"
            require([lccc, *flags, "-c", str(source), "-o", str(direct)], root)
            text = root / f"{label}.s"
            require([lccc, *flags, "-S", str(source), "-o", str(text)], root)
            direct_elf = q.read_elf(direct)
            expected = hierarchy(direct_elf)
            if not any(row[1] == q.DW_TAG_VARIABLE and row[2] == "pd_global" for row in expected):
                raise AssertionError(f"{label}: direct object has no pd_global variable DIE")
            assemblers = [("lccc", [lccc, "-c", "-x", "assembler", str(text), "-o"])]
            if gas is not None:
                assemblers.append(("gas", [gas, f"--{bits}", "-g", f"-gdwarf-{version}",
                                           str(text), "-o"]))
            for name, command in assemblers:
                output = root / f"{label}-{name}.o"
                require([*command, str(output)], root)
                actual = q.read_elf(output)
                actual_tree = hierarchy(actual)
                if actual_tree != expected:
                    detail = next((f"row {i}: direct={x!r} assembled={y!r}"
                                   for i, (x, y) in enumerate(zip(expected, actual_tree)) if x != y),
                                  f"row count direct={len(expected)} assembled={len(actual_tree)}")
                    raise AssertionError(f"D7 {label}/{name}: DIE tree differs from direct -c ({detail})")
                for section in TREE_SECTIONS:
                    if direct_elf.sections.get(section) != actual.sections.get(section):
                        raise AssertionError(f"D7 {label}/{name}: {section} payload differs")
                for section, payload in direct_elf.sections.items():
                    if section.startswith(".text") and actual.sections.get(section) != payload:
                        raise AssertionError(f"D7 {label}/{name}: {section} code changed")
                if execute:
                    executable = root / f"{label}-{name}.elf"
                    linker = [gcc, "-no-pie"] if name == "gas" else [lccc, "-no-pie"]
                    if bits == 32 and name == "gas":
                        linker.insert(1, "-m32")
                    require([*linker, str(output), "-o", str(executable)], root)
                    assert_runtime(executable, root)
                if execute and gdb is not None and version == 5 and not extra:
                    script = ["-batch", "-ex", "set pagination off", "-ex", "break pd_value",
                              "-ex", "run", "-ex", "print pd_global", "-ex", "ptype pd_size_t",
                              "-ex", "frame", str(executable)]
                    transcript = require([gdb, *script], root)
                    # pd_size_t is referenced by a parameter, so its DIE survives
                    # referenced-declaration pruning even before local-variable
                    # DIEs (campaign D14) are implemented.
                    if "= 42" not in transcript or "unsigned long" not in transcript:
                        raise AssertionError(f"D7 {label}/{name}: GDB lost object/type visibility:\n{transcript}")
                results[f"assembly-{label}-{name}"] = "PASS"


def check_paths(lccc: str, gas: str | None, root: Path, results: dict) -> None:
    logical = "."
    objects: dict[str, bytes] = {}
    for directory in ("build-a", "build-b"):
        cwd = root / directory
        cwd.mkdir()
        (cwd / "paths.c").write_text(FILE_SOURCE)
        for mode in ("debug", "macro", "file"):
            mapping = f"-f{mode}-prefix-map={cwd}={logical}"
            output = cwd / f"{mode}.o"
            require([lccc, "-O0", "-g", mapping, "-c", str(cwd / "paths.c"), "-o", str(output)], cwd)
            elf = q.read_elf(output)
            debug_paths = b"\0".join(elf.sections.get(name, b"") for name in
                                     (".debug_str", ".debug_line_str"))
            wants_debug_map = mode in ("debug", "file")
            if (str(cwd).encode() in debug_paths) != (not wants_debug_map):
                raise AssertionError(f"D10 {mode}: debug path mapping not honored")
            executable = cwd / mode
            # Read the actual string object via a tiny inspector, not the
            # compiler's preprocessed text: this checks the final machine data.
            inspector = cwd / "inspect.c"
            inspector.write_text('extern const char *pd_file; extern const char *pd_base;'
                                 'int puts(const char *); int main(void){puts(pd_file);puts(pd_base);return 0;}')
            # Compile inspector without the source object's macro mapping.
            require([lccc, "-O0", str(inspector), str(output), "-o", str(executable)], cwd)
            lines = require([str(executable)], cwd).splitlines()
            expected = "./paths.c" if mode in ("macro", "file") else str(cwd / "paths.c")
            if lines != [expected, expected]:
                raise AssertionError(f"D10 {mode}: __FILE__/__BASE_FILE__ = {lines}, expected {[expected]*2}")
            if wants_debug_map:
                # -fdebug-prefix-map intentionally does NOT rewrite __FILE__,
                # so its allocated string constants legitimately differ.  Its
                # complete debug payload must nevertheless be reproducible.
                # -ffile-prefix-map maps both channels and requires equality
                # of the entire object, including allocated data and symbols.
                fingerprint = output.read_bytes() if mode == "file" else tuple(
                    (name, payload) for name, payload in sorted(elf.sections.items())
                    if name.startswith(".debug_"))
                if mode in objects and objects[mode] != fingerprint:
                    raise AssertionError(f"D10 {mode}: mapped {'objects' if mode == 'file' else 'debug sections'} differ across build directories")
                objects.setdefault(mode, fingerprint)
            if gas is not None:
                text = cwd / f"{mode}.s"
                via = cwd / f"{mode}-gas.o"
                require([lccc, "-O0", "-g", mapping, "-S", str(cwd / "paths.c"), "-o", str(text)], cwd)
                require([gas, "--64", "-g", str(text), "-o", str(via)], cwd)
                via_elf = q.read_elf(via)
                via_paths = b"\0".join(via_elf.sections.get(name, b"") for name in
                                        (".debug_str", ".debug_line_str"))
                if (str(cwd).encode() in via_paths) != (not wants_debug_map):
                    raise AssertionError(f"D10 {mode}: -S + GAS debug path mapping not honored")
                via_executable = cwd / f"{mode}-gas"
                require([lccc, "-O0", str(inspector), str(via), "-o", str(via_executable)], cwd)
                if require([str(via_executable)], cwd).splitlines() != [expected, expected]:
                    raise AssertionError(f"D10 {mode}: -S + GAS changed __FILE__/__BASE_FILE__")
            results[f"prefix-{mode}"] = "PASS"
    # First-match precedence, including an overlapping shorter prefix first.
    cwd = root / "build-a"
    require([lccc, "-g", f"-ffile-prefix-map={cwd}=FIRST", f"-ffile-prefix-map={cwd}/paths=SECOND",
             "-c", str(cwd / "paths.c"), "-o", "order.o"], cwd)
    strings = b"\0".join(q.read_elf(cwd / "order.o").sections.get(name, b"")
                         for name in (".debug_str", ".debug_line_str"))
    if b"FIRST" not in strings or b"SECOND" in strings:
        raise AssertionError("D10: first matching prefix did not win")
    results["prefix-first-match"] = "PASS"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lccc", default="target/fastbuild/lccc")
    parser.add_argument("--lccc-i686")
    parser.add_argument("--lccc-arm")
    parser.add_argument("--lccc-riscv")
    parser.add_argument("--gas", default=shutil.which("as"))
    parser.add_argument("--gcc", default=os.environ.get("CC", "gcc"))
    parser.add_argument("--gdb", default=shutil.which("gdb"))
    parser.add_argument("--allow-missing-gdb", action="store_true")
    parser.add_argument("--json", type=Path)
    args = parser.parse_args()
    lccc = str(Path(args.lccc).resolve()) if Path(args.lccc).is_file() else shutil.which(args.lccc)
    if lccc is None:
        parser.error(f"compiler not found: {args.lccc}")
    def resolve_optional(value: str | None) -> str | None:
        if value is None:
            return None
        path = Path(value)
        resolved = str(path.resolve()) if path.is_file() else shutil.which(value)
        if resolved is None:
            parser.error(f"compiler not found: {value}")
        return resolved
    gcc = shutil.which(args.gcc) or args.gcc
    gas = shutil.which(args.gas) if args.gas else None
    gdb = shutil.which(args.gdb) if args.gdb else None
    if gdb is None and not args.allow_missing_gdb:
        print("FAIL: GDB is required; use --allow-missing-gdb only for an explicitly partial run", file=sys.stderr)
        return 2
    results: dict[str, str] = {"gdb": gdb or "SKIPPED (not installed)"}
    try:
        with tempfile.TemporaryDirectory(prefix="dwarf-delivery-") as directory:
            root = Path(directory)
            check_assembly(lccc, gas, gcc, gdb, root, results)
            for name, value, bits, execute in (
                    ("i686", args.lccc_i686, 32, True),
                    ("arm", args.lccc_arm, 64, False),
                    ("riscv", args.lccc_riscv, 64, False)):
                compiler = resolve_optional(value)
                if compiler is not None:
                    target_root = root / name
                    target_root.mkdir()
                    check_assembly(compiler, gas if bits == 32 else None, gcc,
                                   None, target_root, results, prefix=f"{name}-",
                                   bits=bits, execute=execute)
            check_paths(lccc, gas, root, results)
    except (AssertionError, OSError, ValueError, subprocess.TimeoutExpired) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        results["failure"] = str(error)
        if args.json:
            args.json.write_text(json.dumps(results, indent=2) + "\n")
        return 1
    print(f"PASS: DWARF delivery ({len(results) - 1} contracts; GDB {gdb or 'skipped'})")
    if args.json:
        args.json.write_text(json.dumps(results, indent=2) + "\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
