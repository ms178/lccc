int main(void) {
    int s = 0;
    for (int i = 0; i < 3; i++) s += (i + 1) * (i + 1);
    return s == 14 ? 0 : 1;
}
