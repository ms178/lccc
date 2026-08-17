/* Adjacent string literals in _Static_assert message must concatenate. */
int main(void) {
    _Static_assert(1, "a" "b" "c");
    return 0;
}
