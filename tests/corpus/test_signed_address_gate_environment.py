"""Executable harness checks: compiler controls must reach the compiler, not C main."""
import importlib.util
import contextlib
import io
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]


class SignedAddressEnvironmentTests(unittest.TestCase):
    def test_affine_control_is_applied_when_compiling(self):
        script = (ROOT / "tests/regression/check_affine_loop_fold.sh").read_text()
        start = script.index("run_case() {")
        function = script[start:script.index("\n}\n", start) + 3]
        with tempfile.TemporaryDirectory() as directory:
            work = Path(directory)
            compiler = work / "compiler"
            compiler.write_text('''#!/bin/bash
set -eu
output=${@: -1}
printf '%s' "${CCC_LOOP_ROTATE:-missing}" > "$output.compiler_env"
cat > "$output" <<'PROGRAM'
#!/bin/sh
# This is a compile-time control, not a runtime fixture input.
[ "${CCC_LOOP_ROTATE:-missing}" = missing ] || exit 7
printf 'OK\n'
PROGRAM
chmod +x "$output"
''')
            compiler.chmod(0o755)
            command = '''set -eu
work=$1; CCC=$work/compiler; corpus=unused; expected=OK
bad() { echo "$*" >&2; exit 1; }
note() { :; }
''' + function + '\nrun_case rotated "-O2" "CCC_LOOP_ROTATE=1"\n'
            environment = {k: v for k, v in os.environ.items() if not k.startswith("CCC_")}
            result = subprocess.run(["bash", "-c", command, "test", directory],
                                    env=environment, capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual((work / "rotated.compiler_env").read_text(), "1")

    def test_memcheck_oracles_remove_inherited_kill_switches(self):
        spec = importlib.util.spec_from_file_location("wrapv_gate", ROOT / "scripts/check_wrapv_oracles.py")
        gate = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(gate)
        for fixture, expected_runs in [("domains", 30), ("signed-scale", 30),
                                       ("canonical", 60), ("pointer", 90),
                                       ("pointer-i128", 30)]:
            with self.subTest(fixture=fixture), tempfile.TemporaryDirectory() as directory:
                seen = []

                def check_environment(environment):
                    self.assertFalse(any(k.startswith("CCC_") for k in environment))
                    self.assertEqual(environment["CCC"], "preserved-selector")
                    self.assertEqual(environment["PR835_CANARY"], "preserved")

                def version(command, **kwargs):
                    if command[0] != "valgrind":
                        check_environment(kwargs["env"])
                    return "mock version\n"

                def run(command, **kwargs):
                    check_environment(kwargs["env"])
                    if command[0] != "valgrind":
                        Path(command[-1]).write_bytes(b"mock executable")
                        return subprocess.CompletedProcess(command, 0, "", "")
                    seen.append(command)
                    log = next(arg.split("=", 1)[1] for arg in command if arg.startswith("--log-file="))
                    Path(log).write_text("mock Memcheck log\n")
                    pointer_i128_case = (fixture == "pointer-i128" or
                                         (fixture == "pointer" and "pointer-i128" in command[-1]))
                    output = ("pointer-to-int128 implementation-defined extension: PASS"
                              if pointer_i128_case else
                              "682afb0d" if "reverse-sub" in command[-1] else
                              "integer pointer widths: PASS" if fixture == "pointer" else
                              "i32 address canonical: PASS 82" if fixture == "canonical" else
                              "signed scale ring: PASS 16" if fixture == "signed-scale" else
                              ["28", "-1073665362", "1", "10"][int(command[-1])])
                    return subprocess.CompletedProcess(command, 0, output + "\n", "")

                argv = ["gate", "--lccc", sys.executable, "--gcc", sys.executable,
                        "--clang", sys.executable, "--fixture", fixture,
                        "--work-dir", directory, "--json", str(Path(directory) / "report.json")]
                with patch.dict(os.environ, {"CCC_NO_MACHINST": "1", "CCC_NO_GEP_FOLD": "1",
                                             "CCC": "preserved-selector", "PR835_CANARY": "preserved"}), \
                     patch.object(sys, "argv", argv), \
                     patch.object(gate.subprocess, "check_output", side_effect=version), \
                     patch.object(gate.subprocess, "run", side_effect=run), \
                     patch.object(gate.shutil, "which", return_value=sys.executable):
                    with contextlib.redirect_stdout(io.StringIO()):
                        gate.main()
                self.assertEqual(len(seen), expected_runs)


if __name__ == "__main__":
    unittest.main()
