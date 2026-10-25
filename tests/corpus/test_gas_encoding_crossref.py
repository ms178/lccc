"""Every declared encoder row must be a legal GNU as 2.47 encoding.

The differential harness compares LCCC against `as` 2.47 byte for byte, but only
for the instructions a corpus happens to contain.  ``scripts/gas_encoding_crossref.py``
closes that gap: it expands `opcodes/i386-opc.tbl` — GNU as 2.47's own
instruction table, including its template blocks — into a vendored snapshot, and
checks every declarative row of *both* encoder backends (`i686` and `x86`)
against it.

This test runs that check hermetically (no network, no reference file) so a row
that is added with a wrong opcode, prefix, map or `/digit` fails in CI even when
no corpus exercises it, and so the claim "cross-referenced with GNU as 2.47"
stays true as the encoder grows.

The snapshot is regenerated from upstream with::

    gh api repos/RTEMS/sourceware-mirror-binutils-gdb/contents/opcodes/i386-opc.tbl \\
        ?ref=binutils-2_47-branch --jq .content | tr -d '\\n' | base64 -d > /tmp/i386-opc.tbl
    python3 scripts/gas_encoding_crossref.py --build /tmp/i386-opc.tbl

``scripts/isa/gas-2.47.manifest.json`` records the upstream path, ref and blob
digest of the table the snapshot was built from.

The two backends differ in one thing the model cannot infer from a row: `40..4F`
is a REX prefix in 64-bit mode and an `inc`/`dec` opcode in 32-bit mode, so the
mode is taken from the row's own path (`sdm.model_any`).
"""

from __future__ import annotations

import importlib.util
import contextlib
import io
from unittest import mock
import json
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))

CROSSREF = ROOT / "scripts" / "gas_encoding_crossref.py"
SDM = ROOT / "scripts" / "sdm_isa_crossref.py"


def _load(path: Path, name: str):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


class GasEncodingCrossrefTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.gas = _load(CROSSREF, "gas_encoding_crossref")
        cls.sdm = _load(SDM, "sdm_isa_crossref")
        cls.table = cls.gas.load_snapshot()
        cls.rows = []
        for path in cls.sdm.ENCODER_FILES:
            cls.rows.extend(cls.sdm.extract_rows(Path(path).resolve()))

    def test_snapshot_has_provenance(self):
        manifest = json.loads((ROOT / "scripts" / "isa" / "gas-2.47.manifest.json").read_text())
        upstream = manifest["upstream"]
        self.assertEqual(upstream["ref"], "binutils-2_47-branch")
        self.assertEqual(upstream["path"], "opcodes/i386-opc.tbl")
        self.assertEqual(len(upstream["sha256"]), 64)
        # The table is generated from template blocks; a snapshot built without
        # expanding them loses the largest families (the ALU, the <sd>/<bw>
        # pairs), which would silently shrink the gate instead of failing it.
        self.assertGreater(manifest["stats"]["declarations"], 60)
        self.assertGreater(manifest["mnemonics"], 1500)
        self.assertGreater(manifest["encodings"], 3000)

    def test_snapshot_pins_the_upstream_table_it_was_built_from(self):
        """The snapshot is only an oracle if it came from *that* table.

        `upstream.sha256` is the digest of `opcodes/i386-opc.tbl` at
        `binutils-2_47-branch`; a regeneration from another branch or a docored
        copy would change it.  The digest below was re-fetched from upstream and
        a rebuild from those bytes reproduces `scripts/isa/gas-2.47.json`
        byte-for-byte.
        """
        manifest = json.loads(
            (ROOT / "scripts" / "isa" / "gas-2.47.manifest.json").read_text())
        upstream = manifest["upstream"]
        self.assertEqual(upstream["repo"], "RTEMS/sourceware-mirror-binutils-gdb")
        self.assertEqual(
            upstream["sha256"],
            "a48171c05930cbe120a26fdf802689a4865607b3fe42eca784541dbf7ba81179")
        self.assertEqual(upstream["bytes"], 288187)

    def test_every_row_is_a_legal_encoding(self):
        findings, unmodeLLed, checked, _ = self.gas.verify(
            self.rows, self.table,
            lambda row, name=None: self.sdm.model_row(row, False, name))
        self.assertEqual([r["body"] for r in unmodeLLed], [])
        self.assertGreater(checked, 400)
        self.assertEqual(
            findings,
            [],
            "\n".join(
                f"{f['kind']} {f['name']} at {f['file']}:{f['line']}: {f['body']}"
                for f in findings[:20]
            ),
        )

    def test_both_backends_are_legal_encodings(self):
        rows = []
        for path in self.sdm.encoder_files("all"):
            rows.extend(self.sdm.extract_rows(Path(path).resolve()))
        findings, unmodeLLed, checked, _ = self.gas.verify(
            rows, self.table, self.sdm.model_any)
        self.assertGreater(checked, 900)
        self.assertLess(len(unmodeLLed), len(rows))
        self.assertEqual(
            findings,
            [],
            "\n".join(
                f"{f['kind']} {f['name']} at {f['file']}:{f['line']}: {f['body']}"
                for f in findings[:20]
            ),
        )

    def test_32_bit_inc_dec_is_not_read_as_a_rex_prefix(self):
        # `40` is `inc %eax` in 32-bit mode, so a 32-bit row must keep it; the
        # 64-bit backend reads the same byte as a REX prefix.
        row = {"file": "src/backend/i686/assembler/encoder/mod.rs", "line": 1,
               "names": ["incl"], "body": "self.encode_inc_dec(ops, 0x40)", "arrays": [[0x40]]}
        self.assertFalse(self.sdm.row_is_64bit(row))
        self.assertEqual(self.sdm.model_any(row)[0]["opcode"], 0x40)
        self.assertTrue(self.sdm.row_is_64bit({**row, "file": row["file"].replace("i686", "x86")}))

    def test_coverage_lists_only_mnemonics_without_a_row(self):
        table = {
            # Implemented: must not appear in the worklist.
            "addl": [{"family": "legacy", "map": "none", "pp": "none",
                      "opcode": 0x01, "w": None, "reg": None}],
            # VEX-encoded and unimplemented, and EVEX-only (out of scope for a
            # VEX/legacy backend), which the worklist drops.
            "vmptrld": [{"family": "vex", "map": "0f38", "pp": "none",
                         "opcode": 0x06, "w": None, "reg": 6}],
            "vpaddd": [{"family": "evex", "map": "0f", "pp": "66",
                        "opcode": 0xFE, "w": None, "reg": None}],
        }
        rows = [{"file": "x", "line": 1, "names": ["addl"],
                 "body": "self.encode_alu(ops)", "arrays": []}]
        self.assertEqual(sorted(self.gas.coverage(rows, table)), ["vmptrld"])

    def test_single_byte_opcodes_are_not_read_as_prefixes(self):
        """The BMI rows spell their opcode as a lone `F2`/`F3` byte.

        `opcodes/i386-opc.tbl` packs `[mandatory prefix] opcode` into one field,
        and a three-byte VEX or EVEX row leaves the opcode alone in it: `andn
        0xf2` is VEX.NDS.LZ.0F38.W0 F2, `blsi 0xf3/3` is ...F3 /3.  A distiller
        that reads that byte as a prefix has no opcode left and silently drops
        the row, which is how `andn`/`blsi`/`blsr`/`blsmsk` used to be missing
        from the snapshot while being perfectly legal encodings.
        """

        def encs(name):
            return {(e["family"], e["map"], e["pp"], e["opcode"], e["reg"])
                    for e in self.table.get(name, [])}

        self.assertIn(("vex", "0f38", "none", 0xF2, None), encs("andn"))
        self.assertIn(("vex", "0f38", "none", 0xF3, 3), encs("blsi"))
        self.assertIn(("vex", "0f38", "none", 0xF3, 1), encs("blsr"))
        self.assertIn(("vex", "0f38", "none", 0xF3, 2), encs("blsmsk"))
        # Same shape, EVEX-only encoding: EVEX.66.0F3A.W0 66 /r ib.
        self.assertIn(("evex", "0f3a", "66", 0x66, None), encs("vfpclassps"))

    def test_prefix_and_opcode_in_one_field_still_split(self):
        """`shrx` is `0xf2f7`: mandatory prefix F2, opcode F7."""
        forms = {(e["family"], e["map"], e["pp"], e["opcode"], e["reg"])
                 for e in self.table["shrx"]}
        self.assertIn(("vex", "0f38", "f2", 0xF7, None), forms)

    def test_three_byte_opcode_rows_keep_their_modrm_digit(self):
        """`0F 01 <ModRM>`: the digit the row encodes is ModRM's `/reg` field.

        The table spells the whole thing in one field (`wrmsrns 0x0f01c6`,
        `sgdt 0x0f010/0`), so the distiller has to split escape, opcode and
        ModRM.  Verified against GNU as 2.47: `swapgs` is `0F 01 F8` (=/7),
        `monitor` `0F 01 C8` (=/1), `xgetbv` `0F 01 D0` (=/2), `rdpkru` `0F 01
        EE` (=/5), `wrmsrns` `0F 01 C6` (=/0).
        """

        def encs(name):
            return {(e["family"], e["map"], e["opcode"], e["reg"])
                    for e in self.table.get(name, [])}

        for name, digit in (("wrmsrns", 0), ("sgdt", 0), ("monitor", 1),
                            ("mwait", 1), ("xgetbv", 2), ("serialize", 5),
                            ("rdpkru", 5), ("swapgs", 7), ("rdtscp", 7)):
            self.assertIn(("legacy", "0f", 0x01, digit), encs(name), name)

    def test_no_encoder_row_is_uncovered(self):
        """The gate's worklist must stay empty (MB-43).

        `andn`, `blsi`, `blsr`, `blsmsk` and the parameterised `setcc` used to
        appear here; the first four are now distilled from the table and
        `setcc` resolves through `SNAPSHOT_ALIASES` to the `setc` row it emits.
        """
        rows = []
        for path in self.sdm.encoder_files("all"):
            rows.extend(self.sdm.extract_rows(Path(path).resolve()))
        findings, _, checked, uncovered = self.gas.verify(
            rows, self.table, self.sdm.model_any)
        self.assertEqual(uncovered, [])
        self.assertEqual(findings, [])
        self.assertGreater(checked, 1600)

    def test_setcc_alias_resolves_to_the_row_it_encodes(self):
        row = {"file": "src/backend/x86/assembler/encoder/mod.rs", "line": 1,
               "names": ["setcc"], "body": 'self.encode_setcc(ops, "setc")',
               "arrays": []}
        setc = {"family": "legacy", "map": "0f", "pp": "none", "opcode": 0x92,
                "reg": 0, "w": None}
        findings, unmodeLLed, checked, uncovered = self.gas.verify(
            [row], self.table, lambda r, name=None: [dict(setc)])
        self.assertEqual((findings, unmodeLLed, uncovered), ([], [], []))
        self.assertEqual(checked, 1)

    def test_expression_evaluator_uses_c_precedence(self):
        for expression, expected in [
            ("1 + 2 << 3", 24), ("1 << 2 + 1", 8),
            ("2 + 3 * 4 << 1", 28), ("1 | 2 + 3 << 1", 11),
            ("1 << 1 << 2", 8), ("1 + (2 << 3)", 17),
            ("(1 + 2) * 3 << 1", 18),
        ]:
            with self.subTest(expression=expression):
                self.assertEqual(self.gas.eval_expr(expression), expected)

    def test_prefix_family_is_not_a_substring_guess(self):
        for attrs, expected in [
            ("EVex128|Space0F|VexW0", ["evex"]),
            ("EVexLIG|Space0F|VexW1", ["evex"]),
            ("Masking|Space0F|VexW=2", ["evex"]),
            ("Vex128|Space0F|VexW0", ["vex"]),
            ("Vex128|EVex128|Space0F|VexW0", ["vex", "evex"]),
            ("Space0F|VexW0", ["legacy"]),
        ]:
            with self.subTest(attrs=attrs):
                self.assertEqual(self.gas._canonical(0x90, None, attrs)[1], expected)
        # The real snapshot must not admit an invented VEX FPCLASS form.
        self.assertEqual({e["family"] for e in self.table["vfpclassps"]}, {"evex"})

    def test_family_and_both_modrm_w_constraints_must_match(self):
        enc = {"family": "vex", "map": "0f38", "pp": "none",
               "opcode": 0xF3, "reg": 3, "w": 0}
        self.assertTrue(self.gas._same(enc, dict(enc)))
        for key, bad in [("family", "evex"), ("family", "legacy"),
                         ("reg", 2), ("w", 1), ("opcode", 0xF2),
                         ("pp", "f2"), ("map", "0f")]:
            with self.subTest(key=key, bad=bad):
                self.assertFalse(self.gas._same(enc, dict(enc, **{key: bad})))
        # Unknown W is deliberately unconstrained, not silently interpreted as0.
        self.assertTrue(self.gas._same(enc, dict(enc, w=None)))
        # Exercise the public verifier: a wrong W cannot pass just because /3
        # matches. The only candidate encoding is intentionally malformed.
        row = {"file": "synthetic", "line": 1, "names": ["blsi"], "body": "mutant"}
        findings, _, checked, _ = self.gas.verify(
            [row], {"blsi": [enc]}, lambda row, name=None: [dict(enc, w=1)])
        self.assertEqual(checked, 1)
        self.assertEqual([f["kind"] for f in findings], ["LCCC-EXTRA"])

    def test_numeric_w_attributes_are_not_unknown(self):
        for value, expected in [("1", 0), ("2", 1), ("3", None),
                                ("VEXW0", 0), ("VEXW1", 1), ("VEXWIG", None)]:
            with self.subTest(value=value):
                enc, _ = self.gas._canonical(0x666f, None, "Masking|Space0F|VexW=" + value)
                self.assertEqual(enc["w"], expected)
        self.assertTrue(all(e["w"] == 0 for e in self.table["vmovdqa32"]))
        self.assertTrue(all(e["w"] == 1 for e in self.table["vmovdqa64"]))

    def test_v_spelling_requires_the_gas_template_attribute(self):
        for fake in ("vandn", "vblsi", "vblsr", "vblsmsk", "vshrx"):
            with self.subTest(fake=fake):
                self.assertNotIn(fake, self.table)
                row = {"file": "synthetic", "line": 1, "names": [fake], "body": "mutant"}
                base = self.table[fake[1:]][0]
                findings, _, checked, uncovered = self.gas.verify(
                    [row], self.table, lambda row, name=None: [dict(base)])
                self.assertEqual(findings, [])
                self.assertEqual(checked, 0)
                self.assertEqual([u["name"] for u in uncovered], [fake])
        # Legacy sqrtps and its SSE2AVX spelling have distinct families.
        self.assertEqual({e["family"] for e in self.table["sqrtps"]}, {"legacy"})
        self.assertEqual({e["family"] for e in self.table["vsqrtps"]}, {"vex", "evex"})
        parsed = self.gas._parse_row(
            "sqrtps, 0x0f51, AVX, Modrm|Vex128|SSE2AVX, { RegXMM, RegXMM }")
        self.assertEqual({e["mnemonic"] for e in parsed}, {"vsqrtps"})

    def test_strict_cli_fails_for_an_uncovered_mnemonic(self):
        uncovered = [{"name": "invented", "file": "synthetic", "line": 1}]
        for strict, expected in [(True, 1), (False, 0)]:
            argv = ["gas_encoding_crossref.py", "--verify", "--backend", "all"]
            if strict:
                argv.append("--strict")
            with self.subTest(strict=strict), mock.patch.object(sys, "argv", argv), \
                 mock.patch.object(self.gas, "verify", return_value=([], [], 0, uncovered)), \
                 contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(self.gas.main(), expected)

    def test_core_isa_encodings_are_pinned(self):
        """A spot check so a snapshot regenerated from the wrong branch fails."""
        def has(name, family, map_, pp, opcode):
            return any(
                e["family"] == family
                and e["map"] == map_
                and e["pp"] == pp
                and e["opcode"] == opcode
                for e in self.table.get(name, [])
            )

        self.assertTrue(has("add", "legacy", "none", "none", 0x00))
        self.assertTrue(has("add", "legacy", "none", "none", 0x83))
        self.assertTrue(has("mov", "legacy", "none", "none", 0xA0))
        self.assertTrue(has("imul", "legacy", "none", "none", 0x6B))
        self.assertTrue(has("crc32", "legacy", "0f38", "f2", 0xF0))
        self.assertTrue(has("fcmovb", "legacy", "none", "none", 0xDA))

    def test_the_alu_code_model_derives_the_opcode_from_the_dispatch_arm(self):
        """`code_models()` reads `encode_alu`, it does not restate the SDM.

        The nine legacy forms are the arithmetic in that function: the six
        opcode forms are `0x01 + op * 8` off the dispatch index, the three
        immediate forms are `/op` over a fixed opcode.  `add` (0), `and` (4) and
        `cmp` (7) therefore land on three different blocks from one table.
        """
        models = self.gas.code_models()

        def forms(name):
            return sorted((f["opcode"], f["reg"]) for f in models[name])

        self.assertEqual(
            forms("add"),
            [(0x00, None), (0x01, None), (0x02, None), (0x03, None),
             (0x04, None), (0x05, None), (0x80, 0), (0x81, 0), (0x83, 0)],
        )
        self.assertEqual(
            forms("and"),
            [(0x20, None), (0x21, None), (0x22, None), (0x23, None),
             (0x24, None), (0x25, None), (0x80, 4), (0x81, 4), (0x83, 4)],
        )
        self.assertEqual(
            forms("cmp"),
            [(0x38, None), (0x39, None), (0x3A, None), (0x3B, None),
             (0x3C, None), (0x3D, None), (0x80, 7), (0x81, 7), (0x83, 7)],
        )
        # The model is keyed by the GAS spelling, so all four suffixed arms of
        # `add` collapse onto one name instead of duplicating the forms.
        self.assertEqual(len(models["add"]), 9)

    def test_a_code_modelled_family_leaves_the_gap_list(self):
        """The ALU gap entries were a modelling hole, not missing encodings.

        These forms are now covered by the declarative per-spelling row model;
        code models may expose new APX gaps but must not silently erase row gaps.
        """
        model = self.sdm.model_any
        before = self.gas.missing(self.rows, self.table, model)
        after = self.gas.missing(self.rows, self.table, model, self.gas.code_models())

        def opcodes(gaps, name):
            return sorted(e["opcode"] for e in gaps.get(name, [])
                          if e["family"] == "legacy")

        # `add` is now covered per spelling by the row model (including the
        # direction and width variants), so it must not be presented as a gap
        # even before code-driven families are layered on.
        for name in ("add", "and", "cmp", "sub", "xor", "or", "adc", "sbb"):
            self.assertEqual(opcodes(before, name), [], name)
            self.assertEqual(opcodes(after, name), [], name)
        # The model may *add* names to the comparison: `cmov<cc>`, `j<cc>` and
        # `set<cc>` have no declarative row at all today, so before the model
        # they were not looked at, and with it their APX forms (`cmovne` is
        # also `EVexMap4 0x45` in the table) become visible as real gaps.  What
        # must never happen is a mnemonic the row model already covered losing
        # coverage, so the guarantee is per name, not on the total.
        for name, gaps in before.items():
            self.assertLessEqual(len(after.get(name, [])), len(gaps), name)
        self.assertEqual(sum(len(after.get(n, [])) for n in before),
                         sum(len(v) for v in before.values()))


