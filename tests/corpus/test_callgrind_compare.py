"""Execute the legacy shell driver's failure protocol without a compiler or PMU."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class ComparisonProtocolTests(unittest.TestCase):
    def run_driver(self, mode="ok", only="probe"):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            tools = root / "tools/oracle"
            tools.mkdir(parents=True)
            shutil.copy(ROOT / "tools/oracle/callgrind_compare.sh", tools)
            shutil.copytree(ROOT / "tools/corpus", root / "tools/corpus")
            corpus = root / "tests/oracle/programs"
            corpus.mkdir(parents=True)
            (corpus / "probe.c").write_text("int main(void) { return 0; }\n")
            bindir = root / "bin"
            bindir.mkdir()
            compiler = '''#!/usr/bin/env python3
import os,sys
from pathlib import Path
if os.environ['MODE']=='build': raise SystemExit(1)
out=Path(sys.argv[sys.argv.index('-o')+1])
text='different' if os.environ['MODE']=='mismatch' and Path(sys.argv[0]).name=='ref' else 'same'
code=1 if os.environ['MODE']=='execute' else 0
out.write_text('#!/bin/sh\\necho '+text+'\\nexit '+str(code)+'\\n')
out.chmod(0o755)
'''
            valgrind = '''#!/usr/bin/env python3
import os,subprocess,sys
from pathlib import Path
if os.environ['MODE']=='instrument': raise SystemExit(1)
path=next(x.split('=',1)[1] for x in sys.argv if x.startswith('--callgrind-out-file='))
counter=Path(path+'.calls')
count=int(counter.read_text())+1 if counter.exists() else 1
counter.write_text(str(count))
if os.environ['MODE']!='missing' and not (os.environ['MODE']=='stale' and count==2):
    Path(path).write_text('counters')
if os.environ['MODE']=='instrument-output': print('different'); raise SystemExit(0)
raise SystemExit(subprocess.run([sys.argv[-1]]).returncode)
'''
            for name, text in [("mine", compiler), ("ref", compiler), ("valgrind", valgrind)]:
                p = bindir / name
                p.write_text(text)
                p.chmod(0o755)
            (tools / "callgrind_own_ir.py").write_text('''import json,os,sys
from pathlib import Path
if os.environ['MODE']=='parse' or not Path(sys.argv[-1]).is_file(): raise SystemExit(1)
print(json.dumps({'totals':{'Ir':0 if os.environ['MODE']=='empty' else 100,'D1mr':2,'DLmr':1}}))
''')
            dest = root / "result.json"
            env = dict(os.environ, MODE=mode, LCCC=str(bindir / "mine"), PATH=str(bindir)+os.pathsep+os.environ["PATH"])
            result = subprocess.run(["bash", str(tools / "callgrind_compare.sh"), "--ref", "ref", "--only", only, "--json", str(dest)], env=env, capture_output=True, text=True, timeout=20)
            return result, dest.read_text() if dest.exists() else None

    def test_success_publishes_real_rows(self):
        import json
        result, report = self.run_driver()
        self.assertEqual(result.returncode, 0, result.stderr)
        row = json.loads(report)["rows"][0]
        self.assertEqual(row["ir"], 100)
        self.assertEqual(row["ratio_vs_ref"], 1.0)

    def test_failures_never_publish_rankings(self):
        for mode in ["build", "execute", "instrument", "instrument-output", "parse", "empty", "mismatch", "missing", "stale"]:
            with self.subTest(mode=mode):
                result, report = self.run_driver(mode)
                self.assertNotEqual(result.returncode, 0, result.stdout)
                self.assertIsNone(report)
                self.assertIn("FAIL:", result.stderr)

    def test_unknown_selection_is_not_success(self):
        result, report = self.run_driver(only="typo")
        self.assertNotEqual(result.returncode, 0)
        self.assertIsNone(report)


if __name__ == "__main__":
    unittest.main()
