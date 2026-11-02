#!/usr/bin/env python3
"""Reproducible Compiler Explorer oracle for LCCC code-generation research.

The module API is intentionally small so other tools can reuse it::

    compile_on_godbolt(compiler_id, source, flags, intel=False)
    list_compilers("c")

CLI examples::

    scripts/godbolt.py audit                     # are the pinned oracles still current?
    scripts/godbolt.py list --filter 'gcc 16.2|icx|icc'
    scripts/godbolt.py compile gcc16.2 kernel.c --flags '-O3 -march=raptorlake'
    scripts/godbolt.py compare kernel.c --local target/fastbuild/lccc \
        --oracles gcc16.2,clang,icc,icx --function hot_loop \
        --artifact-dir results/hot-loop --json results/hot-loop/manifest.json

Aliases are pinned where a fixed release exists. ``icx`` deliberately resolves
through Compiler Explorer's ``cicxlatest`` channel; every comparison manifest
records the resolved id/name. These identify a channel/version label, not a
compiler binary; a moving channel may change between runs.  Assembly is requested in AT&T syntax to match LCCC and permit
operand-side load/store analysis in codegen_oracle.py (compare and --rank).
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shlex
import shutil
import subprocess
import tempfile
import sys
import threading
import time
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any

API = os.environ.get("GODBOLT_API", "https://godbolt.org/api").rstrip("/")
USER_AGENT = "lccc-codegen-research/2 (+https://github.com/ms178/lccc)"

# Where records live, and how they are written, is now one module's job:
# scripts/godbolt_cache.py is shared with tools/oracle/godbolt_oracle.py,
# scripts/encdiff.py and scripts/codegen_oracle.py so that all four tools hit
# the network once per tuple instead of once per tool. `CACHE` stays a module
# attribute because existing callers address it as `godbolt.CACHE`.
sys.path.insert(0, str(Path(__file__).resolve().parent))
import godbolt_cache  # noqa: E402
CACHE = godbolt_cache.CACHE

# Required competition set. Keep these ids reviewable and deterministic.
# ICC Classic ended at 2021.10.0; ICX remains a moving channel and is therefore
# identified by resolved API metadata in every artifact manifest.
COMPILER_ALIASES: dict[str, str] = {
    "gcc": "cg162",
    "gcc16.2": "cg162",
    "gcc-trunk": "cgsnapshot",
    # Clang 23.1.0 is the newest x86-64 release channel on CE; 22.1 is kept so
    # a bisect across the LLVM 22 -> 23 boundary stays one flag away.
    "clang": "cclang2310",
    "clang23": "cclang2310",
    "clang23.1": "cclang2310",
    "clang22.1": "cclang2210",
    "icc": "cicc2021100",
    "icc2021.10": "cicc2021100",
    "icx": "cicxlatest",
    "icx-latest": "cicxlatest",
    # RISC-V (64-bits) GCC channels on CE. "latest-toolchains-only" policy:
    # the trunk channel plus the two newest releases are kept as oracles.
    "crv64gtrunk": "rv64-cgcctrunk",
    "crv64g1610": "rv64-cgcc1610",
    "crv64g1520": "rv64-cgcc1520",
    "crv64g1430": "rv64-cgcc1430",
    # RISC-V (32-bits) trunk, for the LP64/ILP32 cross-checks.
    "crv32gtrunk": "rv32-cgcctrunk",
    # Native AArch64 and RISC-V 64 channels.  These exist because the
    # host-architecture channels cannot answer a cross-architecture question by
    # themselves: an x86 oracle id under `--arch aarch64` used to be accepted
    # and produced x86 assembly labelled aarch64 (the arch guard in
    # scripts/codegen_oracle.py now refuses that), and the per-architecture
    # oracle sets had no way to age -- `crv64g*` were dead ids, so every
    # RISC-V survey reported "no oracles" on stderr and an empty table.
    # Aliases route them through the same audit as the x86 pins.
    "arm64gcc": "carm64g1610",
    "arm64gcc-trunk": "carm64gtrunk",
    "armv8clang": "armv8-cclang2310",
    "rv64gcc": "rv64-cgcc1610",
    "rv64gcc-trunk": "rv64-cgcctrunk",
    "rv64clang": "rv64-cclang2310",
}
DEFAULT_ORACLES = ("gcc16.2", "clang", "icc", "icx")

# Aliases that are supposed to point at the newest available RELEASE of a
# family (moving channels such as icx-latest / gcc-trunk are excluded: they
# resolve server-side and are recorded per-run in every manifest instead).
# `godbolt.py audit` re-derives the newest release from the live API and fails
# if one of these has gone stale, so the oracle set cannot silently rot -- the
# clang pin sat two releases behind before this check existed.
PINNED_LATEST: dict[str, str] = {
    "gcc": r"^x86-64 gcc (\d+\.\d+)$",
    "clang": r"^x86-64 clang (\d+\.\d+\.\d+)$",
    "icc": r"^x86-64 icc (\d+\.\d+\.\d+)$",
    # The other architectures the compiler covers (x86-64, i686, AArch64,
    # RISC-V): a cross-architecture claim is only as good as the freshness of
    # the channel behind it, so those pins are audited the same way.
    "arm64gcc": r"^ARM64 GCC (\d+\.\d+\.\d+)$",
    "armv8clang": r"^armv8-a clang (\d+\.\d+\.\d+)$",
    "rv64gcc": r"^RISC-V \(64-bits\) gcc (\d+\.\d+\.\d+)$",
    "rv64clang": r"^RISC-V rv64gc clang (\d+\.\d+\.\d+)$",
}


class GodboltError(RuntimeError):
    """Compiler Explorer request or compilation failure."""


def _request_json(url: str, *, body: dict[str, Any] | None = None,
                  timeout: int = 120, retries: int = 3) -> Any:
    data = json.dumps(body).encode() if body is not None else None
    headers = {"Accept": "application/json", "User-Agent": USER_AGENT}
    if body is not None:
        headers["Content-Type"] = "application/json"
    last: Exception | None = None
    for attempt in range(retries):
        try:
            req = urllib.request.Request(url, data=data, headers=headers)
            with urllib.request.urlopen(req, timeout=timeout) as response:
                return json.load(response)
        except (urllib.error.HTTPError, urllib.error.URLError, TimeoutError,
                json.JSONDecodeError) as error:
            last = error
            # 4xx other than rate limiting is deterministic; retrying only
            # wastes API capacity and obscures the useful response body.
            if isinstance(error, urllib.error.HTTPError) and error.code < 500 \
                    and error.code != 429:
                detail = error.read().decode("utf-8", "replace")[:2000]
                raise GodboltError(f"HTTP {error.code}: {detail}") from error
            if attempt + 1 < retries:
                time.sleep(0.5 * (2 ** attempt))
    raise GodboltError(f"Compiler Explorer request failed after {retries} attempts: {last}")


def list_compilers(language: str = "c", *, refresh: bool = False) -> list[dict[str, Any]]:
    """Return CE compiler metadata, cached briefly for reproducible alias lookup."""
    cache = CACHE / f"compilers-{language}.json"
    CACHE.mkdir(parents=True, exist_ok=True)
    if cache.exists() and not refresh and time.time() - cache.stat().st_mtime < 86400:
        try:
            value = json.loads(cache.read_text())
            if isinstance(value, list):
                return value
        except (OSError, json.JSONDecodeError):
            pass
    value = _request_json(f"{API}/compilers/{language}", timeout=60)
    if not isinstance(value, list):
        raise GodboltError("unexpected compiler-list response")
    # Metadata lookup runs on the survey thread pool too. PID alone lets two
    # cold readers overwrite/rename the same staging file. Match the shared
    # payload cache's per-process, per-thread atomic-publication discipline.
    tmp = cache.with_suffix(f".tmp.{os.getpid()}.{threading.get_ident()}")
    tmp.write_text(json.dumps(value, indent=1))
    tmp.replace(cache)
    return value


def resolve_compiler(name: str) -> str:
    """Resolve a reviewable alias or pass through an explicit CE compiler id."""
    return COMPILER_ALIASES.get(name.lower(), name)


def compiler_metadata(compiler_id: str, *, compilers: list[dict[str, Any]] | None = None) -> dict[str, Any]:
    cid = resolve_compiler(compiler_id)
    entries = compilers if compilers is not None else list_compilers("c")
    found = next((item for item in entries if item.get("id") == cid), None)
    if found is None:
        return {"id": cid, "name": "unknown", "semver": "unknown"}
    return {
        "id": cid,
        "name": found.get("name", "unknown"),
        "semver": found.get("semver", "unknown"),
        "compilerType": found.get("compilerType", "unknown"),
    }


# Returned by compiler_fingerprint() when the version cannot be established
# (offline, retired id, reshaped API). A constant keeps the cache usable -- it
# just loses drift detection -- rather than turning every hit into a failure.
FINGERPRINT_UNKNOWN = "v-unknown"


# A moving channel -- gcc (trunk), icx (latest) and friends -- is rebuilt
# continuously while its reported version stays a placeholder ("(trunk)",
# "(latest)"). Keying on that string would assert a pin that does not exist,
# so its records instead roll into a fresh key slot once per window. One
# week: long enough that a corpus sweep still hits cache between runs, short
# enough that a trunk result quoted in a report is never more than a week
# stale. Release channels are unaffected -- they carry a real version and
# never expire.
MOVING_CHANNEL_TTL = 7 * 24 * 3600.0

# A real version string opens with a digit ("16.2", "23.1.0 (assertions)").
# Anything else -- "(trunk)", "(latest)", "" -- is a placeholder.
_PINNED_SEMVER = re.compile(r"^\s*\d")


def compiler_fingerprint(compiler_id: str, *, now: float | None = None) -> str:
    """Drift-sensitive identity of what ``compiler_id`` denotes *today*.

    A Compiler Explorer id is a label, not a binary. When Godbolt rebuilds
    ``cg162`` against a newer GCC -- or retires an id and repoints the alias --
    the id is unchanged while the code it emits is not. Any cache keyed on the
    id alone then keeps serving assembly produced by the *previous* build,
    which is precisely how a stale comparison table comes to claim a win that
    nobody ever measured against the compiler named in its header.

    Folding this value into the key turns that silent lie into an ordinary
    cache miss. Two shapes come back:

    ``"<name>|<semver>"``
        A release channel. The reported version is keyed and never expires.
        Metadata changes invalidate old records; a rebuild that keeps the
        same name/version is NOT detected. This is not a binary-build pin.

    ``"moving|<name>|<window>"``
        A moving channel, whose version string is a placeholder that never
        changes however often the build does. Pinning on it would be a lie,
        so the window counter rolls over every MOVING_CHANNEL_TTL seconds and
        the record is quietly re-fetched instead. Changes within that window
        are not detected. Use a cold cache plus explicit version probes for
        fresh evidence; even a version string is not a compiler-binary hash.

    Never raises. An offline run or an unrecognised id degrades to
    FINGERPRINT_UNKNOWN, so the cache keeps working and simply loses its
    drift detection for that run.
    """
    try:
        meta = compiler_metadata(compiler_id)
    except Exception:  # network down, id retired, API reshaped -- all non-fatal
        return FINGERPRINT_UNKNOWN
    name = str(meta.get("name") or "").strip()
    semver = str(meta.get("semver") or "").strip()
    # compiler_metadata() substitutes the literal string "unknown" for both
    # fields when the id is absent; normalise that to the sentinel so that
    # "we could not tell" is one value, not several.
    parts = [x for x in (name, semver) if x and x != "unknown"]
    if not parts:
        return FINGERPRINT_UNKNOWN
    if not semver or not _PINNED_SEMVER.match(semver):
        stamp = time.time() if now is None else now
        window = int(stamp // MOVING_CHANNEL_TTL)
        return f"moving|{name}|{window}"
    return "|".join(parts)


def compilation_error(data: Any) -> str | None:
    """Validate a compiler response, never the executed program's exit status.

    Partial assembly and an exit-zero driver that ignored target options are
    not matched-target evidence. Ordinary source warnings remain admissible.
    """
    if not isinstance(data, dict):
        return "malformed compiler response"
    code = data.get("code")
    if type(code) is not int or code != 0:
        diagnostics = data.get("stderr") or data.get("stdout") or ""
        if isinstance(diagnostics, list):
            diagnostics = "\n".join(str(x.get("text", "")) if isinstance(x, dict) else str(x)
                                    for x in diagnostics)
        return f"compilation exit missing or unsuccessful: {code!r}: {str(diagnostics)[:500]}"
    if data.get("timedOut") or data.get("truncated"):
        return "compiler resource evidence incomplete"
    for stream in (data.get("stderr"), data.get("stdout")):
        if isinstance(stream, str):
            lines = stream.splitlines()
        elif isinstance(stream, list):
            lines = [str(item.get("text", "")) if isinstance(item, dict) else str(item)
                     for item in stream]
        else:
            lines = []
        for line in lines:
            if re.search(r"\b(?:warning|error)\b", line, re.I) and re.search(
                r"(?:ignoring (?:unknown )?(?:option|argument)|"
                r"(?:unknown|unsupported|unrecogni[sz]ed) (?:command.line )?(?:option|argument|architecture)|"
                r"argument unused during compilation|"
                r"(?:option|argument|flag|architecture).*?(?:ignored|not supported|unsupported|not implemented))",
                line, re.I,
            ):
                return f"requested compiler options not honored: {line[:240]}"
    return None


def compiler_flags(compiler_id: str, flags: str) -> str:
    """Translate explicit Haswell, contraction-off and -fno-fast-math for classic ICC.

    This is option admission, not a claim of cross-vendor FP equivalence.
    Requested/effective flags are both retained. Arbitrary CPU names and
    contraction modes are NOT guessed. Ambiguous override sets fail closed.
    """
    cid = resolve_compiler(compiler_id)
    tokens = shlex.split(flags)
    if not re.fullmatch(r"cicc[0-9]+", cid):
        return flags
    changed = False
    if "-march=haswell" in tokens:
        if sum(t.startswith(("-march=", "-x", "-ax")) for t in tokens) != 1:
            raise GodboltError("ambiguous ICC target options; refusing profile translation")
        tokens = ["-xCORE-AVX2" if t == "-march=haswell" else t for t in tokens]
        changed = True
    if "-ffp-contract=off" in tokens:
        if any((t.startswith("-ffp-contract=") and t != "-ffp-contract=off")
               or t in ("-fma", "-ffast-math", "-Ofast", "-funsafe-math-optimizations")
               or t.startswith("-fp-model") for t in tokens):
            raise GodboltError("ambiguous ICC FP options; refusing contraction translation")
        tokens = ["-no-fma" if t == "-ffp-contract=off" else t for t in tokens]
        changed = True
    # ICC classic rejects -fno-fast-math instead of honoring it. Its
    # documented value-safe dialect is -fp-model precise. Contraction remains
    # a separate caller option; this is not a strict-FENV claim.
    if "-fno-fast-math" in tokens:
        if any(t in ("-ffast-math", "-Ofast", "-funsafe-math-optimizations")
               or t.startswith("-fp-model") for t in tokens):
            raise GodboltError("ambiguous ICC FP options; refusing value-safety translation")
        expanded = []
        for t in tokens:
            expanded.extend(["-fp-model", "precise"] if t == "-fno-fast-math" else [t])
        tokens = expanded
        changed = True
    return shlex.join(tokens) if changed else flags


def compile_on_godbolt(compiler_id: str, source: str, flags: str,
                        *, intel: bool = False, timeout: int = 120,
                        keep_directives: bool = False,
                        execute: bool = False,
                        raise_on_error: bool = False) -> dict[str, Any] | None:
    """Compile C source on CE and return its JSON result.

    ``None`` is retained for backward compatibility with the original helper;
    diagnostics are emitted to stderr. ``raise_on_error`` preserves the exact
    failure in a GodboltError for structured report producers instead of losing
    it behind None. Failed/ignored-option responses remain uncacheable.

    ``keep_directives`` keeps assembler directives (``.p2align``/``.p2alignr``
    and friends) in the returned ``asm`` lines; the default follows the
    research-UI convention and filters them out.

    ``execute`` asks the same endpoint to RUN the program; the result then also
    carries ``execResult`` (``code``/``stdout``/``stderr``) from the executor
    pool.  The executor is a shared, sandboxed VM: it is a correctness oracle,
    never a performance one.
    """
    cid = resolve_compiler(compiler_id)
    effective_flags = compiler_flags(cid, flags)
    body = {
        "source": source,
        "lang": "c",
        "allowStoreCodeDebug": False,
        "options": {
            "userArguments": effective_flags,
            "filters": {
                "binary": False,
                "binaryObject": False,
                # `execute` makes CE run the program and return `execResult`
                # (stdout/stderr/exit code).  It is the only way to compare
                # SEMANTICS on a vendor whose instruction shape merely looks
                # consistent -- see engineering/FOLLOWUP-2026-10-07-scale-ring-wrap-audit.md.
                "execute": execute,
                "intel": intel,
                "demangle": True,
                # CE filter booleans are REMOVE switches: true hides the
                # category. Directive alignment lines are data for
                # alignment-policy research.
                "directives": not keep_directives,
                "labels": True,
                "commentOnly": True,
                "trim": True,
                "libraryCode": False,
            },
            "compilerOptions": {"executorRequest": False, "skipAsm": False},
            "tools": [],
            "libraries": [],
        },
    }
    try:
        data = _request_json(f"{API}/compiler/{cid}/compile", body=body, timeout=timeout)
    except GodboltError as error:
        if raise_on_error:
            raise
        print(f"godbolt: {cid}: {error}", file=sys.stderr)
        return None
    # Executor responses may put the PROGRAM status at the top level.
    build = data.get("buildResult", data) if isinstance(data, dict) else data
    error = compilation_error(build)
    if error:
        if raise_on_error:
            raise GodboltError(f"{cid}: {error}")
        print(f"godbolt: {cid}: {error}", file=sys.stderr)
        return None
    data["lcccOracleOptions"] = {"requested": flags, "effective": effective_flags}
    return data


def assembly_lines(result: dict[str, Any]) -> list[str]:
    return [item.get("text", "") for item in (result.get("asm") or [])]


_OPTIMIZED_FUNCTION_SUFFIX = re.compile(
    r"^(?:constprop|isra|part|cold|llvm\.[A-Za-z0-9_.-]+)(?:\.\d+)*$"
)


def _label_is_function(wanted: str, actual: str) -> bool:
    """Match compiler-created clone suffixes without matching a new symbol.

    GCC routinely turns ``foo`` into ``foo.constprop.0``/``foo.isra.0``.
    Treating that as an absent function silently produced a zero-instruction
    oracle row, which is worse than a hard failure because it can make a bad
    compiler look optimal.  Only the documented optimization suffix families
    are accepted; ``foo_helper`` remains a different function.
    """
    if actual == wanted:
        return True
    prefix = wanted + "."
    return actual.startswith(prefix) and bool(
        _OPTIMIZED_FUNCTION_SUFFIX.fullmatch(actual[len(prefix):])
    )


def _function_body(lines: list[str], wanted: str | None) -> list[str] | None:
    if not wanted:
        return lines
    # Handles foo:, foo.constprop.0:, "foo": (GCC CE output), and comments.
    label = re.compile(r'^\s*"?([^"\s:]+)"?:\s*(?:[#;].*)?$')
    out: list[str] = []
    active = False
    for line in lines:
        match = label.match(line)
        if match and not match.group(1).startswith("."):
            name = match.group(1)
            if active:
                # ICC labels its entry block `<function>.:` -- a trailing dot,
                # which is not a valid C identifier, so it can never name a
                # distinct symbol.  Breaking there truncated `word_pun` to one
                # instruction and reported icc=1 against clang=9: a bogus
                # "optimal" oracle row, which is the exact failure mode
                # _label_is_function exists to prevent.  GCC's clone suffixes
                # (`foo.constprop.0`) are the same family of same-function
                # label; only a genuinely different symbol ends the body.
                if not (name == wanted or name.startswith(wanted + ".")):
                    break
            else:
                active = _label_is_function(wanted, name)
        if active and line.strip().startswith(".size "):
            break
        if active and line.strip().startswith(".cfi_endproc"):
            continue
        if active:
            out.append(line)
    return out if active else None


def _instruction_count(lines: list[str]) -> int:
    count = 0
    for line in lines:
        text = line.strip()
        if not text or text.startswith((".", "#", "//")) or text.endswith(":"):
            continue
        if re.match(r'^"?[^"\s:]+"?:', text):
            continue
        count += 1
    return count


def _compile_local(executable: str, source: Path, flags: str) -> list[str]:
    # A PID/digest name still collides between threads and can be pre-created
    # as a symlink. Own a private directory and clean it on every failure path.
    with tempfile.TemporaryDirectory(prefix="lccc-godbolt-") as td:
        output = Path(td) / "output.s"
        try:
            command = [executable, *shlex.split(flags), "-S", str(source), "-o", str(output)]
            result = subprocess.run(command, capture_output=True, text=True, timeout=120)
            error = compilation_error({"code": result.returncode, "stdout": result.stdout,
                                       "stderr": result.stderr})
            if error:
                raise GodboltError(f"local compile failed: {' '.join(command)}\n{error}")
            text = output.read_text(errors="replace")
            if not text.strip():
                raise GodboltError("local compiler produced empty assembly")
            return text.splitlines()
        except (OSError, ValueError, subprocess.TimeoutExpired) as error:
            raise GodboltError(f"local compile failed: {error}") from error


# ---------------------------------------------------------------------------
# Local oracles: compilers that run on this host, not on Compiler Explorer.
#
# Local release-line channels are OPT-IN, never DEFAULT_ORACLES. A fresh
# prefix selects the newest authenticated Intel APT build in that line;
# manifests and measurement records retain the actual patch/build identity.
LOCAL_ORACLES: dict[str, dict[str, Any]] = {
    "icx2026.2": {
        "name": "Intel oneAPI DPC++/C++ 2026.2.x (local)",
        "release_line": "2026.2",
        "compilerType": "local-icx",
        "env": "LCCC_ICX_BIN",
        "provision": "ensure_icx_2026.sh",
        "probe_macro": "__INTEL_LLVM_COMPILER",
    },
}
_LOCAL_BINARIES: dict[str, tuple[tuple[str | None, ...], Path, tuple[int, ...], str]] = {}
_LOCAL_LOCK = threading.Lock()


def _probe_macros(binary: str) -> dict[str, str]:
    """Probe release and effective native data model in one compiler process."""
    result = subprocess.run(
        [binary, "-E", "-dM", "-x", "c", os.devnull],
        capture_output=True, text=True, timeout=60,
    )
    if result.returncode:
        raise GodboltError(f"compiler macro probe failed: {result.stderr.strip()}")
    macros = {}
    for line in result.stdout.splitlines():
        parts = line.split(None, 2)
        if len(parts) == 3 and parts[0] == "#define":
            macros[parts[1]] = parts[2].strip()
    return macros


def validate_local_oracle_flags(name: str, flags: str) -> None:
    """Local ICX channels measure native x86-64 LP64, not arbitrary retargets.

    Response/config files and forwarded driver/frontend options can hide a
    target change; reject these opaque channels instead of guessing their
    meaning. Ordinary ISA tuning, optimization and language flags remain valid.
    """
    try:
        tokens = shlex.split(flags)
    except ValueError as error:
        raise GodboltError(f"{name}: invalid compiler flags: {error}") from error
    forbidden = ('-target', '--target', '-arch', '-mabi', '--config', '-config',
                 '-Xclang', '-Xarch', '-Xpreprocessor', '-cc1', '-ccc-host-triple',
                 '--gcc-toolchain', '-specs', '--specs', '-wrapper')
    for token in tokens:
        if (token in ('-m16', '-m32', '-mx32', '--') or token.startswith('@')
                or any(token == opt or token.startswith(opt + '=') for opt in forbidden)
                or token.startswith(('-B', '-Xarch_', '-Wp,', '-Xclang='))):
            raise GodboltError(f"{name}: native x86-64 LP64 oracle rejects target/opaque option {token!r}")


def _binary_identity(path: Path) -> tuple[int, ...]:
    stat = path.stat()
    return (stat.st_dev, stat.st_ino, stat.st_size, stat.st_mtime_ns, stat.st_ctime_ns)


def _local_semver(spec: dict[str, Any], probed: str | None) -> str:
    if probed is None or not re.fullmatch(r"[0-9]{8}", probed):
        raise GodboltError(f"missing or invalid {spec['probe_macro']} probe: {probed!r}")
    value = int(probed)
    major, minor, patch = value // 10000, value // 100 % 100, value % 100
    if f"{major}.{minor}" != spec["release_line"]:
        raise GodboltError(
            f"compiler probes {spec['probe_macro']}={probed!r}, "
            f"expected release line {spec['release_line']}"
        )
    return f"{major}.{minor}.{patch}"


def _validated_local_oracle(name: str) -> tuple[Path, str]:
    spec = LOCAL_ORACLES[name]
    with _LOCAL_LOCK:
        explicit = os.environ.get(spec["env"])
        context = (explicit, os.environ.get("LCCC_ICX_PREFIX"), os.environ.get("LCCC_ICX_MIRROR"))
        cached = _LOCAL_BINARIES.get(name)
        try:
            binary = None
            if explicit is not None:
                if not explicit.strip():
                    raise GodboltError(f"{spec['env']} must not be empty")
                binary = shutil.which(os.path.expanduser(explicit))
                if binary is None:
                    raise GodboltError(f"{spec['env']} is not an executable: {explicit}")
            if cached is not None and cached[0] == context and explicit is None:
                _, path, identity, _ = cached
                if _binary_identity(path) == identity:
                    binary = str(path)
            # Always reprobe: some filesystems have coarse timestamps, so
            # stat identity alone cannot prove an executable is unchanged.
            if binary is None:
                provisioner = Path(__file__).resolve().parent / spec["provision"]
                result = subprocess.run([str(provisioner)], capture_output=True, text=True, timeout=3600)
                if result.returncode:
                    raise GodboltError(f"{name}: provisioning failed: {result.stderr.strip()}")
                lines = result.stdout.strip().splitlines()
                if len(lines) != 1:
                    raise GodboltError(f"{name}: provisioner must print exactly one binary path")
                binary = lines[0]
            path = Path(binary).resolve()
            before = _binary_identity(path)
            macros = _probe_macros(str(path))
            probed = macros.get(spec["probe_macro"])
            if macros.get('__x86_64__') != '1' or macros.get('__SIZEOF_POINTER__') != '8':
                raise GodboltError(f"{name}: compiler probe is not native x86-64 LP64")
            _local_semver(spec, probed)
            if _binary_identity(path) != before:
                raise GodboltError(f"{name}: compiler changed during version probe")
            assert probed is not None
            _LOCAL_BINARIES[name] = (context, path, before, probed)
            return path, probed
        except (OSError, subprocess.TimeoutExpired) as error:
            raise GodboltError(f"{name}: compiler resolution/probe failed: {error}") from error


def local_oracle_binary(name: str) -> Path:
    """Resolve and probe an opt-in local release line; invalidate changed binaries."""
    return _validated_local_oracle(name)[0]


def local_oracle_metadata(name: str) -> dict[str, Any]:
    """Record the actual patch release, binary digest and available APT lock."""
    spec = LOCAL_ORACLES[name]
    binary, probed = _validated_local_oracle(name)
    before = _binary_identity(binary)
    digest = hashlib.sha256()
    with binary.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    if _binary_identity(binary) != before:
        raise GodboltError(f"{name}: compiler changed while hashing provenance")
    metadata = {
        "id": name,
        "name": spec["name"],
        "semver": _local_semver(spec, probed),
        "compilerType": spec["compilerType"],
        "binary": str(binary),
        "binary_sha256": digest.hexdigest(),
        "probe": f"{spec['probe_macro']}={probed}",
        "target": "x86_64",
        "pointer_bits": 64,
    }
    for parent in binary.parents:
        manifest = parent / ".lccc-icx.json"
        if manifest.is_file():
            try:
                locked = json.loads(manifest.read_text())
                if not isinstance(locked, dict) or not all(k in locked for k in ('binary', 'binary_sha256', 'semver')):
                    raise ValueError('missing required identity fields')
            except (OSError, ValueError) as error:
                raise GodboltError(f"{name}: invalid provisioning manifest: {error}") from error
            if ((parent / locked["binary"]).resolve() != binary
                    or locked["binary_sha256"] != metadata["binary_sha256"]
                    or locked["semver"] != metadata["semver"]):
                raise GodboltError(f"{name}: provisioning manifest disagrees with compiler")
            metadata["provisioning"] = locked
            break
    return metadata


def compile_local_oracle(name: str, source: Path, flags: str) -> list[str]:
    """Assembly lines from a local oracle; never touches Compiler Explorer."""
    validate_local_oracle_flags(name, flags)
    return _compile_local(str(local_oracle_binary(name)), source, flags)


def oracle_assembly(name: str, source: Path, flags: str, *,
                    compilers: list[dict[str, Any]] | None = None,
                    intel: bool = False) -> tuple[dict[str, Any], list[str]]:
    """Shared dispatch for CLI users: local names never touch remote APIs."""
    if name in LOCAL_ORACLES:
        validate_local_oracle_flags(name, flags)
        metadata = local_oracle_metadata(name)
        effective_flags = flags + (' -masm=intel' if intel else '')
        lines = compile_local_oracle(name, source, effective_flags)
    else:
        metadata = compiler_metadata(name, compilers=compilers)
        result = compile_on_godbolt(name, source.read_text(), flags, intel=intel, raise_on_error=True)
        if result is None:
            raise GodboltError(f'{name}: missing compilation response')
        lines = assembly_lines(result)
    return metadata, lines


def command_list(args: argparse.Namespace) -> int:
    pattern = re.compile(args.filter, re.IGNORECASE) if args.filter else None
    rows = []
    local_items = [{"id": name, "name": spec['name'], "semver": spec['release_line'] + '.x'}
                   for name, spec in LOCAL_ORACLES.items()]
    remote_items = [] if args.local_only else list_compilers(args.language, refresh=args.refresh)
    for item in [*remote_items, *local_items]:
        text = f"{item.get('id', '')} {item.get('name', '')} {item.get('semver', '')}"
        if pattern and not pattern.search(text):
            continue
        rows.append(item)
    for item in rows:
        print(f"{item.get('id', ''):<24} {item.get('name', '')}")
    return 0


def command_compile(args: argparse.Namespace) -> int:
    if args.compiler in LOCAL_ORACLES:
        if args.execute:
            raise GodboltError('local oracle compile is assembly-only; --execute is unsupported')
        _, assembly = oracle_assembly(args.compiler, args.source, args.flags, intel=args.intel)
        body = _function_body(assembly, args.function)
        if body is None:
            raise GodboltError(f"function {args.function!r} was not emitted")
        print('\n'.join(body))
        return 0
    result = compile_on_godbolt(args.compiler, args.source.read_text(), args.flags,
                                intel=args.intel, execute=args.execute)
    if result is None:
        return 1
    if args.execute:
        ex = result.get("execResult") or {}
        print(f"exit {ex.get('code')}")

        def _text(chunks) -> str:
            # CE returns each stream as a list of {"text": ...} objects (older
            # API revisions used bare strings); accept both.
            return "".join(
                (c.get("text") or "") if isinstance(c, dict) else str(c) for c in chunks or []
            )

        stdout = _text(ex.get("stdout"))
        stderr = _text(ex.get("stderr"))
        if stdout:
            print(stdout, end="" if stdout.endswith("\n") else "\n")
        if stderr:
            print(stderr, file=sys.stderr, end="" if stderr.endswith("\n") else "\n")
    lines = _function_body(assembly_lines(result), args.function)
    if lines is None:
        print(f"godbolt: function '{args.function}' was not emitted (likely inlined or removed)",
              file=sys.stderr)
        return 1
    print("\n".join(lines))
    if args.execute:
        execution = result.get("execResult") or {}
        return int(execution.get("code") != 0
                   or execution.get("timedOut", False)
                   or execution.get("didExecute") is False)
    return 0


def stable_launcher_digest(path: Path) -> str:
    """Hash only the launcher and reject changes during the read.

    A wrapper's target is deliberately outside this identity scope.
    """
    try:
        before = _binary_identity(path)
        digest = hashlib.sha256()
        with path.open("rb") as source:
            for chunk in iter(lambda: source.read(1 << 20), b""):
                digest.update(chunk)
        if _binary_identity(path) != before:
            raise GodboltError(f"compiler launcher changed while hashing: {path}")
        return digest.hexdigest()
    except OSError as error:
        raise GodboltError(f"cannot hash compiler launcher {path}: {error}") from error


def local_launcher_metadata(label: str, executable: str) -> dict[str, Any]:
    """Identify the invoked launcher, not a wrapper's transitive toolchain."""
    if not executable.strip():
        raise GodboltError(f"{label}: empty local executable")
    resolved = shutil.which(os.path.expanduser(executable))
    if resolved is None:
        raise GodboltError(f"{label}: local compiler is not executable: {executable}")
    path = Path(resolved).resolve()
    try:
        before = _binary_identity(path)
        proc = subprocess.run([str(path), "--version"], capture_output=True, text=True, timeout=60)
        lines = proc.stdout.splitlines()
        if proc.returncode or not lines or not lines[0].strip():
            raise GodboltError(f"{label}: local compiler did not report a version")
        digest = stable_launcher_digest(path)
        if _binary_identity(path) != before:
            raise GodboltError(f"{label}: compiler launcher changed during identification")
        return {
            "key": label,
            "name": lines[0].strip(),
            "launcher": str(path),
            "launcher_sha256": digest,
            "identity_scope": "launcher only; wrapper targets are not pinned",
        }
    except (OSError, subprocess.TimeoutExpired) as error:
        raise GodboltError(f"{label}: local compiler probe failed: {error}") from error


