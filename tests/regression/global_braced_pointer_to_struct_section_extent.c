/*
 * A braced initializer for a pointer-to-struct object must occupy exactly one
 * pointer slot.  The pointee layout must not leak into the initializer and pad
 * each slot to sizeof(struct payload).  Linux x86's .apicdrivers table uses
 * this exact shape and walks from __start to __stop one pointer at a time.
 */
struct payload {
    unsigned long words[28];
};

static struct payload a, b, c;

__attribute__((used, section("pointer_table")))
static const struct payload *slot_a = { &a };
__attribute__((used, section("pointer_table")))
static const struct payload *slot_b = { &b };
__attribute__((used, section("pointer_table")))
static const struct payload *slot_c = { &c };

extern const struct payload *__start_pointer_table[];
extern const struct payload *__stop_pointer_table[];

int main(void)
{
    if (__stop_pointer_table - __start_pointer_table != 3)
        return 1;
    unsigned seen = 0;
    for (unsigned i = 0; i != 3; ++i) {
        if (__start_pointer_table[i] == &a)
            seen |= 1;
        else if (__start_pointer_table[i] == &b)
            seen |= 2;
        else if (__start_pointer_table[i] == &c)
            seen |= 4;
        else
            return 2;
    }
    return seen != 7;
}