class GasAbsentTriageTests(unittest.TestCase):
    """``--triage`` splits the declarative-row gap list by source spelling.

    The split is a reading aid, and these tests pin exactly the two ways it can
    mislead so the help text keeps saying so: a name that is only a *prefix* of
    another mnemonic must not count as a mention, and a family that composes its
    mnemonic has no literal to find.
    """

    @classmethod
    def setUpClass(cls):
        cls.gas = _load(CROSSREF, "gas_encoding_crossref")

    def _triage(self, source: str, names: list[str]):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "encoder.rs"
            path.write_text(source)
            return self.gas.triage_absent(names, [str(path)])

    def test_a_prefix_of_a_longer_mnemonic_is_not_a_mention(self):
        unmentioned, mentioned = self._triage(
            'fn f() { match m { "vaddps" | "addn" | "blsi" => {} } }',
            ["add", "vaddps", "addn"],
        )
        self.assertEqual(mentioned, ["vaddps", "addn"])
        self.assertEqual(unmentioned, ["add"])

    def test_a_quoted_or_commented_spelling_counts_as_a_mention(self):
        unmentioned, mentioned = self._triage(
            '// movbe is handled by encode_movbe\nlet n = "rorx";\n',
            ["movbe", "rorx", "pdep"],
        )
        self.assertEqual(mentioned, ["movbe", "rorx"])
        self.assertEqual(unmentioned, ["pdep"])

    def test_a_composed_mnemonic_has_no_literal_and_lands_unmentioned(self):
        # The documented limitation: `format!("vfmadd{op}132pd")` covers the
        # whole family, so the UNMENTIONED half is not a coverage verdict.
        unmentioned, mentioned = self._triage(
            'let m = format!("vfmadd{}132pd", op);\n', ["vfmadd132pd"]
        )
        self.assertEqual(unmentioned, ["vfmadd132pd"])
        self.assertEqual(mentioned, [])