def local_reference_metadata(label: str, executable: str, flags: str) -> dict[str, Any]:
    """Identify an explicit local native oracle without provisioning or network.

    The executable hash is deliberately named *launcher*: a shell shim can
    dispatch to another binary, so its digest alone does not pin the compiler.
    Macro and version probes supply evidence, not an attestation of that binary.
    """
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,63}", label):
        raise GodboltError(f"invalid local reference label {label!r}")
    validate_local_oracle_flags(label, flags)
    metadata = local_launcher_metadata(label, executable)
    path = Path(metadata["launcher"])
    try:
        before = _binary_identity(path)
        macros = _probe_macros(str(path))
        if macros.get("__x86_64__") != "1" or macros.get("__SIZEOF_POINTER__") != "8":
            raise GodboltError(f"{label}: local compiler is not native x86-64 LP64")
        if (_binary_identity(path) != before
                or stable_launcher_digest(path) != metadata["launcher_sha256"]):
            raise GodboltError(f"{label}: compiler launcher changed during identification")
        return {
            **metadata,
            "id": f"local:{label}",
            "compilerType": "explicit-local",
            "target": "x86_64",
            "pointer_bits": 64,
        }
    except (OSError, subprocess.TimeoutExpired) as error:
        raise GodboltError(f"{label}: local compiler probe failed: {error}") from error


