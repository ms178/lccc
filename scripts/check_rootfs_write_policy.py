#!/usr/bin/env python3
"""Fail CI when a tracked script can write a package payload onto the live root.

Why this gate exists
--------------------
Extracting a Debian biarch payload (``libc6-i386``, ``libc6-dev-i386``,
``lib32*``) onto ``/`` of a *usr-merged* sandbox replaces the ``/lib -> usr/lib``
and ``/lib64 -> usr/lib64`` symlinks with i386 directory trees.  Every subsequent
``execve`` then fails, because ``/lib64/ld-linux-x86-64.so.2`` is gone: the whole
session dies, including the tools needed to repair it.  This has happened in
three separate agent sessions, each time through a *different* script, and once
through an "environment recovery" script that runs first after a harness wipe.

Prose warnings did not stop it.  This gate makes the pattern a build failure.

What it enforces
----------------
1. ``dpkg-deb -x`` / ``dpkg -x`` (and ``--extract``/``-X``/``--vextract``) may
   never target ``/`` or a system directory.
2. ``dpkg -i`` / ``--install`` / ``--unpack`` / ``--configure`` (live package
   database mutation) is not allowed in tracked scripts.
3. ``dpkg --add-architecture i386`` and ``apt-get install`` of i386/multilib
   packages must go through ``scripts/ensure_i386_multilib_safe.sh``, which
   simulates and audits the transaction first and prefers an offline, jailed,
   rootless sysroot that provides the same coverage.
4. Archive grafts (``tar -C /``, ``cp -a ... /usr/``, ``rsync ... /lib``) into
   system directories are flagged: they bypass every package-manager safeguard.
5. Destructive ``rm -rf`` of system directories is flagged.

Coverage is *not* reduced: ``scripts/ensure_i386_multilib_safe.sh`` provides real
32-bit compile/link/run for C and C++ from an isolated sysroot, and
``scripts/rootfs_jail.sh`` makes any risky extraction provably unable to write to
the live root (it fails closed when user namespaces are unavailable).

Exemptions are explicit and inventoried: append
``# rootfs-write-policy: allow(<reason>)`` to the line (or the line above).  The
reason must be substantive, and every exemption is printed so a reviewer can see
the complete list instead of discovering it later.
"""

from __future__ import annotations

import argparse
import os
import re
import shlex
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

SYSTEM_DIRS = (
    "/", "/usr", "/usr/", "/lib", "/lib/", "/lib32", "/lib32/", "/lib64", "/lib64/",
    "/bin", "/bin/", "/sbin", "/sbin/", "/etc", "/etc/", "/var/lib/dpkg",
)
MULTILIB_PACKAGES = re.compile(
    r"^(libc6-i386|libc6-dev-i386|lib32[a-z0-9.+-]*|gcc-multilib|g\+\+-multilib|"
    r"gcc-[0-9]+-multilib|g\+\+-[0-9]+-multilib|linux-libc-dev:i386|libc6:i386|"
    r"libc6-dev:i386)$"
)
ALLOW_MARKER = re.compile(r"rootfs-write-policy:\s*allow\((?P<reason>[^)]{16,})\)")
SCAN_SUFFIXES = {".sh", ".bash", ".yml", ".yaml", ".py"}
SCAN_SKIP_DIRS = {".git", "target", "node_modules", "build", "dist", "__pycache__"}


@dataclass(frozen=True)
class Finding:
    path: str
    line: int
    rule: str
    text: str
    remedy: str


# Trees whose *contents* are equally load-bearing: removing /usr/lib is exactly
# as fatal as removing /lib.  /usr/local and /opt are locally administered and
# excluded so legitimate tool installs there are not flagged.
SYSTEM_SUBTREES = ("/usr/lib", "/usr/bin", "/usr/sbin", "/usr/include",
                   "/usr/share", "/usr/libexec", "/lib", "/lib32", "/lib64",
                   "/bin", "/sbin", "/etc", "/var/lib/dpkg")