class GasDirectionModelTests(unittest.TestCase):
    """`D`/`W`/`+rd` expansion, pinned to what GNU as 2.47 really emits.

    PR #965 made the snapshot an oracle that rejects invented encodings, but it
    modelled the direction bit as "XOR 1" for everything.  GNU as picks one of
    four schemes, and three of the nine cases it applies to change nothing --
    so the snapshot simultaneously *invented* encodings (`movdqa 66 0f 6e` is
    `movd`) and *missed* real ones (`and (%rax),%al` is `22 00`).  Every
    assertion below is a byte sequence produced by the pinned 2.47 pair, not a
    reading of the SDM.
    """

    @classmethod
    def setUpClass(cls):
        cls.gas = _load(CROSSREF, "gas_encoding_crossref")
        cls.table = cls.gas.load_snapshot()

    def opcodes(self, name, family="legacy", map_="none", pp="none"):
        return sorted(e["opcode"] for e in self.table.get(name, [])
                      if e["family"] == family and e["map"] == map_
                      and e["pp"] == pp)

    def test_the_alu_direction_pair_is_two_apart_not_one(self):
        """`Opcode_D` is 0x2: `add %al,(%rax)` is `00`, `add (%rax),%al` is `02`."""
        for name, base in (("add", 0x00), ("or", 0x08), ("adc", 0x10),
                           ("sbb", 0x18), ("and", 0x20), ("sub", 0x28),
                           ("xor", 0x30), ("cmp", 0x38)):
            block = self.opcodes(name)
            self.assertIn(base + 0, block, name)
            self.assertIn(base + 2, block, name)
            self.assertIn(base + 2 + 1, block, name)   # +2 direction, +1 word
            # The accumulator-immediate rows (`04`/`05` for add) are a
            # different row and carry no `D`, so they stay where the table
            # puts them.
            acc = base + 4
            self.assertIn(acc, block, name)
            self.assertIn(acc + 1, block, name)

    def test_direction_and_width_compose(self):
        """`mov` carries `D|W`: all four of `88`, `89`, `8a`, `8b` are real."""
        block = self.opcodes("mov")
        for opcode in (0x88, 0x89, 0x8A, 0x8B):
            self.assertIn(opcode, block)
        # ... and `8d` is `lea`, which the old one-dimensional expansion
        # claimed for `mov`.
        self.assertNotIn(0x8D, block)

    def test_the_compact_simd_moves_pair_by_ten_not_by_one(self):
        """`Opcode_SIMD_IntD`: `movdqa` is `6f`/`7f`; `6e` is `movd`."""
        self.assertEqual(self.opcodes("movdqa", map_="0f", pp="66"), [0x6F, 0x7F])
        self.assertEqual(self.opcodes("movd", map_="0f", pp="66"), [0x6E, 0x7E])
        # `movq %xmm0,%rax` is `66 0f 7e`, and `66 0f 6f` is `movdqa`.
        self.assertIn(0x7E, self.opcodes("movq", map_="0f", pp="66"))
        self.assertNotIn(0x6F, self.opcodes("movq", map_="0f", pp="66"))

    def test_a_commutative_row_keeps_its_opcode(self):
        """`C` is `#define C StaticRounding`, the commutative bit.

        `test %eax,(%rax)` and `test (%rax),%eax` are both `85 00`, so `86`/`87`
        -- which are `xchg` -- must not appear under `test`.
        """
        self.assertNotIn(0x86, self.opcodes("test"))
        self.assertNotIn(0x87, self.opcodes("test"))
        self.assertIn(0x86, self.opcodes("xchg"))
        self.assertIn(0x87, self.opcodes("xchg"))

    def test_ugh_and_fma4_rows_never_flip(self):
        """`faddp` is rewritten, not re-encoded; FMA4 puts the order in VEX.W."""
        self.assertNotIn(0xDA, self.opcodes("faddp"))     # `DA` is `fiadd`
        self.assertIn(0xDE, self.opcodes("faddp"))
        # The eight scalar FMA4 spellings are `6a/6b/6e/6f/7a/7b/7e/7f`.
        self.assertIn(0x6E, self.opcodes("vfmsubss", family="vex", map_="0f3a",
                                         pp="66"))
        self.assertNotIn(0x7E, self.opcodes("vfmsubss", family="vex",
                                            map_="0f3a", pp="66"))

    def test_x87_reverse_is_the_neighbouring_block(self):
        """`fadd %st,%st(1)` is `DC`, four above `D8` (`Opcode_FloatD`)."""
        self.assertIn(0xD8, self.opcodes("fadd"))
        self.assertIn(0xDC, self.opcodes("fadd"))

    def test_mov_to_and_from_the_special_registers_keeps_the_base_scheme(self):
        """`mov cr/dr/tr` (`0f 20..27`) is exempt from the extended scheme."""
        block = self.opcodes("mov", map_="0f")
        for opcode in (0x20, 0x21, 0x22, 0x23, 0x24, 0x26):
            self.assertIn(opcode, block)
        self.assertNotIn(0x25, block)     # `0f 25` is not an instruction

    def test_register_in_opcode_rows_expand_over_all_eight_registers(self):
        """No ModR/M means the register number is the opcode's low three bits."""
        for opcode in range(0x50, 0x58):
            self.assertIn(opcode, self.opcodes("push"))
            self.assertIn(opcode + 0x08, self.opcodes("pop"))
        self.assertEqual(self.opcodes("bswap", map_="0f"),
                         list(range(0xC8, 0xD0)))
        self.assertIn(0xB9, self.opcodes("mov"))
        self.assertIn(0x91, self.opcodes("xchg"))     # `xchg %eax,%ecx`
        # ... but only when the row really has a GPR operand in the opcode:
        # `in $1,%al` is `e4 01`, with the accumulator implied.
        self.assertEqual(self.opcodes("in")[:2], [0xE4, 0xE5])


