#!/usr/bin/env python3
"""Every regression fixture that uses an ISA-gated x86 intrinsic must ship a
NAME.flags file that enables that ISA, or be one of the named fixtures that
grant it in the source (`SOURCE_GRANTED`).

run_regression.py falls back to DEFAULT_FLAGS (-O2, no ISA) when a .flags file
is absent. A fixture that calls an AVX intrinsic under that default fails with
"requires the avx instruction-set extension", which is what broke the SSA
corpus lane on avx1_fp256_logic_only.c. This check makes that a static failure
instead of a CI-only one.
"""
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent

# Intrinsic-name prefix -> the -m flag that enables it. Only the families whose
# absence the compiler rejects; SSE2 is the x86-64 baseline and needs no flag.
REQUIRES = (
    (re.compile(r"\b_mm256_\w+"), "-mavx"),
    (re.compile(r"\b_mm512_\w+"), "-mavx512f"),
)


# A flag enables every ISA it implies, so -mavx2 satisfies -mavx. Only the
# implications the x86 driver actually has are listed; each is a superset.
IMPLIES = {
    "-mavx2": {"-mavx"},
    "-mavx512f": {"-mavx", "-mavx2"},
    "-mavx512bw": {"-mavx512f", "-mavx", "-mavx2"},
    "-mavx512vl": {"-mavx512f", "-mavx", "-mavx2"},
}


def flags_for(src: pathlib.Path) -> set[str]:
    f = src.with_suffix(".flags")
    words = f.read_text().split() if f.exists() else ["-O2"]
    enabled = set(words)
    for word in words:
        enabled |= IMPLIES.get(word, set())
    return enabled


# Preprocessor guards that remove an intrinsic when its ISA is absent. A use
# inside such a region is legitimate without the flag: the fixture is written to
# exercise the non-ISA path. Only unguarded uses are required to have the flag.
_GUARD = re.compile(
    r"^[ \t]*#[ \t]*if(?:def)?[ \t]*\(?[ \t]*(?:defined[ \t]*\(?[ \t]*)?__AVX",
    re.M,
)


def strip_isa_guarded(text: str) -> str:
    """Return text with every `#if ...__AVX...` region (to its matching #endif) blanked."""
    out, depth, guarded = [], 0, False
    for line in text.splitlines(keepends=True):
        stripped = line.strip()
        if stripped.startswith("#"):
            directive = stripped[1:].lstrip()
            if directive.startswith(("if", "ifdef", "ifndef")):
                if guarded:
                    depth += 1
                elif _GUARD.match(line):
                    guarded, depth = True, 1
                    out.append("\n")
                    continue
            elif guarded and directive.startswith("endif"):
                depth -= 1
                if depth == 0:
                    guarded = False
                out.append("\n")
                continue
        out.append("\n" if guarded else line)
    return "".join(out)


# Fixtures whose ISA flags are supplied by a dedicated driver, not by a
# NAME.flags file that run_regression.py reads. Each entry names the driver that
# owns the ISA configuration, so the exemption is auditable and cannot grow
# silently: a new unguarded AVX fixture still fails this check.
DRIVER_OWNED = {
    "legacy_sse_native_chain.c": "check_legacy_sse_native.py (-mavx/-mavx2 -DUSE_AVX)",
    "native_packed_fp_helpers.c": "check_legacy_sse_native.py (-mavx/-mavx2 -DUSE_AVX)",
}


# Fixtures that grant the ISA in the source -- `#pragma GCC target` regions and
# `target` attributes -- because the grant is what they test: they have to
# compile under the default flags. Which function a grant reaches is no
# question for a regular expression (a prototype inside a region grants its
# definition outside it), so each such fixture is named here, and the corpus
# run, which compiles it without an -m flag, proves that its grants suffice.
# What this check verifies is that the stated reason is true and still needed:
# the source holds a grant for every ISA its intrinsics need, and the flags
# alone would not have been enough.
SOURCE_GRANTED = {
    "pragma_gcc_target.c": "#pragma GCC target regions and target attributes",
}

_PRAGMA_TARGET = re.compile(r"^[ \t]*#[ \t]*pragma[ \t]+GCC[ \t]+target\b(.*)$", re.M)
_ATTRIBUTE_TARGET = re.compile(r"\btarget[ \t]*\(((?:[ \t]*\"[^\"\n]*\"[ \t]*,?)+)[ \t]*\)")
_STRING_OR_COMMA = re.compile(r"\"([^\"\n]*)\"|(,)")
_COMMENT = re.compile(r"/\*.*?\*/|//[^\n]*", re.S)


