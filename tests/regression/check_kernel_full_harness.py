#!/usr/bin/env python3
"""Full-kernel harness contract without downloading or building a kernel.

Mocks make/Kbuild helpers and verifies configuration preservation, explicit
reconfiguration, provenance stamps, real-objtool enforcement, rejection of the
no-op shim, invalid job counts, package-feature drift, and post-build identity
re-verification.
"""

from __future__ import annotations

import hashlib
import os
import pathlib
import shutil
import subprocess
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts/build_kernel_full.sh"


def sha256(path: pathlib.Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def write_exec(path: pathlib.Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)
    path.chmod(0o755)


def run(env: dict[str, str], expect: int = 0) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(["bash", str(env["TEST_SCRIPT"])], env=env,
                            capture_output=True, text=True)
    if result.returncode != expect:
        raise AssertionError(
            f"harness exited {result.returncode}, expected {expect}\n"
            f"stdout:\n{result.stdout}\nstderr:\n{result.stderr}"
        )
    return result


def main() -> int:
    with tempfile.TemporaryDirectory(prefix="lccc-kfull-") as td:
        tmp = pathlib.Path(td)
        scripts = tmp / "scripts"
        scripts.mkdir()
        shutil.copy2(SCRIPT, scripts / "build_kernel_full.sh")
        bindir = tmp / "bin"
        bindir.mkdir()
        kdir = tmp / "linux-test"
        pkg = tmp / "pkg"
        pkg.mkdir()

        write_exec(scripts / "kernel_version.sh",
                   'lccc_kernel_default_dir(){ echo "$KERNEL_DIR"; }\n'
                   'lccc_kernel_pkgdir(){ echo "$PKGDIR"; }\n')
        write_exec(scripts / "kconfig_query.sh",
                   'lccc_config_drift(){ if [ "${TEST_DRIFT:-0}" = 1 ]; then\n'
                   '  echo "CONFIG_SCHED_BORE"; return 1; fi; return 0; }\n')
        write_exec(scripts / "kernel_tool_identity.sh",
                   '#!/bin/sh\necho "identity $*" >> "$TEST_HELPER_LOG"\n')
        write_exec(scripts / "kernel_boot_isa.sh",
                   '#!/bin/sh\necho "boot-isa $*" >> "$TEST_HELPER_LOG"\n')
        write_exec(scripts / "prepare_kernel_tree.sh",
                   '#!/bin/sh\necho "prepare $*" >> "$TEST_HELPER_LOG"\n'
                   'mkdir -p "$1/include/generated"\n'
                   'touch "$1/Makefile" "$1/.lccc-prepared" "$1/include/generated/autoconf.h"\n')

        for name in ("lccc", "lccc-ld", "hostcc"):
            write_exec(bindir / name, f'#!/bin/sh\necho "{name} test 1.0"\n')
        write_exec(bindir / "file", '#!/bin/sh\necho "ELF 64-bit executable"\n')
        write_exec(bindir / "make", r'''#!/bin/sh
printf '%s\n' "$*" >> "$TEST_MAKE_LOG"
case " $* " in
  *" defconfig "*)
    printf 'CONFIG_TEST=y\nCONFIG_OBJTOOL=y\nCONFIG_HZ=800\n' > .config ;;
  *" tools/objtool "*)
    mkdir -p tools/objtool
    printf '\177ELF' > tools/objtool/objtool
    chmod +x tools/objtool/objtool ;;
  *" bzImage "*)
    mkdir -p arch/x86/boot
    echo image > arch/x86/boot/bzImage
    if [ "${TEST_MUTATE_CONFIG_ON_BZIMAGE:-0}" = 1 ]; then
      echo '# drifted' >> .config
    fi ;;
esac
exit 0
''')
        write_exec(kdir / "scripts/config",
                   '#!/bin/sh\necho "config $*" >> "$TEST_HELPER_LOG"\n')

        (kdir / "Makefile").write_text("all:\n")
        (kdir / ".lccc-prepared").write_text("prepared\n")
        (kdir / "include/generated").mkdir(parents=True)
        (kdir / "include/generated/autoconf.h").write_text("/* autoconf */\n")
        original_config = "CONFIG_OBJTOOL=y\nCONFIG_HZ=800\nCONFIG_SCHED_BORE=y\n"
        (kdir / ".config").write_text(original_config)
        (pkg / "config").write_text("CONFIG_OBJTOOL=y\nCONFIG_HZ=1000\nCONFIG_SCHED_BORE=y\n")

        env = dict(
            os.environ,
            TEST_SCRIPT=str(scripts / "build_kernel_full.sh"),
            KERNEL_DIR=str(kdir),
            PKGDIR=str(pkg),
            LCCC=str(bindir / "lccc"),
            LCCC_LD=str(bindir / "lccc-ld"),
            HOSTCC=str(bindir / "hostcc"),
            TEST_MAKE_LOG=str(tmp / "make.log"),
            TEST_HELPER_LOG=str(tmp / "helper.log"),
            PATH=f"{bindir}:{os.environ['PATH']}",
            LCCC_JOBS="2",
        )

        # 1. Existing config is preserved; no defconfig is run.
        run(env)
        stamp = (kdir / ".lccc-full-build-config").read_text()
        assert "recipe=existing-config" in stamp, stamp
        assert f"config-sha256={sha256(kdir / '.config')}" in stamp
        assert f"compiler-sha256={sha256(bindir / 'lccc')}" in stamp
        assert f"linker-sha256={sha256(bindir / 'lccc-ld')}" in stamp
        log = (tmp / "make.log").read_text()
        assert " defconfig" not in log, log
        assert "olddefconfig" in log and "syncconfig" in log and "prepare" in log
        assert "tools/objtool" in log and "bzImage" in log
        assert (kdir / "tools/objtool/objtool").read_bytes().startswith(b"\x7fELF")
        helpers = (tmp / "helper.log").read_text()
        assert "identity" in helpers and "boot-isa" in helpers

        # 2. Explicit reconfiguration selects the package config.
        (tmp / "make.log").unlink()
        run(dict(env, LCCC_RECONFIG="1"))
        stamp = (kdir / ".lccc-full-build-config").read_text()
        assert "recipe=package-config" in stamp, stamp
        assert f"package-config-sha256={sha256(pkg / 'config')}" in stamp
        assert (kdir / ".config").read_text() == (pkg / "config").read_text()

        # 3. Package-feature drift is refused unless explicitly allowed.
        result = run(dict(env, LCCC_RECONFIG="1", TEST_DRIFT="1"), expect=1)
        assert "refusing to build a kernel the package does not ship" in result.stderr
        run(dict(env, LCCC_RECONFIG="1", TEST_DRIFT="1", LCCC_ALLOW_CONFIG_DRIFT="1"))

        # 4. No package config: documented defconfig recipe.
        (pkg / "config").unlink()
        (tmp / "make.log").unlink()
        run(dict(env, LCCC_RECONFIG="1"))
        stamp = (kdir / ".lccc-full-build-config").read_text()
        assert "recipe=fresh-defconfig-cachymod" in stamp, stamp
        assert " defconfig" in (tmp / "make.log").read_text()
        (pkg / "config").write_text("CONFIG_OBJTOOL=y\nCONFIG_HZ=1000\nCONFIG_SCHED_BORE=y\n")

        # 5. The no-op objtool shim and invalid job counts are rejected.
        result = run(dict(env, LCCC_NOOBJTOOL="1"), expect=2)
        assert "no longer supported" in result.stderr
        run(dict(env, LCCC_JOBS="0"), expect=2)
        run(dict(env, LCCC_JOBS="abc"), expect=2)

        # 6. A configuration mutated by Kbuild invalidates the run.
        (kdir / ".config").write_text(original_config)
        result = run(dict(env, TEST_MUTATE_CONFIG_ON_BZIMAGE="1"), expect=1)
        assert "configuration changed during Kbuild" in result.stderr

        # 7. A missing prepared tree triggers preparation exactly once.
        (kdir / ".config").write_text(original_config)
        (kdir / ".lccc-prepared").unlink()
        (kdir / "include/generated/autoconf.h").unlink()
        (tmp / "helper.log").write_text("")
        run(env)
        assert "prepare" in (tmp / "helper.log").read_text()

    print("PASS kernel full harness: config preservation/reconfig, provenance, "
          "real objtool, shim/job rejection, drift refusal, post-build identity")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
