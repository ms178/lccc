#!/usr/bin/env python3
"""Independent modular check of the bounded unsigned exit-test proof.

All 4-bit seeds/addends/bounds/steps and both predicates are enumerated;
300,000 seeded 8-bit samples supplement them. This is not an exhaustive
32/64-bit compiler verification. Rust tests check admission/CFG restrictions.
"""
import itertools
import random


def check(bits, cases):
    maximum = (1 << bits) - 1
    total = admitted = 0
    for start, addend, bound, step, inclusive in cases:
        total += 1
        threshold = bound + int(inclusive) - addend
        new_bound = bound - addend
        if step == 0 or not 0 <= threshold <= maximum or not 0 <= new_bound <= maximum:
            continue
        if start + addend > maximum or start >= threshold:
            continue
        last = start + ((threshold - start + step - 1) // step) * step
        if last + addend > maximum:
            continue
        admitted += 1
        value = start
        for _ in range(maximum + 2):
            wrapped = (value + addend) & maximum
            original = wrapped <= bound if inclusive else wrapped < bound
            rewritten = value <= new_bound if inclusive else value < new_bound
            assert original == rewritten, (bits, start, addend, bound, step, inclusive, value)
            if not original:
                break
            value = (value + step) & maximum
        else:
            raise AssertionError('proof admitted a nonterminating loop')
    assert admitted > 0
    print(f'unsigned affine model: {bits}-bit {total} cases, {admitted} admitted; every observed condition identical')


if __name__ == '__main__':
    check(4, (args + (le,) for args in itertools.product(range(16), repeat=4) for le in (False, True)))
    rng = random.Random(824)
    check(8, (tuple(rng.randrange(256) for _ in range(4)) + (bool(rng.randrange(2)),) for _ in range(300000)))