def command_compare(args: argparse.Namespace) -> int:
    source_bytes = args.source.read_bytes()
    # Match read_text()'s universal-newline behavior for the remote request;
    # the manifest records both its digest and the local raw-byte digest.
    source_text = source_bytes.decode("utf-8").replace("\r\n", "\n").replace("\r", "\n")
    requested = [item.strip() for item in args.oracles.split(",") if item.strip()]
    if getattr(args, "offline", False) and requested:
        print("--offline requires an empty --oracles list and explicit local references", file=sys.stderr)
        return 1
    if len(requested) != len(set(requested)):
        print("duplicate remote oracle label", file=sys.stderr)
        return 1
    local_specs = getattr(args, "local_ref", [])
    local_pairs = []
    names = {"lccc", *requested}
    for spec in local_specs:
        label, separator, executable = spec.partition("=")
        if not separator or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,63}", label):
            print(f"invalid --local-ref {spec!r}; expected LABEL=EXECUTABLE", file=sys.stderr)
            return 1
        if label in names:
            print(f"duplicate comparator label: {label}", file=sys.stderr)
            return 1
        names.add(label)
        local_pairs.append((label, executable))
    if not requested and not local_pairs:
        print("compare requires at least one oracle or explicit local reference", file=sys.stderr)
        return 1

    # If the CE catalog is unavailable, preserve the local results and record
    # each remote as an error. Never turn missing ICC/ICX evidence into a pass.
    compilers: list[dict[str, Any]] = []
    catalog_error = None
    if any(name not in LOCAL_ORACLES for name in requested):
        try:
            compilers = list_compilers("c")
        except GodboltError as error:
            catalog_error = str(error)
    artifact_dir: Path | None = args.artifact_dir
    if artifact_dir:
        artifact_dir.mkdir(parents=True, exist_ok=True)

    records: list[dict[str, Any]] = []
    try:
        local_metadata = local_launcher_metadata("lccc", args.local)
        local_lines = _compile_local(local_metadata["launcher"], args.source, args.local_flags or args.flags)
        if stable_launcher_digest(Path(local_metadata["launcher"])) != local_metadata["launcher_sha256"]:
            raise GodboltError("lccc: compiler launcher changed during compilation")
        local_body = _function_body(local_lines, args.function)
        if local_body is None:
            raise GodboltError(
                f"function '{args.function}' was not emitted by local LCCC "
                "(likely inlined or removed)"
            )
        records.append({
            **local_metadata, "id": str(Path(args.local)),
            "flags": args.local_flags or args.flags,
            "instructions": _instruction_count(local_body), "assembly": local_body,
        })
    except GodboltError as error:
        print(error, file=sys.stderr)
        return 1

    for label, executable in local_pairs:
        try:
            metadata = local_reference_metadata(label, executable, args.flags)
            lines = _compile_local(metadata["launcher"], args.source, args.flags)
            if stable_launcher_digest(Path(metadata["launcher"])) != metadata["launcher_sha256"]:
                raise GodboltError(f"{label}: compiler launcher changed during compilation")
            body = _function_body(lines, args.function)
            if body is None:
                raise GodboltError(f"function '{args.function}' was not emitted (likely inlined or removed)")
            records.append({**metadata, "flags": args.flags,
                            "instructions": _instruction_count(body), "assembly": body})
        except GodboltError as error:
            records.append({"key": label, "id": f"local:{label}", "error": str(error)})

    for requested_name in requested:
        if catalog_error is not None and requested_name not in LOCAL_ORACLES:
            records.append({"key": requested_name, "id": resolve_compiler(requested_name),
                            "error": f"Compiler Explorer catalog unavailable: {catalog_error}"})
            continue
        try:
            meta, lines = oracle_assembly(requested_name, args.source, args.flags, compilers=compilers)
        except GodboltError as error:
            records.append({"key": requested_name, "id": resolve_compiler(requested_name), "error": str(error)})
            continue
        body = _function_body(lines, args.function)
        if body is None:
            records.append({
                "key": requested_name, **meta, "flags": args.flags,
                "error": f"function '{args.function}' was not emitted (likely inlined or removed)",
            })
            continue
        records.append({
            "key": requested_name, **meta, "flags": args.flags,
            "instructions": _instruction_count(body), "assembly": body,
        })

    if args.source.read_bytes() != source_bytes:
        records.append({"key": "input", "id": str(args.source),
                        "error": "source changed during comparison; results are not comparable"})

    print("Static assembly comparison only; correctness and runtime are not measured.")
    print(f"{'compiler':<14} {'resolved id':<20} {'instructions':>12}  name")
    print("-" * 82)
    for record in records:
        count = record.get("instructions")
        count_text = str(count) if count is not None else "ERROR"
        print(f"{record['key']:<14} {record['id']:<20} {count_text:>12}  {record.get('name', '')}")
        if "error" in record:
            print(f"  {record['key']}: {record['error']}", file=sys.stderr)
        if artifact_dir and "assembly" in record:
            safe = re.sub(r"[^A-Za-z0-9_.-]+", "_", record["key"])
            (artifact_dir / f"{safe}.s").write_text("\n".join(record["assembly"]) + "\n")

    manifest = {
        "schema": 1,
        "scope": "static-assembly-only",
        "source": str(args.source),
        "source_sha256": hashlib.sha256(source_bytes).hexdigest(),
        "remote_source_sha256": hashlib.sha256(source_text.encode()).hexdigest(),
        "function": args.function,
        "remote_flags": args.flags,
        "local_flags": args.local_flags or args.flags,
        "oracles": [{k: v for k, v in record.items() if k != "assembly"}
                    for record in records],
    }
    manifest_path = args.json or (artifact_dir / "manifest.json" if artifact_dir else None)
    if manifest_path:
        manifest_path.parent.mkdir(parents=True, exist_ok=True)
        tmp = manifest_path.with_suffix(f".tmp.{os.getpid()}")
        tmp.write_text(json.dumps(manifest, indent=2) + "\n")
        tmp.replace(manifest_path)
        print(f"wrote {manifest_path}")
    return 1 if any("error" in record for record in records) else 0


