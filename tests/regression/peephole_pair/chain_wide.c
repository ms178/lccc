/* Wide dual-layout timing fixture: the same three block shapes as
 * peephole_pair_chain.c, replicated so one TU contains several explored
 * functions. Header-free, so x86-64, i686, AArch64 and RISC-V can all
 * preprocess it without a cross sysroot. Timing target for
 * scripts/measure_peephole_pair.py; the gate uses the compact probe. */

typedef unsigned long ulong;
typedef unsigned int uint;
typedef unsigned char uchar;

static uint g_state = 0x9e3779b9u;

static uint mix0(uint x)
{
    x ^= x >> 16;
    x *= 0x7feb352du;
    x ^= x >> 15;
    x *= 0x846ca68bu;
    x ^= x >> 16;
    return x + 0u;
}

int classify0(const uchar *data, ulong n)
{
    int score = 0;
    for (ulong k = 0; k < n; k++) {
        uint v = data[k];
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
            score ^= (int)mix0(v);
        }
    }
    return score;
}

ulong walk0(const uint *a, ulong n, ulong stride)
{
    ulong acc = 0;
    ulong k = 0;
    while (k < n) {
        uint v = a[k];
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
        k += stride ? stride : 1;
    }
    return acc;
}

static uint mix1(uint x)
{
    x ^= x >> 16;
    x *= 0x7feb352du;
    x ^= x >> 15;
    x *= 0x846ca68bu;
    x ^= x >> 16;
    return x + 1u;
}

int classify1(const uchar *data, ulong n)
{
    int score = 1;
    for (ulong k = 0; k < n; k++) {
        uint v = data[k];
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
            score ^= (int)mix1(v);
        }
    }
    return score;
}

ulong walk1(const uint *a, ulong n, ulong stride)
{
    ulong acc = 1;
    ulong k = 0;
    while (k < n) {
        uint v = a[k];
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
        k += stride ? stride : 1;
    }
    return acc;
}

static uint mix2(uint x)
{
    x ^= x >> 16;
    x *= 0x7feb352du;
    x ^= x >> 15;
    x *= 0x846ca68bu;
    x ^= x >> 16;
    return x + 2u;
}

int classify2(const uchar *data, ulong n)
{
    int score = 2;
    for (ulong k = 0; k < n; k++) {
        uint v = data[k];
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
            score ^= (int)mix2(v);
        }
    }
    return score;
}

ulong walk2(const uint *a, ulong n, ulong stride)
{
    ulong acc = 2;
    ulong k = 0;
    while (k < n) {
        uint v = a[k];
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
        k += stride ? stride : 1;
    }
    return acc;
}

static uint mix3(uint x)
{
    x ^= x >> 16;
    x *= 0x7feb352du;
    x ^= x >> 15;
    x *= 0x846ca68bu;
    x ^= x >> 16;
    return x + 3u;
}

int classify3(const uchar *data, ulong n)
{
    int score = 3;
    for (ulong k = 0; k < n; k++) {
        uint v = data[k];
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
            score ^= (int)mix3(v);
        }
    }
    return score;
}

ulong walk3(const uint *a, ulong n, ulong stride)
{
    ulong acc = 3;
    ulong k = 0;
    while (k < n) {
        uint v = a[k];
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
        k += stride ? stride : 1;
    }
    return acc;
}

static uint mix4(uint x)
{
    x ^= x >> 16;
    x *= 0x7feb352du;
    x ^= x >> 15;
    x *= 0x846ca68bu;
    x ^= x >> 16;
    return x + 4u;
}

int classify4(const uchar *data, ulong n)
{
    int score = 4;
    for (ulong k = 0; k < n; k++) {
        uint v = data[k];
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
            score ^= (int)mix4(v);
        }
    }
    return score;
}

ulong walk4(const uint *a, ulong n, ulong stride)
{
    ulong acc = 4;
    ulong k = 0;
    while (k < n) {
        uint v = a[k];
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
        k += stride ? stride : 1;
    }
    return acc;
}

static uint mix5(uint x)
{
    x ^= x >> 16;
    x *= 0x7feb352du;
    x ^= x >> 15;
    x *= 0x846ca68bu;
    x ^= x >> 16;
    return x + 5u;
}

