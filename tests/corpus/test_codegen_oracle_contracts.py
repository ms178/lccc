"""Run the existing codegen metric contracts in hosted/local corpus discovery."""
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'scripts'))
import test_codegen_oracle


def load_tests(loader, tests, pattern):
    suite = loader.loadTestsFromModule(test_codegen_oracle)
    if not suite.countTestCases():
        raise RuntimeError('codegen oracle contract suite unexpectedly empty')
    return suite
