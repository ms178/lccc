#!/usr/bin/env python3
"""Static mirrored tooling gates and discovery: no compiler process runs."""
from pathlib import Path
import shlex
import unittest

REPO=Path(__file__).resolve().parents[2]
COMMANDS=["python3 scripts/edg_changes_mine.py selftest","python3 scripts/edg_corpus_mine.py selftest",
          "python3 scripts/edg_corpus_mine.py verify-index","python3 scripts/edg_changes_mine.py verify-artifacts",
          "python3 -m unittest discover -s tests/corpus -p 'test_*.py'","python3 scripts/test_differential_corpus_paths.py"]


def executable_commands(text):
    rows=set()
    for line in text.replace('\\\n',' ').splitlines():
        line=line.strip()
        if not line.startswith(('python3 ','gate ')):continue
        tokens=shlex.split(line)
        if tokens and tokens[0]=='gate':tokens=tokens[3:]
        if tokens and tokens[0]=='python3':rows.add(tuple(tokens))
    return rows


class CiContract(unittest.TestCase):
    def test_three_entrypoints_use_complete_discovery_and_strict_artifacts(self):
        for path in [REPO/'scripts/ci_local.sh',REPO/'.github/workflows/ci.yml',REPO/'scripts/ci_corpus_tools.sh']:
            rows=executable_commands(path.read_text())
            for command in COMMANDS:self.assertIn(tuple(shlex.split(command)),rows,str(path))
    def test_mutated_discovery_or_integrity_command_rejected(self):
        text=(REPO/'scripts/ci_corpus_tools.sh').read_text()
        for command in COMMANDS:
            for replacement in ('# '+command,'echo '+command,command.replace('test_*.py','test_runner_contracts.py') if 'test_*.py' in command else command+' || true'):
                rows=executable_commands(text.replace(command,replacement,1))
                self.assertNotIn(tuple(shlex.split(command)),rows)
    def test_no_hosted_native_corpus_execution(self):
        for path in (REPO/'.github/workflows').glob('*.yml'):
            for line in path.read_text().splitlines():
                if not line.lstrip().startswith('#'):self.assertNotIn('run_clang_c_corpus.py',line)
    def test_standalone_gate_no_compiler_or_install_commands(self):
        rows=executable_commands((REPO/'scripts/ci_corpus_tools.sh').read_text())
        self.assertTrue(rows)
        self.assertTrue(all(row[0]=='python3' for row in rows))
        for forbidden in ('cargo build','build_lccc','apt-get','rustup','callgrind_ab.py','run_clang_c_corpus.py'):
            self.assertNotIn(forbidden,(REPO/'scripts/ci_corpus_tools.sh').read_text())

if __name__=='__main__':unittest.main()