def _version_key(text: str) -> tuple[int, ...]:
    return tuple(int(part) for part in text.split("."))


def command_audit(args: argparse.Namespace) -> int:
    """Fail if a pinned alias no longer names the newest release of its family.

    Compiler Explorer adds releases continuously. An oracle set that is not
    re-checked drifts: `clang` sat on 22.1.0 while 23.1.0 had shipped, so every
    "we match Clang" claim was measured against a superseded compiler. This
    turns that silent rot into a non-zero exit status.
    """
    compilers = list_compilers(args.language, refresh=True)
    by_id = {c["id"]: c for c in compilers}
    problems = 0

    print(f"{'ALIAS':<14}{'PINNED ID':<18}{'PINNED':<12}{'NEWEST':<12}STATUS")
    print("-" * 62)
    for alias, pattern in PINNED_LATEST.items():
        pinned_id = COMPILER_ALIASES[alias]
        rx = re.compile(pattern)
        available: list[tuple[tuple[int, ...], str, str]] = []
        for comp in compilers:
            match = rx.match(comp.get("name", ""))
            if match:
                try:
                    available.append((_version_key(match.group(1)),
                                      match.group(1), comp["id"]))
                except ValueError:
                    continue
        if not available:
            print(f"{alias:<10}{pinned_id:<16}{'?':<12}{'?':<12}NO MATCH FOR PATTERN")
            problems += 1
            continue
        available.sort()
        newest_key, newest_ver, newest_id = available[-1]

        pinned_name = by_id.get(pinned_id, {}).get("name", "<missing>")
        pinned_match = rx.match(pinned_name)
        pinned_ver = pinned_match.group(1) if pinned_match else "?"

        if pinned_id == newest_id:
            status = "ok"
        else:
            status = f"STALE -> use {newest_id}"
            problems += 1
        print(f"{alias:<10}{pinned_id:<16}{pinned_ver:<12}{newest_ver:<12}{status}")

    # Moving channels: report what they resolve to right now so the manifest
    # reader can tell which build a past run actually used.
    print()
    for alias in ("icx", "gcc-trunk", "arm64gcc-trunk", "rv64gcc-trunk"):
        cid = COMPILER_ALIASES.get(alias)
        if cid:
            name = by_id.get(cid, {}).get("name", "<missing>")
            semver = by_id.get(cid, {}).get("semver", "")
            print(f"  moving channel {alias:<10} -> {cid:<14} {name} {semver}")

    # Every alias must at least exist, or a comparison run dies mid-way.
    print()
    for alias, cid in sorted(COMPILER_ALIASES.items()):
        if cid not in by_id:
            print(f"  BROKEN ALIAS {alias} -> {cid} (not offered by the API)")
            problems += 1

    if problems:
        print(f"\naudit: {problems} problem(s)")
        return 1
    print("\naudit: all pinned aliases resolve and are current")
    return 0