def _is_system_target(token: str, *, deep: bool = False) -> bool:
    """True when a destination token denotes the live root or a system tree."""
    t = token.strip().strip("'\"")
    if t in SYSTEM_DIRS:
        return True
    if deep and t.startswith("/") and not t.startswith(("/usr/local", "/opt")):
        normalized_deep = os.path.normpath(t)
        if any(normalized_deep == d or normalized_deep.startswith(d + "/")
               for d in SYSTEM_SUBTREES):
            return True
    # /usr/. and /lib/../ collapse onto system trees just as effectively.
    normalized = os.path.normpath(t) if t.startswith("/") else t
    return normalized in SYSTEM_DIRS or normalized.rstrip("/") in {
        d.rstrip("/") for d in SYSTEM_DIRS if d != "/"
    }


def _commands(line: str) -> list[list[str]]:
    """Tokenize the shell-ish command segments of one source line."""
    out: list[list[str]] = []
    for segment in re.split(r"&&|\|\||[|;]|\$\(", line):
        segment = segment.strip()
        if not segment:
            continue
        try:
            tokens = shlex.split(segment, comments=True)
        except ValueError:
            continue
        # Drop leading env assignments, shell keywords and privilege wrappers
        # *including their own options*.  `sudo -n dpkg-deb -x ... /` is the
        # exact command that destroyed a sandbox; a naive stripper that only
        # removed the word "sudo" left "-n" as the program name and missed it.
        wrappers = {"sudo", "command", "exec", "env", "then", "do", "if", "!",
                    "time", "nohup", "xargs", "setsid", "unshare", "timeout"}
        options_with_argument = {"-u", "-g", "-C", "-p", "-h", "--user",
                                 "--group", "--close-from", "--prompt", "--host"}
        while tokens:
            head = tokens[0]
            if re.match(r"^[A-Za-z_][A-Za-z0-9_]*=", head) or head in wrappers:
                tokens.pop(0)
                continue
            if head in options_with_argument:
                del tokens[0:min(2, len(tokens))]
                continue
            if head.startswith("-") and not head.startswith("--"):
                tokens.pop(0)
                continue
            if head == "--":
                tokens.pop(0)
                continue
            break
        if tokens:
            out.append(tokens)
    return out


def scan_line(path: str, lineno: int, line: str) -> list[Finding]:
    stripped = line.strip()
    if not stripped or stripped.startswith("#"):
        return []
    findings: list[Finding] = []

    def add(rule: str, remedy: str) -> None:
        findings.append(Finding(path, lineno, rule, stripped[:160], remedy))

    for tokens in _commands(line):
        prog = os.path.basename(tokens[0])
        args = tokens[1:]
        if prog in {"dpkg-deb", "dpkg"}:
            extract = {"-x", "--extract", "-X", "--vextract", "--fsys-tarfile"}
            if any(a in extract for a in args) or any(
                a.startswith("-") and not a.startswith("--") and "x" in a for a in args
            ):
                dest = args[-1] if args else ""
                if _is_system_target(dest) or (
                    len(args) == 1 and any(a in extract for a in args)
                ):
                    add("live-root-extraction",
                        "extract into an isolated sysroot via "
                        "scripts/ensure_i386_multilib_safe.sh, or run the command "
                        "through scripts/rootfs_jail.sh")
            if {"-i", "--install", "--unpack", "--configure"} & set(args):
                add("live-dpkg-mutation",
                    "never mutate the live package database from a tracked "
                    "script; stage into a sysroot instead")
            if "--add-architecture" in args:
                add("foreign-architecture",
                    "route i386 provisioning through "
                    "scripts/ensure_i386_multilib_safe.sh (offline sysroot first, "
                    "audited apt only as a fallback)")
        if prog in {"apt-get", "apt", "aptitude"}:
            if any(a in {"install", "reinstall", "upgrade", "dist-upgrade",
                         "full-upgrade"} for a in args):
                if any(MULTILIB_PACKAGES.match(a) for a in args):
                    add("unaudited-multilib-install",
                        "use scripts/ensure_i386_multilib_safe.sh, which audits "
                        "the transaction and refuses host-libc changes")
        if prog == "tar":
            for i, a in enumerate(args):
                if a in {"-C", "--directory"} and i + 1 < len(args):
                    if _is_system_target(args[i + 1]):
                        add("archive-graft-to-system-dir",
                            "extract into a staging directory and use an isolated "
                            "sysroot / jail instead of grafting onto /")
                if re.match(r"^-[^-]*C", a) and not a.startswith("--"):
                    tail = a.split("C", 1)[1]
                    if tail and _is_system_target(tail):
                        add("archive-graft-to-system-dir",
                            "extract into a staging directory, never onto /")
        if prog in {"cp", "rsync", "mv", "install"} and args:
            # `install -d` / `mkdir` create an empty directory; they graft no
            # payload and cannot replace a loader symlink.
            directory_only = prog == "install" and any(
                a in {"-d", "--directory"} for a in args)
            if not directory_only and _is_system_target(args[-1], deep=True):
                add("graft-to-system-dir",
                    "copy into an isolated sysroot; if a live graft is genuinely "
                    "required, enforce an allowlist of payload paths first and "
                    "annotate the exemption")
        if prog == "rm" and any(a in {"-rf", "-fr", "--recursive"} or
                                (a.startswith("-") and "r" in a and "f" in a)
                                for a in args):
            if any(_is_system_target(a, deep=True) for a in args
                   if not a.startswith("-")):
                add("destructive-system-rm", "never remove system directories")
    return findings


