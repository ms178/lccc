/* Late segment-fill must never assign a register home to a Copy destination
 * whose materialization the stack-layout copy-alias layer suppresses.
 *
 * The original segment-fill prototype assigned the second loop's `i` Copy to
 * %esi after copy aliases had been decided. No move populated %esi; the GEP
 * then used a stale stack slot as its index and crashed at -m32 -O0. This is a
 * reduced alias_fuzz_m32 seed 3, retaining the two adjacent fs[i] loads and
 * the loop-carried Copy web that exposed the location-model mismatch. */
typedef unsigned int u32;

static float fs[32];
static u32 hash = 2166136261u;

static void mix(u32 value)
{
    hash = (hash ^ value) * 16777619u;
}

static u32 bits(float value)
{
    union {
        float f;
        u32 u;
    } pun = { .f = value };
    return pun.u;
}

static u32 probe(void)
{
    for (int i = 0; i < 31; ++i)
        fs[i] = (float)i * 1.5f;

    float sum = 0.0f;
    for (int i = 1; i < 31; ++i) {
        sum += fs[i] + fs[i];
        fs[i - 1] = fs[i] + 1.0f;
    }

    mix(bits(sum));
    for (int i = 0; i < 32; ++i)
        mix(bits(fs[i]));
    return hash;
}

int main(void)
{
    return probe() != 0x3314789fu;
}