def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="command")

    list_p = sub.add_parser("list", help="list CE C compilers")
    list_p.add_argument("--language", default="c")
    list_p.add_argument("--filter", default="")
    list_p.add_argument("--refresh", action="store_true")
    list_p.add_argument("--local-only", action="store_true", help="list opt-in local channels without network/provisioning")
    list_p.set_defaults(func=command_list)

    compile_p = sub.add_parser("compile", help="compile one source with one CE compiler")
    compile_p.add_argument("compiler")
    compile_p.add_argument("source", type=Path)
    compile_p.add_argument("--flags", default="-O3 -march=x86-64-v3")
    compile_p.add_argument("--function")
    compile_p.add_argument("--intel", action="store_true")
    compile_p.add_argument("--execute", action="store_true",
                           help="run the program on CE and report stdout/exit")
    compile_p.set_defaults(func=command_compile)

    compare_p = sub.add_parser("compare", help="compare local LCCC with all competition oracles")
    compare_p.add_argument("source", type=Path)
    compare_p.add_argument("--local", default="target/fastbuild/lccc")
    compare_p.add_argument("--flags", default="-O3 -march=x86-64-v3")
    compare_p.add_argument("--local-flags")
    compare_p.add_argument("--oracles", default=",".join(DEFAULT_ORACLES))
    compare_p.add_argument("--offline", action="store_true",
                           help="refuse all oracle channels and use explicit local references only")
    compare_p.add_argument("--local-ref", action="append", default=[], metavar="LABEL=EXECUTABLE",
                           help="add a native x86-64 LP64 compiler without network; repeatable")
    compare_p.add_argument("--function")
    compare_p.add_argument("--artifact-dir", type=Path)
    compare_p.add_argument("--json", type=Path)
    compare_p.set_defaults(func=command_compare)

    audit_p = sub.add_parser(
        "audit",
        help="verify every pinned alias still resolves and names the newest release",
    )
    audit_p.add_argument("--language", default="c")
    audit_p.set_defaults(func=command_audit)
    return parser


