"""Contract tests for ``scripts/arena_provision.py``'s tar extraction.

The hosted CI lane "Fast local contracts (shard b)" went red on
``test_workload_extraction_safety.test_no_unconditional_data_filter_anywhere_in_the_tree``
because the provisioning rewrite added a THIRD ``extractall(filter="data")``
call site, inside ``safe_extract``, outside the two guarded helpers the
repository permits.  This file pins the wiring and, more importantly, the
SECURITY and PORTABILITY behaviour that the structural rule exists to
protect:

* ``filter=`` is a Python 3.11.4+/3.12+ keyword.  Passing it on the declared
  3.9+ floor raises ``TypeError`` before a byte is extracted; silently
  dropping it without member validation would trade a loud failure for a
  directory-traversal hole.
* The capability probe must inspect the exact API that is called
  (``TarFile.extractall``), not ``hasattr(tarfile, "data_filter")``: those
  are different objects and the latter is a proxy, not a proof.
* Every archive member is validated BEFORE any write, so extraction never
  leaves a partial toolchain behind after a rejection.

The runner is loaded straight off disk with ``importlib`` so these tests
exercise the shipped script, not a copy of it.
"""
from __future__ import annotations

import ast
import importlib.util
import inspect
import io
import pathlib
import tarfile
import tempfile
import unittest
from pathlib import Path
from unittest import mock

REPO = pathlib.Path(__file__).resolve().parents[2]
PROVISIONER = REPO / "scripts" / "arena_provision.py"

_spec = importlib.util.spec_from_file_location("arena_provision_under_test", PROVISIONER)
prov = importlib.util.module_from_spec(_spec)
assert _spec.loader is not None
_spec.loader.exec_module(prov)


def member(name: str, *, kind: str = "file", link: str | None = None,
           data: bytes = b"payload") -> tarfile.TarInfo:
    info = tarfile.TarInfo(name)
    if kind == "file":
        info.size = len(data)
    elif kind == "dir":
        info.type = tarfile.DIRTYPE
        info.mode = 0o755
    elif kind == "sym":
        info.type = tarfile.SYMTYPE
        info.linkname = link or ""
    elif kind == "lnk":
        info.type = tarfile.LNKTYPE
        info.linkname = link or ""
    elif kind == "dev":
        info.type = tarfile.CHRTYPE
        info.devmajor = 1
        info.devminor = 3
    elif kind == "fifo":
        info.type = tarfile.FIFOTYPE
    else:  # pragma: no cover - guards against a typo in a future case
        raise ValueError(kind)
    return info


def archive(entries: list[tuple[tarfile.TarInfo, bytes | None]]) -> tarfile.TarFile:
    """An OPEN TarFile with the given members, exactly what the helper takes."""
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w") as out:
        for info, body in entries:
            # A member that DECLARES a size needs a fileobj on every supported
            # Python: 3.13's tarfile.addfile raises ValueError("fileobj not
            # provided for non zero-size regular file") where earlier versions
            # wrote a short member silently, so passing None made this whole
            # module error out on 3.13 before any policy was exercised. The
            # unsafe-member cases only exercise the NAME/TYPE policy, so pad
            # the content with NULs instead of omitting it.
            stream = io.BytesIO(body if body is not None else b"\0" * info.size)
            out.addfile(info, stream)
    buf.seek(0)
    return tarfile.open(fileobj=buf, mode="r")


def enclosing_functions(tree: ast.AST) -> dict[int, str]:
    owner: dict[int, str] = {}
    for fn in ast.walk(tree):
        if isinstance(fn, (ast.FunctionDef, ast.AsyncFunctionDef)):
            for node in ast.walk(fn):
                owner.setdefault(id(node), fn.name)
    return owner


class CapabilityProbeTests(unittest.TestCase):
    def test_probe_matches_the_api_that_is_called(self):
        expected = "filter" in inspect.signature(tarfile.TarFile.extractall).parameters
        self.assertIs(prov.extractall_takes_filter(), expected)

    def test_probe_fails_closed_when_signature_is_unavailable(self):
        with mock.patch.object(prov.inspect, "signature", side_effect=ValueError):
            self.assertFalse(prov.extractall_takes_filter())
        with mock.patch.object(prov.inspect, "signature", side_effect=TypeError):
            self.assertFalse(prov.extractall_takes_filter())

    def test_probe_is_not_a_python_version_tuple(self):
        source = PROVISIONER.read_text()
        tree = ast.parse(source, str(PROVISIONER))
        probe = next(n for n in tree.body
                     if isinstance(n, ast.FunctionDef)
                     and n.name == "extractall_takes_filter")
        code_only = ast.Module(
            body=[n for n in probe.body
                  if not (isinstance(n, ast.Expr)
                          and isinstance(n.value, ast.Constant)
                          and isinstance(n.value.value, str))],
            type_ignores=[])
        text = ast.unparse(code_only)
        self.assertIn("inspect.signature", text)
        self.assertIn("extractall", text)
        self.assertNotIn("version_info", text)
        self.assertNotIn("hasattr", text)


