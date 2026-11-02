unsigned long long vector_witness(unsigned long long value);
int main(void) {
    const unsigned long long values[] = {0, 1, 0x8000000000000000ULL, ~0ULL};
    for (unsigned i = 0; i < 4; ++i) {
        if (vector_witness(values[i]) != values[i])
            return 1;
    }
    return 0;
}
