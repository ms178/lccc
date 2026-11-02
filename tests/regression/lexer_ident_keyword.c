/* Keywords vs identifiers still tokenize correctly after interning. */
int main(void) {
    int int_ = 1; /* identifier, not keyword */
    int x = 2;
    if (int_ + x != 3) return 1;
    return 0;
}