class GasCodeModelGateTests(unittest.TestCase):
    """`--verify` gates the code-driven families, not just the declarative rows.

    The ALU, the condition-code spelling, `inc`/`dec` and FMA3 derive their
    opcodes in code, so before this gate no check looked at them at all: a
    model could drift from the encoder (or the encoder from `as`) and the only
    symptom would be a *smaller* gap list.
    """

    @classmethod
    def setUpClass(cls):
        cls.gas = _load(CROSSREF, "gas_encoding_crossref")
        cls.table = cls.gas.load_snapshot()

    def test_every_claim_is_a_form_as_accepts(self):
        for backend in ("i686", "x86", "all"):
            with self.subTest(backend=backend):
                claims = self.gas.code_models(backend, self.table)
                findings, checked = self.gas.verify_code_models(claims, self.table)
                self.assertEqual(findings, [])
                self.assertEqual(checked, sum(len(v) for v in claims.values()))
                self.assertGreater(len(claims), 100)

    def test_the_cc_and_fma_claims_are_derived_from_the_encoder_tables(self):
        claims = self.gas.code_models("all", self.table)
        # `jne` short form, `cmovne` `0f 45`, `setne` `0f 95 /0`.
        self.assertEqual([e["opcode"] for e in claims["jne"]], [0x75])
        self.assertEqual([(e["map"], e["opcode"]) for e in claims["cmovne"]],
                         [("0f", 0x45)])
        self.assertEqual([(e["map"], e["opcode"], e["reg"])
                          for e in claims["setne"]], [("0f", 0x95, 0)])
        # FMA3 reads the opcode out of the mnemonic: `vfmadd132ps` is
        # VEX.66.0F38.W0 98, and `vfnmsub231sd` is `9e` + `0x20` for the 231
        # operand order + `1` for the scalar form, with W=1 for `sd`.
        self.assertEqual(self.gas.fma3_encoding("vfmadd132ps")["opcode"], 0x98)
        self.assertEqual(self.gas.fma3_encoding("vfnmsub231sd")["opcode"], 0xBF)
        self.assertIsNone(self.gas.fma3_encoding("vfmaddsub132ss"))

    def test_a_claim_the_oracle_cannot_confirm_is_a_finding(self):
        bogus = {"add": [self.gas._enc(0x0F)], "not_a_mnemonic":
                 [self.gas._enc(0x90)]}
        findings, checked = self.gas.verify_code_models(bogus, self.table)
        self.assertEqual(checked, 0)
        self.assertEqual(sorted(f["kind"] for f in findings),
                         ["CODE-EXTRA", "CODE-UNKNOWN"])
        self.assertIn("not_a_mnemonic",
                      [f["name"] for f in findings if f["kind"] == "CODE-UNKNOWN"])