def iter_source_lines(text: str):
    for i, line in enumerate(text.splitlines(), start=1):
        yield i, line


def scan_text(path: str, text: str) -> tuple[list[Finding], list[tuple[str, int, str]]]:
    findings: list[Finding] = []
    exemptions: list[tuple[str, int, str]] = []
    lines = text.splitlines()
    for lineno, line in iter_source_lines(text):
        line_findings = scan_line(path, lineno, line)
        if not line_findings:
            continue
        context = line
        if lineno >= 2:
            context += "\n" + lines[lineno - 2]
        marker = ALLOW_MARKER.search(context)
        if marker:
            exemptions.append((path, lineno, marker.group("reason").strip()))
            continue
        findings.extend(line_findings)
    return findings, exemptions


def tracked_files() -> list[Path]:
    try:
        out = subprocess.run(["git", "ls-files", "-z"], cwd=REPO_ROOT,
                             capture_output=True, text=True, check=True).stdout
        names = [n for n in out.split("\0") if n]
    except (OSError, subprocess.CalledProcessError):
        names = []
        for root, dirs, files in os.walk(REPO_ROOT):
            dirs[:] = [d for d in dirs if d not in SCAN_SKIP_DIRS]
            for f in files:
                names.append(os.path.relpath(os.path.join(root, f), REPO_ROOT))
    result = []
    for name in names:
        p = REPO_ROOT / name
        if p.suffix in SCAN_SUFFIXES and p.is_file():
            result.append(p)
    return sorted(result)


SELF_TEST_CASES: list[tuple[str, str, bool]] = [
    # (description, source line, must_flag)
    ("catastrophe: biarch extraction onto /",
     'dpkg-deb -x "$D"/libc6-i386_2.36_amd64.deb /', True),
    ("catastrophe with sudo -n (the real historical command)",
     'sudo -n dpkg-deb -x "$D"/libc6-i386_2.36-9+deb12u14_amd64.deb /', True),
    ("sudo with an explicit user",
     'sudo -u root dpkg -i libc6-i386.deb', True),
    ("env wrapper with assignment",
     'env FOO=1 dpkg-deb -x pkg.deb /', True),
    ("catastrophe via sudo and && chain",
     'sudo -n true && sudo dpkg-deb -x "$deb" /', True),
    ("long-form extract onto /",
     'dpkg-deb --extract pkg.deb /', True),
    ("extract onto /usr",
     'dpkg-deb -x pkg.deb /usr', True),
    ("extract onto /lib64",
     'dpkg -x pkg.deb /lib64/', True),
    ("live dpkg install",
     'sudo dpkg -i libc6-i386.deb', True),
    ("live dpkg unpack",
     'dpkg --unpack libc6-i386.deb', True),
    ("foreign architecture",
     'sudo dpkg --add-architecture i386', True),
    ("unaudited multilib install",
     'sudo apt-get install -y -qq libc6-dev-i386 gcc-multilib lib32gcc-14-dev', True),
    ("unaudited foreign libc dev",
     'apt-get install -y linux-libc-dev:i386', True),
    ("tar graft onto root",
     'tar -xf payload.tar -C /', True),
    ("tar graft combined flag",
     'tar -xzf payload.tar.gz -C/usr', True),
    ("cp graft onto /usr",
     'sudo cp -a "$work/usr/." /usr/', True),
    ("rsync graft onto /lib",
     'rsync -a staging/ /lib/', True),
    ("destructive rm",
     'rm -rf /usr/lib', True),
    # Negative controls: the safe replacements must never be flagged.
    ("isolated sysroot extraction",
     'jail_run -- dpkg-deb -x "$deb" "$sysroot"', False),
    ("staging dir extraction",
     'dpkg-deb -x "$deb" "$staging/root"', False),
    ("tar into staging",
     'tar -xf payload.tar -C "$work"', False),
    ("cp into sysroot",
     'cp -a "$work/usr/." "$sysroot/usr/"', False),
    ("non-multilib install",
     'sudo apt-get install -y python3-yaml autoconf automake', False),
    ("safe provisioner invocation",
     'bash scripts/ensure_i386_multilib_safe.sh --offline-debs "$cache/debs"', False),
    ("comment mentioning the pattern",
     '# dpkg-deb -x libc6-i386.deb /   # historically fatal', False),
    ("rm inside a scratch dir",
     'rm -rf "$tmp/staging"', False),
    ("mkdir-style install -d",
     'sudo install -d -m 0755 /etc/apt/keyrings', False),
    ("rm of a deep system tree",
     'rm -rf /usr/lib/x86_64-linux-gnu', True),
    ("locally administered prefix is allowed",
     'cp -a payload/. /usr/local/', False),
]


