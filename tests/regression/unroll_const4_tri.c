int main(void) {
    int s = 0;
    for (int i = 0; i < 4; i++)
        for (int j = i + 1; j < 4; j++)
            s += 1;
    return s == 6 ? 0 : 1;
}
