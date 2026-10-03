"""Small exhaustive assignment oracle and dense matching resource regression."""
import itertools
from pathlib import Path
import sys
import time
import unittest

REPO = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO))
from tools.corpus import diagnostics


def expectation(i, minimum=0, maximum=None):
    return dict(id=str(i), kind='warning', regex_form=False, message=f':E{i}:',
                location=dict(file='$SOURCE', line=1, any_file=False),
                count_min=minimum, count_max=maximum)


def brute(edges, bounds, ignored=False):
    for assignment in itertools.product(range(-1, len(bounds)), repeat=len(edges)):
        if not ignored and -1 in assignment:
            continue
        if any(owner >= 0 and owner not in edges[i] for i, owner in enumerate(assignment)):
            continue
        counts = [assignment.count(i) for i in range(len(bounds))]
        if all(lo <= count and (hi is None or count <= hi)
               for count, (lo, hi) in zip(counts, bounds)):
            return True
    return False


class MatchingTests(unittest.TestCase):
    def test_all_three_by_three_adjacencies_and_bounds(self):
        bounds_cases = list(itertools.product(((0, 1), (1, 1), (0, None)), repeat=3))
        for mask in range(1 << 9):
            edges = [{e for e in range(3) if mask & (1 << (i * 3 + e))}
                     for i in range(3)]
            text = '\n'.join('x.c:1:1: warning: ' + ' '.join(f':E{e}:' for e in row)
                             for row in edges)
            observed = diagnostics.parse(text)
            for bounds in bounds_cases:
                expected = [expectation(i, *bound) for i, bound in enumerate(bounds)]
                wanted = brute(edges, bounds)
                actual = diagnostics.verify(expected, observed, source=Path('x.c'))
                self.assertEqual(actual['ok'], wanted, (mask, bounds, actual))

    def test_ignored_unexpected_still_requires_mandatory_occurrences(self):
        for mask in range(1 << 6):
            edges = [{e for e in range(2) if mask & (1 << (i * 2 + e))} for i in range(3)]
            text = '\n'.join('x.c:1:1: warning: ' + ' '.join(f':E{e}:' for e in row)
                             for row in edges)
            expected = [expectation(0, 1, 2), expectation(1, 1, 1)]
            actual = diagnostics.verify(expected, diagnostics.parse(text), source=Path('x.c'),
                                        ignore_unexpected=['warning'])
            self.assertEqual(actual['ok'], brute(edges, [(1, 2), (1, 1)], ignored=True))

    def test_dense_optional_matching_not_artificial_slot_failure(self):
        expected = [dict(expectation(i), message='') for i in range(128)]
        observed = diagnostics.parse('\n'.join('x.c:1:1: warning: x' for _ in range(512)))
        start = time.monotonic()
        actual = diagnostics.verify(expected, observed, source=Path('x.c'))
        self.assertTrue(actual['ok'], actual)
        self.assertEqual(len(actual['matches']), 512)
        self.assertLess(time.monotonic() - start, 2)

    def test_no_diagnostics_cannot_bypass_resource_limit(self):
        expected = [dict(expectation(0), kind='no-diagnostics')]
        observed = diagnostics.parse('\n'.join('x.c:1:1: warning: x' for _ in range(513)))
        self.assertIn('resource budget', diagnostics.verify(expected, observed,
                       source=Path('x.c'))['detail'])


if __name__ == '__main__':
    unittest.main()
