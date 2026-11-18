"""Offline contracts for system-QEMU provisioning and the objtool census."""

import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
PROVISION = ROOT / "scripts/ensure_qemu_system.sh"
CENSUS = ROOT / "scripts/kernel_objtool_census.sh"
FIRMWARE = (
    "bios-256k.bin", "linuxboot_dma.bin", "kvmvapic.bin", "efi-e1000.rom",
    "vgabios-stdvga.bin",
)


def executable(path, text):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)
    path.chmod(0o755)


class QemuProvisioning(unittest.TestCase):
    @unittest.skipUnless(shutil.which("dpkg-deb"), "requires dpkg-deb")
    def test_cold_and_warm_machine_outputs_and_pin_verification(self):
        # Substitute only the package pin in a temporary copy. Package hashing,
        # extraction, firmware assembly, loader wrapper and publication run for real.
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp)
            cache = base / "cache with spaces"
            cache.mkdir()
            payload = base / "package"
            (payload / "DEBIAN").mkdir(parents=True)
            (payload / "DEBIAN/control").write_text(
                "Package: lccc-qemu-fixture\nVersion: 1\nArchitecture: all\n"
                "Maintainer: LCCC <test@example.invalid>\nDescription: Test fixture\n"
            )
            executable(payload / "usr/lib64/ld-linux-x86-64.so.2", "#!/bin/sh\nshift 2\nexec \"$@\"\n")
            executable(payload / "usr/bin/qemu-system-x86_64", """#!/bin/sh
case "$1" in
    --version) printf '%s\\n' 'QEMU emulator version 11.1.3 (fixture)' ;;
    -machine) printf '%s\\n' 'pc machine'; yes 'other machines' | head -n 20000 ;;
    -cpu) printf '%s\\n' 'qemu64 CPU'; yes 'other CPUs' | head -n 20000 ;;
    *) exit 5 ;;
esac
""")
            for i, filename in enumerate(FIRMWARE):
                firmware = payload / "usr/share" / ("qemu" if i else "seabios") / filename
                firmware.parent.mkdir(parents=True, exist_ok=True)
                firmware.write_text(filename)
            archive = cache / "qemu-fixture.deb"
            subprocess.run(["dpkg-deb", "--build", payload, archive], check=True, capture_output=True)
            blob = archive.read_bytes()
            source = PROVISION.read_text()
            before, pins_and_rest = source.split("PINS=$(cat <<'TABLE'\n", 1)
            _, after = pins_and_rest.split("\nTABLE\n)", 1)
            fixture = base / "provision.sh"
            fixture.write_text(
                before + "PINS=$(cat <<'TABLE'\n"
                + f"fixture\t1\tpool/{archive.name}\t{len(blob)}\t{hashlib.sha256(blob).hexdigest()}\tunused"
                + "\nTABLE\n)" + after
            )
            # Force all network attempts to fail immediately, even for bad pins.
            executable(base / "bin/curl", "#!/bin/sh\nexit 42\n")
            env = dict(os.environ, QEMU_DL_DIR=str(cache), PATH=f"{base / 'bin'}:{os.environ['PATH']}")

            def provision(mode, prefix):
                return subprocess.run(
                    ["bash", fixture, prefix, mode], env=env, capture_output=True, text=True,
                    timeout=20,
                )

            prefix = base / "a 'quoted' prefix"
            for _ in range(2):  # cold and warm stdout must be identical
                result = provision("--print-bin", prefix)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stdout, f"{prefix}/bin/qemu-system-x86_64\n")
            env_prefix = base / "another prefix"
            for _ in range(2):
                result = provision("--print-env", env_prefix)
                self.assertEqual(result.returncode, 0, result.stderr)
                values = subprocess.check_output([
                    "bash", "-ec", result.stdout
                    + 'printf "%s\\n" "$QEMU" "$QEMU_DATA_DIR" "$QEMU_ROM_DIR" "$QEMU_BIOS_DIR"',
                ], text=True).splitlines()
                self.assertEqual(values, [
                    f"{env_prefix}/bin/qemu-system-x86_64", f"{env_prefix}/share/qemu",
                    f"{env_prefix}/share/qemu", f"{env_prefix}/share/seabios",
                ])
            sentinel = base / "keep-this-directory" / "important.txt"
            sentinel.parent.mkdir()
            sentinel.write_text("preserve")
            refused = provision("--print-bin", sentinel.parent)
            self.assertEqual(refused.returncode, 2, refused.stderr)
            self.assertEqual(sentinel.read_text(), "preserve")
            self.assertEqual(refused.stdout, "")
            link = base / "alias"
            link.symlink_to(prefix)
            self.assertEqual(provision("--print-bin", link).returncode, 2)
            archive.write_bytes(b"corrupted cache entry")
            corrupt = base / "corrupt-install"
            result = provision("--print-bin", corrupt)
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(corrupt.exists())
            self.assertEqual(result.stdout, "")


class ObjtoolCensus(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.base = Path(tmp.name)
        self.kernel = self.base / "kernel"
        executable(self.kernel / "tools/objtool/objtool", """#!/bin/sh
[ "$1" = --dry-run ] && [ "$2" = --stackval ] && [ "$#" -eq 3 ] || exit 98
case "$3" in
    */warn.o) echo 'warning: objtool: synthetic warning' >&2 ;;
    */skip.o) echo 'file already has .static_call_sites section, skipping' >&2 ;;
    */fail.o) echo 'error: unreadable ELF' >&2; exit 7 ;;
    *) exit 0 ;;
esac
""")

    def record(self, name, *, flags="--stackval", target=None, present=True):
        obj = self.kernel / name
        obj.parent.mkdir(parents=True, exist_ok=True)
        if present:
            obj.write_bytes(b"fixture\n")
        obj.with_name(f".{obj.name}.cmd").write_text(
            f"savedcmd_{name} := cc -o {name} ; ./tools/objtool/objtool {flags} {target or name}\n"
        )
        return obj

    def census(self):
        return subprocess.run([
            "bash", CENSUS, self.kernel, "--out", self.base / "reports/output",
        ], capture_output=True, text=True, timeout=20)

    def test_complete_with_warning_and_split_scope(self):
        obj = self.record("init/warn.o")
        self.record("arch/x86/boot/main.o")
        self.record("tools/ignore.o", flags="-unexpected")
        result = self.census()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertRegex(result.stdout, r"COMPARABLE TOTAL\s+objects=2\s+warnings=1\s+skipped=0\s+errors=0")
        self.assertIn("warning: objtool: synthetic warning", result.stdout)
        self.assertEqual(obj.read_bytes(), b"fixture\n")

    def test_empty_scope_is_not_a_successful_census(self):
        result = self.census()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("objects=0", result.stderr)

    def test_errors_and_skips_are_visible_and_incomplete(self):
        self.record("init/skip.o")
        self.record("init/fail.o")
        self.record("init/missing.o", present=False)
        self.record("init/mismatch.o", target="init/skip.o")
        self.record("init/bad-flag.o", flags="-unexpected")
        result = self.census()
        self.assertNotEqual(result.returncode, 0)
        self.assertRegex(result.stdout, r"COMPARABLE TOTAL\s+objects=2\s+warnings=0\s+skipped=1\s+errors=4")
        self.assertIn("error: unreadable ELF", result.stderr)
        self.assertIn("SKIPPED\tinit/skip.o", result.stderr)
        self.assertIn("init/.missing.o.cmd", result.stderr)
        self.assertIn("invalid saved objtool target", result.stderr)


if __name__ == "__main__":
    unittest.main()
