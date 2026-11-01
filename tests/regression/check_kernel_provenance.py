#!/usr/bin/env python3
"""Kernel preparation is content-addressed and never overwrites a changed recipe.

Exercises the fast path of scripts/prepare_kernel_tree.sh with a mocked make:
legacy/mismatched stamps are refused without running make, a matching stamp is
a strict no-op, a compiler-content change cleans and re-prepares while
preserving source/config provenance, and a damaged prepared tree requires an
explicit LCCC_REPREPARE=1.
"""

from __future__ import annotations

import hashlib
import os
import pathlib
import re
import shutil
import subprocess
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts/prepare_kernel_tree.sh"

CANARIES = [
    "Makefile",
    "scripts/Kbuild.include",
    "init/main.c",
    "kernel/workqueue.c",
    "arch/x86/boot/setup.ld",
    "include/generated/autoconf.h",
    "include/generated/utsversion.h",
    "arch/x86/include/generated/asm/rwonce.h",
    "arch/x86/kernel/cpu/capflags.c",
    "arch/x86/boot/zoffset.h",
    "arch/x86/boot/voffset.h",
    "arch/x86/boot/cpustr.h",
]

PREFLIGHT_TOOLS = [
    "ld", "ar", "nm", "objcopy", "perl", "awk", "sed", "bc", "flex", "bison",
    "xz", "patch", "tar", "curl", "zstd", "lz4", "lzop", "gzip", "bzip2",
]