def source_grants(text: str) -> set[str]:
    """The -m flags that the target strings of the source stand for: every
    option of every `#pragma GCC target` and `target(...)` attribute, with
    what it implies. `no-` options and `arch=`/`tune=` grant nothing here,
    and neither does a target string that a comment only quotes."""
    text = _COMMENT.sub(lambda m: "\n" * m.group(0).count("\n") or " ", text)
    granted = set()
    operands = [m.group(1) for m in _PRAGMA_TARGET.finditer(text)]
    operands += [m.group(1) for m in _ATTRIBUTE_TARGET.finditer(text)]
    for operand in operands:
        # Adjacent literals paste (`"ss" "se3"` is `ssse3`); a comma between
        # two starts a new string, as a comma inside one starts a new option.
        options = "".join("," if comma else literal
                          for literal, comma in _STRING_OR_COMMA.findall(operand))
        for option in options.split(","):
            if option and "=" not in option and not option.startswith("no-"):
                flag = "-m" + option
                granted |= {flag} | IMPLIES.get(flag, set())
    return granted


def problems_for(name: str, text: str, flags: set[str]) -> list[str]:
    """The verdict on one fixture: `text` is its source, `flags` what its
    NAME.flags file (or the default) enables."""
    text = strip_isa_guarded(text)
    needed = [(pattern, flag) for pattern, flag in REQUIRES if pattern.search(text)]
    if name not in SOURCE_GRANTED:
        return [f"{name}: uses {pattern.pattern!r} but flags {sorted(flags)} lack {flag}"
                for pattern, flag in needed if flag not in flags]
    missing = [flag for _, flag in needed if flag not in flags]
    if not missing:
        return [f"{name}: listed in SOURCE_GRANTED ({SOURCE_GRANTED[name]}) but its flags "
                f"{sorted(flags)} already enable everything it uses; drop the entry"]
    granted = source_grants(text)
    return [f"{name}: listed in SOURCE_GRANTED ({SOURCE_GRANTED[name]}) but no target "
            f"string in the source grants {flag}"
            for flag in missing if flag not in granted]


def check() -> list[str]:
    problems = []
    for src in sorted(ROOT.glob("*.c")):
        if src.name in DRIVER_OWNED:
            continue
        problems += problems_for(src.name, src.read_text(errors="replace"), flags_for(src))
    for name in sorted(SOURCE_GRANTED):
        if not (ROOT / name).exists():
            problems.append(f"{name}: listed in SOURCE_GRANTED but no such fixture")
    return problems


def self_check() -> None:
    """The rules above on inputs small enough to read, on every run: this
    check is a few regular expressions, and a rule that silently stopped
    matching would pass every fixture."""
    avx = "__m256 f(__m256 a) { return _mm256_add_ps(a, a); }\n"
    default = {"-O2"}
    name = next(iter(SOURCE_GRANTED))
    # An unlisted fixture needs the flag, however the source grants it.
    assert problems_for("x.c", avx, default)
    assert problems_for("x.c", '#pragma GCC target("avx2")\n' + avx, default)
    assert not problems_for("x.c", avx, {"-O2", "-mavx"})
    assert not problems_for("x.c", avx, {"-O2", "-mavx2"} | IMPLIES["-mavx2"])
    assert not problems_for("x.c", "#ifdef __AVX__\n" + avx + "#endif\n", default)
    # A listed one is accepted for a grant that covers the ISA it uses...
    for grant in ('#pragma GCC target("avx")\n', '#pragma GCC target "sse4.1", "avx2"\n',
                  '#  pragma GCC target ("bmi2,avx2")\n',
                  '__attribute__((target("avx2,fma"))) ', '__attribute__((target("bmi", "avx"))) '):
        assert not problems_for(name, grant + avx, default), grant
    # ... and refused for none, for one that grants something else, for one
    # that takes the ISA away, and when the flags made the entry pointless.
    for grant in ("", '#pragma GCC target("bmi2")\n', '#pragma GCC target("no-avx")\n',
                  '#pragma GCC target("arch=haswell")\n', '#pragma GCC optimize("avx")\n',
                  '/* as if it carried __attribute__((target("avx"))) */\n',
                  '// #pragma GCC target("avx")\n', '/*\n#pragma GCC target("avx")\n*/\n'):
        assert problems_for(name, grant + avx, default), grant
    assert problems_for(name, '#pragma GCC target("avx")\n' + avx, {"-O2", "-mavx"})
    wide = "__m512 g(__m512 a) { return _mm512_add_ps(a, a); }\n"
    assert problems_for(name, '#pragma GCC target("avx2")\n' + wide, default)
    assert not problems_for(name, '#pragma GCC target("avx512f")\n' + wide + avx, default)
    assert source_grants('#pragma GCC target("sse4.1", "popcnt")\n') == {"-msse4.1", "-mpopcnt"}
    assert source_grants('#pragma GCC target "ss" "se3", "av" "x"\n') == {"-mssse3", "-mavx"}
    assert source_grants('__attribute__((target("avx512bw")))') == (
        {"-mavx512bw"} | IMPLIES["-mavx512bw"])


def main() -> int:
    self_check()
    problems = check()
    for p in problems:
        print(f"ISA-FLAGS FAIL: {p}", file=sys.stderr)
    if problems:
        return 1
    print("ISA-FLAGS OK: every fixture using an ISA-gated intrinsic enables its ISA")
    return 0


if __name__ == "__main__":
    sys.exit(main())