class WiringTests(unittest.TestCase):
    def test_every_filter_call_sits_inside_the_guarded_helper(self):
        tree = ast.parse(PROVISIONER.read_text(), str(PROVISIONER))
        owner = enclosing_functions(tree)
        filtered = [n for n in ast.walk(tree)
                    if isinstance(n, ast.Call)
                    and isinstance(n.func, ast.Attribute)
                    and n.func.attr == "extractall"
                    and any(kw.arg == "filter" for kw in n.keywords)]
        self.assertTrue(filtered, "filter= disappeared; rewrite this test")
        for call in filtered:
            self.assertEqual(owner.get(id(call)), "extract_untrusted",
                             "filter= must only be reached through the guard")

    def test_guard_is_called_by_the_extractor(self):
        tree = ast.parse(PROVISIONER.read_text(), str(PROVISIONER))
        helper = next(n for n in tree.body
                      if isinstance(n, ast.FunctionDef)
                      and n.name == "extract_untrusted")
        called = {n.func.id for n in ast.walk(helper)
                  if isinstance(n, ast.Call) and isinstance(n.func, ast.Name)}
        self.assertIn("extractall_takes_filter", called)

    def test_all_call_sites_use_the_guarded_helper(self):
        tree = ast.parse(PROVISIONER.read_text(), str(PROVISIONER))
        names = {n.func.id for n in ast.walk(tree)
                 if isinstance(n, ast.Call) and isinstance(n.func, ast.Name)}
        self.assertIn("extract_untrusted", names)
        self.assertNotIn("safe_extract", names,
                         "an unguarded alias reappeared")


class ExtractionPolicyTests(unittest.TestCase):
    def test_filter_is_passed_only_when_the_api_supports_it(self):
        src = archive([(member("toolchain/bin/cc", data=b"cc"), b"cc")])
        with tempfile.TemporaryDirectory() as td:
            for supported in (False, True):
                with self.subTest(supported=supported), \
                        mock.patch.object(prov, "extractall_takes_filter",
                                          return_value=supported), \
                        mock.patch.object(tarfile.TarFile, "extractall") as spy:
                    prov.extract_untrusted(src, Path(td))
                    self.assertEqual(spy.call_count, 1)
                    kw = spy.call_args.kwargs
                    if supported:
                        self.assertEqual(kw.get("filter"), "data")
                    else:
                        self.assertNotIn("filter", kw)

    def test_legacy_python_still_extracts_a_valid_archive(self):
        src = archive([
            (member("clang-23.1.5", kind="dir"), None),
            (member("clang-23.1.5/bin/clang", data=b"#!/bin/sh\n"), b"#!/bin/sh\n"),
        ])
        with tempfile.TemporaryDirectory() as td, \
                mock.patch.object(prov, "extractall_takes_filter", return_value=False):
            prov.extract_untrusted(src, Path(td))
            self.assertEqual((Path(td) / "clang-23.1.5/bin/clang").read_bytes(),
                             b"#!/bin/sh\n")

    def test_unsafe_members_are_refused_before_any_write(self):
        cases = [
            ("traversal", member("../escape", data=b"x")),
            ("absolute name", member("/tmp/escape", data=b"x")),
            ("absolute symlink", member("root/link", kind="sym", link="/etc/passwd")),
            ("escaping symlink", member("root/link", kind="sym", link="../../outside")),
            ("escaping hardlink", member("root/link", kind="lnk", link="../outside")),
            ("device node", member("root/mem", kind="dev")),
            ("fifo", member("root/pipe", kind="fifo")),
        ]
        for label, bad in cases:
            with self.subTest(label=label), tempfile.TemporaryDirectory() as td:
                good = member("clang-23.1.5/bin/clang", data=b"ok")
                src = archive([(good, b"ok"), (bad, None)])
                with mock.patch.object(prov, "extractall_takes_filter",
                                       return_value=False), \
                        self.assertRaises(RuntimeError):
                    prov.extract_untrusted(src, Path(td))
                self.assertEqual(list(Path(td).iterdir()), [],
                                 "validation must precede every write")

    def test_safe_symlink_and_hardlink_are_accepted(self):
        entries = [
            (member("p/bin/clang", data=b"cc"), b"cc"),
            (member("p/bin/cc", kind="sym", link="clang"), None),
            (member("p/bin/cc2", kind="lnk", link="p/bin/clang"), None),
        ]
        src = archive(entries)
        with tempfile.TemporaryDirectory() as td, \
                mock.patch.object(prov, "extractall_takes_filter", return_value=False):
            prov.extract_untrusted(src, Path(td))
            self.assertEqual((Path(td) / "p/bin/cc").read_bytes(), b"cc")
            self.assertEqual((Path(td) / "p/bin/cc2").read_bytes(), b"cc")

    def test_native_filter_is_a_second_layer_not_the_only_one(self):
        # Even when the stdlib filter is available, an unsafe member is
        # rejected by OUR validation before extractall is entered.
        src = archive([(member("../../escape", data=b"x"), b"x")])
        with tempfile.TemporaryDirectory() as td, \
                mock.patch.object(prov, "extractall_takes_filter", return_value=True), \
                mock.patch.object(tarfile.TarFile, "extractall") as spy, \
                self.assertRaises(RuntimeError):
            prov.extract_untrusted(src, Path(td))
        spy.assert_not_called()


if __name__ == "__main__":
    unittest.main()