def sha256(path: pathlib.Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(env: dict[str, str], kdir: pathlib.Path,
        expect: int = 0) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(["bash", str(SCRIPT), str(kdir)], env=env,
                            capture_output=True, text=True)
    if result.returncode != expect:
        raise AssertionError(
            f"prepare exited {result.returncode}, expected {expect}\n"
            f"stdout:\n{result.stdout}\nstderr:\n{result.stderr}"
        )
    return result


def main() -> int:
    with tempfile.TemporaryDirectory(prefix="lccc-provenance-") as td:
        tmp = pathlib.Path(td)
        pkg = tmp / "pkg"
        kdir = tmp / "linux-test"
        bindir = tmp / "bin"
        pkg.mkdir()
        bindir.mkdir()
        (pkg / "PKGBUILD").write_text(
            "_major=6\n_minor=18\npkgrel=1\n"
            "sha256sums=('" + "0" * 64 + "')\n"
            "source=(test.patch)\n"
        )
        (pkg / "config").write_text("CONFIG_TEST=y\n")
        (pkg / "test.patch").write_text("patch payload\n")

        cc = bindir / "lccc"
        cc.write_text('#!/bin/sh\necho "lccc test 1.0"\n')
        cc.chmod(0o755)
        hostcc = bindir / "hostcc"
        hostcc.write_text(
            '#!/bin/sh\nwhile [ "$#" -gt 0 ]; do\n'
            '  if [ "$1" = -o ]; then shift;\n'
            "    printf '#!/bin/sh\\necho generated-cpustr\\n' > \"$1\"; chmod +x \"$1\"; exit 0; fi\n"
            '  shift\ndone\nexit 1\n'
        )
        hostcc.chmod(0o755)
        make = bindir / "make"
        make.write_text(
            '#!/bin/sh\nprintf "%s\\n" "$*" >> "$TEST_MAKE_LOG"\n'
            'case " $* " in *" clean "*)\n'
            '  rm -f include/generated/utsversion.h arch/x86/kernel/cpu/capflags.c '
            'arch/x86/boot/cpustr.h arch/x86/boot/zoffset.h arch/x86/boot/voffset.h;;\n'
            'esac\n'
            'case " $* " in *" olddefconfig "*) [ "${TEST_FAIL_PREPARE:-0}" != 1 ] || exit 7;; esac\n'
        )
        make.chmod(0o755)
        # Mock only preflight tools that are genuinely absent so the test does
        # not require a full kernel-build host. sha256sum/sed/awk/grep must stay
        # real because the recipe hash is computed with them.
        for tool in PREFLIGHT_TOOLS:
            if shutil.which(tool) is None:
                mock = bindir / tool
                mock.write_text('#!/bin/sh\nexit 0\n')
                mock.chmod(0o755)
        if shutil.which("sha256sum") is None and shutil.which("shasum") is None:
            print("FAIL: sha256sum or shasum is required", flush=True)
            return 1

        for name in CANARIES:
            file = kdir / name
            file.parent.mkdir(parents=True, exist_ok=True)
            file.write_text("canary\n")

        (kdir / "arch/x86/kernel/cpu/mkcapflags.sh").write_text(
            '#!/bin/sh\necho generated-capflags > "$1"\n'
        )

        env = dict(
            os.environ,
            PATH=f"{bindir}:{os.environ['PATH']}",
            PKGDIR=str(pkg),
            KERNEL_WORK=str(tmp),
            LCCC=str(cc),
            LCCC_LD=str(cc),
            HOSTCC=str(hostcc),
            TEST_MAKE_LOG=str(tmp / "make.log"),
            LCCC_KERNEL_SOURCE_MODE="tarball",
        )

        (kdir / ".lccc-prepared").write_text("legacy stamp\n")
        result = run(env, kdir, expect=3)
        assert "recipe mismatch/legacy stamp" in result.stderr, result.stderr
        match = re.search(r"expected recipe=([0-9a-f]{64})", result.stderr)
        assert match, result.stderr
        recipe = match.group(1)
        assert not (tmp / "make.log").exists(), "mismatch ran make"

        version = subprocess.check_output([str(cc), "--version"], text=True).splitlines()[0]
        first = f"compiler={cc} version={version} sha256={sha256(cc)}"
        stamp = (
            first + "\nformat=2\n"
            f"recipe-sha256={recipe}\nsource=tarball\n"
            "config-input-sha256=original\n"
        )
        (kdir / ".lccc-prepared").write_text(stamp)
        result = run(env, kdir)
        assert "already prepared" in result.stdout, result.stdout
        assert not (tmp / "make.log").exists(), "no-op ran make"

        (pkg / "config").write_text("CONFIG_TEST=n\n")
        result = run(env, kdir, expect=3)
        assert "recipe mismatch" in result.stderr, result.stderr
        assert (kdir / ".lccc-prepared").read_text() == stamp

        (pkg / "config").write_text("CONFIG_TEST=y\n")
        cc.write_text('#!/bin/sh\necho "lccc test 1.0"\n# changed content\n')
        result = run(env, kdir)
        assert "compiler content changed" in result.stdout, result.stdout
        log = (tmp / "make.log").read_text()
        assert "clean" in log and "olddefconfig" in log and "prepare" in log, log
        assert f"LD={cc} AS={cc}" in log, log
        assert all((kdir / name).is_file() for name in CANARIES)
        assert "generated-cpustr" in (kdir / "arch/x86/boot/cpustr.h").read_text()
        new = (kdir / ".lccc-prepared").read_text()
        assert f"recipe-sha256={recipe}" in new
        assert "config-input-sha256=original" in new
        assert not new.startswith(first + "\n")
        result = run(env, kdir)
        assert "already prepared" in result.stdout, result.stdout

        # A damaged prepared tree is not silently replaced.
        (kdir / CANARIES[-1]).unlink()
        result = run(env, kdir, expect=3)
        assert "LCCC_REPREPARE=1" in result.stderr, result.stderr

        # Neither Kconfig failure nor a failed boot host generator may publish
        # the new compiler identity. Clean really removes the boot headers.
        (kdir / CANARIES[-1]).write_text("canary\n")
        cc.write_text(cc.read_text() + "# another compiler change\n")
        run(dict(env, TEST_FAIL_PREPARE="1"), kdir, expect=7)
        assert (kdir / ".lccc-prepared").read_text() == new
        for name in CANARIES:
            (kdir / name).write_text("canary\n")
        hostcc.write_text('#!/bin/sh\nexit 1\n')
        run(env, kdir, expect=1)
        assert (kdir / ".lccc-prepared").read_text() == new
        for name in CANARIES:
            (kdir / name).write_text("canary\n")

        # LCCC_REPREPARE=1 is honored even when the canaries are intact but the
        # recipe changed: the fast path must fall through to replacement rather
        # than exit 3. A mocked git makes the subsequent acquisition fail fast
        # without network access, proving the guard was crossed.
        (kdir / CANARIES[-1]).write_text("canary\n")
        (pkg / "config").write_text("CONFIG_TEST=reprepare\n")
        mock_git = bindir / "git"
        mock_git.write_text('#!/bin/sh\nexit 1\n')
        mock_git.chmod(0o755)
        result = run(dict(env, LCCC_REPREPARE="1", LCCC_KERNEL_SOURCE_MODE="git"),
                     kdir, expect=1)
        assert "LCCC_REPREPARE=1 replaces owned tree" in result.stderr, result.stderr

    print("PASS kernel provenance: mismatch refusal, no-op, config identity, "
          "compiler invalidation, metadata preservation, failure atomicity, damaged-tree guard, "
          "explicit reprepare")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
