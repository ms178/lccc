int main(void) {
    int s = 0;
    for (int i = 0; i < 4; i++)
        for (int j = 0; j < 4; j++)
            s += i * 4 + j;
    return s == 120 ? 0 : 1;
}