def run_self_test() -> int:
    failures = 0
    for desc, line, must_flag in SELF_TEST_CASES:
        got = bool(scan_line("self-test.sh", 1, line))
        if got != must_flag:
            print(f"FAIL: {desc}: expected flag={must_flag}, got {got}\n  {line}")
            failures += 1
    # Exemption mechanism: marker on the same line and on the previous line.
    for text in (
        'dpkg-deb -x p.deb / # rootfs-write-policy: allow(disposable CI container, audited)\n',
        '# rootfs-write-policy: allow(disposable CI container, audited)\ndpkg-deb -x p.deb /\n',
    ):
        findings, exemptions = scan_text("self-test.sh", text)
        if findings or len(exemptions) != 1:
            print(f"FAIL: exemption mechanism: {findings} {exemptions}")
            failures += 1
    # A bare marker without a substantive reason must not exempt anything.
    findings, _ = scan_text("self-test.sh",
                            'dpkg-deb -x p.deb / # rootfs-write-policy: allow(bad)\n')
    if not findings:
        print("FAIL: a short/empty exemption reason was accepted")
        failures += 1
    total = len(SELF_TEST_CASES) + 3
    if failures:
        print(f"rootfs-write-policy self-test FAILED ({failures}/{total})")
        return 1
    print(f"rootfs-write-policy self-test PASSED ({total} vectors, "
          f"{sum(1 for _, _, f in SELF_TEST_CASES if f)} positive / "
          f"{sum(1 for _, _, f in SELF_TEST_CASES if not f)} negative)")
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true",
                        help="prove every detector fires and safe forms do not")
    parser.add_argument("--root", type=Path, default=REPO_ROOT)
    args = parser.parse_args(argv)

    if args.self_test:
        return run_self_test()

    all_findings: list[Finding] = []
    all_exemptions: list[tuple[str, int, str]] = []
    files = tracked_files()
    for path in files:
        try:
            text = path.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError):
            continue
        findings, exemptions = scan_text(str(path.relative_to(REPO_ROOT)), text)
        all_findings.extend(findings)
        all_exemptions.extend(exemptions)

    print(f"rootfs-write-policy: scanned {len(files)} tracked source files")
    if all_exemptions:
        print(f"  inventoried exemptions ({len(all_exemptions)}):")
        for path, lineno, reason in all_exemptions:
            print(f"    {path}:{lineno}: {reason}")
    if all_findings:
        print(f"\nFAILED: {len(all_findings)} live-root write policy violation(s):")
        for f in all_findings:
            print(f"  {f.path}:{f.line}: [{f.rule}] {f.text}")
            print(f"      remedy: {f.remedy}")
        print("\nSee docs/I386-MULTILIB-SAFETY.md.  If the write is genuinely "
              "required in a disposable container, annotate it with\n"
              "  # rootfs-write-policy: allow(<substantive reason>)")
        return 1
    print("rootfs-write-policy PASS: no unguarded live-root payload writes")
    return 0


if __name__ == "__main__":
    sys.exit(main())
