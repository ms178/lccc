typedef short v8si __attribute__((vector_size(16)));

v8si counts = {0, 1, 2, 3, 4, 5, 6, 7};

int main(void)
{
    v8si left = 2 << counts;
    v8si right = 2 >> counts;
    __typeof__(2 << counts) typed = left;

    if (sizeof(2 << counts) != sizeof(v8si))
        return 1;
    for (int i = 0; i < 8; ++i) {
        if (left[i] != (short)(2 << counts[i]))
            return 2;
        if (right[i] != (short)(2 >> counts[i]))
            return 3;
        if (typed[i] != left[i])
            return 4;
    }
    return 0;
}