int classify5(const uchar *data, ulong n)
{
    int score = 5;
    for (ulong k = 0; k < n; k++) {
        uint v = data[k];
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
            score ^= (int)mix5(v);
        }
    }
    return score;
}

ulong walk5(const uint *a, ulong n, ulong stride)
{
    ulong acc = 5;
    ulong k = 0;
    while (k < n) {
        uint v = a[k];
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
        k += stride ? stride : 1;
    }
    return acc;
}

static uint mix6(uint x)
{
    x ^= x >> 16;
    x *= 0x7feb352du;
    x ^= x >> 15;
    x *= 0x846ca68bu;
    x ^= x >> 16;
    return x + 6u;
}

int classify6(const uchar *data, ulong n)
{
    int score = 6;
    for (ulong k = 0; k < n; k++) {
        uint v = data[k];
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
            score ^= (int)mix6(v);
        }
    }
    return score;
}

ulong walk6(const uint *a, ulong n, ulong stride)
{
    ulong acc = 6;
    ulong k = 0;
    while (k < n) {
        uint v = a[k];
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
        k += stride ? stride : 1;
    }
    return acc;
}

static uint mix7(uint x)
{
    x ^= x >> 16;
    x *= 0x7feb352du;
    x ^= x >> 15;
    x *= 0x846ca68bu;
    x ^= x >> 16;
    return x + 7u;
}

int classify7(const uchar *data, ulong n)
{
    int score = 7;
    for (ulong k = 0; k < n; k++) {
        uint v = data[k];
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
            score ^= (int)mix7(v);
        }
    }
    return score;
}

ulong walk7(const uint *a, ulong n, ulong stride)
{
    ulong acc = 7;
    ulong k = 0;
    while (k < n) {
        uint v = a[k];
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
        k += stride ? stride : 1;
    }
    return acc;
}

static uint mix8(uint x)
{
    x ^= x >> 16;
    x *= 0x7feb352du;
    x ^= x >> 15;
    x *= 0x846ca68bu;
    x ^= x >> 16;
    return x + 8u;
}

int classify8(const uchar *data, ulong n)
{
    int score = 8;
    for (ulong k = 0; k < n; k++) {
        uint v = data[k];
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
            score ^= (int)mix8(v);
        }
    }
    return score;
}

ulong walk8(const uint *a, ulong n, ulong stride)
{
    ulong acc = 8;
    ulong k = 0;
    while (k < n) {
        uint v = a[k];
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
        k += stride ? stride : 1;
    }
    return acc;
}

static uint mix9(uint x)
{
    x ^= x >> 16;
    x *= 0x7feb352du;
    x ^= x >> 15;
    x *= 0x846ca68bu;
    x ^= x >> 16;
    return x + 9u;
}

int classify9(const uchar *data, ulong n)
{
    int score = 9;
    for (ulong k = 0; k < n; k++) {
        uint v = data[k];
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
            score ^= (int)mix9(v);
        }
    }
    return score;
}

ulong walk9(const uint *a, ulong n, ulong stride)
{
    ulong acc = 9;
    ulong k = 0;
    while (k < n) {
        uint v = a[k];
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
        k += stride ? stride : 1;
    }
    return acc;
}

ulong run_wide(void)
{
    uchar bytes[256];
    uint words[128];
    ulong acc = 0;
    for (uint i = 0; i < 256; i++) {
        bytes[i] = (uchar)(i * 7u + 3u);
    }
    for (uint i = 0; i < 128; i++) {
        words[i] = g_state + i;
    }
    acc += (ulong)classify0(bytes, 256) + walk0(words, 128, 4);
    acc += (ulong)classify1(bytes, 256) + walk1(words, 128, 4);
    acc += (ulong)classify2(bytes, 256) + walk2(words, 128, 4);
    acc += (ulong)classify3(bytes, 256) + walk3(words, 128, 4);
    acc += (ulong)classify4(bytes, 256) + walk4(words, 128, 4);
    acc += (ulong)classify5(bytes, 256) + walk5(words, 128, 4);
    acc += (ulong)classify6(bytes, 256) + walk6(words, 128, 4);
    acc += (ulong)classify7(bytes, 256) + walk7(words, 128, 4);
    acc += (ulong)classify8(bytes, 256) + walk8(words, 128, 4);
    acc += (ulong)classify9(bytes, 256) + walk9(words, 128, 4);
    return acc;
}
