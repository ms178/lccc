/* Dual-layout exploration probe: no headers, so every backend can preprocess
 * it without a cross sysroot. The shapes below are chosen to make the static
 * chain reorder blocks (chained conditionals and loops with several
 * successors) and to emit fragments large enough that a metric pair is worth
 * splitting on every architecture. Used by
 * tests/regression/check_peephole_pair_parallel.py. */

typedef unsigned long ulong;
typedef unsigned int uint;
typedef unsigned char uchar;

static uint state = 0x9e3779b9u;

static uint mix(uint x)
{
    x ^= x >> 16;
    x *= 0x7feb352du;
    x ^= x >> 15;
    x *= 0x846ca68bu;
    x ^= x >> 16;
    return x;
}

/* Chained compare/branch ladder: each arm is a distinct successor block. */
int classify_chain(const uchar *data, ulong n)
{
    int score = 0;
    for (ulong i = 0; i < n; i++) {
        uint v = data[i];
        if (v < 8) {
            score += 1;
        } else if (v < 16) {
            score += 3;
        } else if (v < 32) {
            score += 5;
        } else if (v < 64) {
            score += 7;
        } else if (v < 128) {
            score += 11;
        } else {
            score += 13;
        }
        if ((v & 1u) != 0) {
            score ^= (int)mix(v);
        }
    }
    return score;
}

/* Two nested loops with early exits: multiple backedges and cold tails. */
ulong reduce_walk(const uint *a, ulong n, ulong stride)
{
    ulong acc = 0;
    ulong i = 0;
    while (i < n) {
        uint v = a[i];
        if (v == 0) {
            break;
        }
        ulong j = 0;
        while (j < stride) {
            if ((v >> (j & 15)) & 1u) {
                acc += j + 1;
            } else if (j & 1u) {
                acc ^= j;
            } else {
                acc -= 1;
            }
            j++;
        }
        i += stride ? stride : 1;
    }
    return acc;
}

/* Switch-like ladder expressed as a chain, plus a loop-carried dependency. */
uint step_machine(uint op, uint a, uint b)
{
    uint r;
    if (op < 4) {
        if (op == 0) {
            r = a + b;
        } else if (op == 1) {
            r = a - b;
        } else if (op == 2) {
            r = a & b;
        } else {
            r = a | b;
        }
    } else if (op < 8) {
        if (op == 4) {
            r = a ^ b;
        } else if (op == 5) {
            r = a << (b & 31u);
        } else if (op == 6) {
            r = b ? (a >> (b & 31u)) : a;
        } else {
            r = mix(a ^ b);
        }
    } else {
        r = mix(a) + mix(b);
    }
    return r;
}

ulong run_probe(void)
{
    uchar bytes[256];
    uint words[128];
    for (uint i = 0; i < 256; i++) {
        bytes[i] = (uchar)(i * 7u + 3u);
    }
    for (uint i = 0; i < 128; i++) {
        words[i] = mix(i + state);
    }
    ulong acc = (ulong)classify_chain(bytes, 256);
    acc += reduce_walk(words, 128, 4);
    uint s = state;
    for (uint i = 0; i < 32; i++) {
        s = step_machine(s & 15u, s, words[i & 127]);
    }
    return acc + s;
}
