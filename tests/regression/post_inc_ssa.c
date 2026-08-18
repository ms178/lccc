/* Post-inc/dec must return the pre-update SSA value without a spill home. */
struct bits {
    unsigned value : 5;
};

static int integer_cases(void)
{
    int x = 7;
    int old = x++;
    int n = 5;
    int sum = 0;

    while (n--)
        sum += n;

    return old == 7 && x == 8 && n == -1 && sum == 10;
}

static int pointer_cases(void)
{
    int values[] = { 11, 22, 33 };
    int *p = values;
    int first = *p++;
    int *old = p++;

    return first == 11 && *old == 22 && p == values + 2;
}

static int narrow_and_volatile_cases(void)
{
    unsigned char byte = 255;
    unsigned char old_byte = byte++;
    volatile int value = 41;
    int old_value = value++;
    struct bits b = { 31 };
    unsigned old_bits = b.value--;

    return old_byte == 255 && byte == 0 &&
           old_value == 41 && value == 42 &&
           old_bits == 31 && b.value == 30;
}

int main(void)
{
    return !(integer_cases() && pointer_cases() && narrow_and_volatile_cases());
}
