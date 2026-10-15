int padding_witness(int value);
int main(void) {
    for (int value = -3; value <= 3; ++value) {
        if (padding_witness(value) != (value == 0))
            return 13 + value;
    }
    return 0;
}