DASH_VALUE_OPTIONS = {"--flags", "--local-flags"}


def join_dash_values(argv: list[str]) -> list[str]:
    """Join ``--flags -O3`` → ``--flags=-O3`` so dash-leading flag values parse.

    argparse treats a token beginning with ``-`` as an option, so the natural
    spelling ``godbolt.py compile gcc16.2 f.c --flags -O3`` dies with
    ``argument --flags: expected one argument``.  ``--flags=-O3`` works, but
    the spaced form is what every docstring and example in this repo shows,
    so the CLI must accept it.  Only the known value options are rewritten;
    ``--flags -O3 --function f`` must not swallow ``--function``.
    """
    out: list[str] = []
    i = 0
    while i < len(argv):
        token = argv[i]
        if token in DASH_VALUE_OPTIONS:
            # A value that starts with '-' is still a value if it is a
            # single-dash token (-O3, -march=...): this CLI has no single-dash
            # options other than -h, which never follows --flags.  Joining
            # makes both "--flags -O3" and "--flags=-O3" work.
            nxt = argv[i + 1] if i + 1 < len(argv) else None
            if nxt is not None and (not nxt.startswith("-") or re.match(r"^-[^-]", nxt)):
                out.append(f"{token}={nxt}")
                i += 2
                continue
            out.append(token)  # let argparse report the missing value
            i += 1
            continue
        out.append(token)
        i += 1
    return out


def main(argv: list[str] | None = None) -> int:
    argv = list(sys.argv[1:] if argv is None else argv)
    # Backward-compatible spelling: godbolt.py <compiler-id> <source> ...
    if argv and argv[0] not in {"list", "compile", "compare", "audit", "-h", "--help"}:
        argv.insert(0, "compile")
    parser = build_parser()
    args = parser.parse_args(join_dash_values(argv))
    if not hasattr(args, "func"):
        parser.print_help(sys.stderr)
        return 2
    try:
        return args.func(args)
    except (GodboltError, OSError, subprocess.TimeoutExpired) as error:
        print(f'godbolt: {error}', file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