class GasGapModelTests(unittest.TestCase):
    """The row model has to see the opcode the *encoder* derives, not one of it.

    Three defects of that shape were live: `missing()` modelled a row once with
    its first mnemonic (so `"rolq" | "roll" | "rolw" | "rolb"` credited every
    spelling with the first one's opcode), `D`/`W` were expanded one-dimensionally
    in the row model (so `testl` claimed `84`, the byte opcode), and `+rd` rows
    (`push`, `pop`, `bswap`, `mov $imm,%reg`, `inc`/`dec` in 32-bit mode) were
    modelled as a single opcode.  Each of them made the gap list quote encodings
    the encoder already emits.
    """

    @classmethod
    def setUpClass(cls):
        cls.gas = _load(CROSSREF, "gas_encoding_crossref")
        cls.sdm = _load(SDM, "sdm_isa_crossref")
        cls.table = cls.gas.load_snapshot()
        cls.rows = []
        for path in cls.sdm.encoder_files("all"):
            cls.rows.extend(cls.sdm.extract_rows(Path(path).resolve()))

    def _row(self, name):
        """The first encoder row that binds `name` (spellings differ by backend)."""
        for row in self.rows:
            if name in row["names"]:
                return row
        self.fail(f"no encoder row binds {name}")

    def test_each_spelling_of_a_shared_row_is_modelled_on_its_own(self):
        row = self._row("roll")
        by_name = {n: sorted(e["opcode"] for e in self.sdm.model_any(row, n))
                   for n in ("rolb", "roll") if n in row["names"]}
        self.assertIn("rolb", by_name)
        self.assertIn(0xD0, by_name["rolb"])
        self.assertNotIn(0xD0, by_name["roll"])
        self.assertIn(0xD1, by_name["roll"])

    def test_the_width_bit_moves_the_opcode(self):
        """`testl` is `85`/`A9`/`F7`, not the byte `84`/`A8`/`F6`."""
        row = self._row("testl")
        wide = {e["opcode"] for e in self.sdm.model_any(row, "testl")}
        narrow = {e["opcode"] for e in self.sdm.model_any(row, "testb")}
        self.assertLessEqual({0x85, 0xA9, 0xF7}, wide)
        self.assertLessEqual({0x84, 0xA8, 0xF6}, narrow)
        self.assertNotIn(0x84, wide)

    def test_register_in_opcode_rows_claim_the_whole_range(self):
        row = self._row("pushq")  # x86 dispatches pushq/pushl to one arm
        opcodes = {e["opcode"] for e in self.sdm.model_any(row, "pushq")}
        self.assertLessEqual(set(range(0x50, 0x58)), opcodes)
        for opcode in range(0x50, 0x58):
            self.assertTrue(
                any(e["opcode"] == opcode for e in self.table["push"]),
                f"push {opcode:02x} is claimed but not in the snapshot")

    def test_the_gap_report_does_not_count_prefixes_or_mm_x_twins(self):
        gaps = self.gas.missing(self.rows, self.table, self.sdm.model_any,
                                self.gas.code_models())
        for name in ("cs", "ds", "es", "ss", "fs", "gs", "movs", "cmps"):
            self.assertNotIn(name, gaps, name)


if __name__ == "__main__":
    unittest.main()
