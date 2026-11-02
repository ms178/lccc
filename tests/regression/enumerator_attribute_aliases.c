/* Enumerator attributes must not invent constants or replace previous values.
 * This is the LZ4 obsolete-enum header shape, independent of system headers. */
enum sizes { block64 = 4, block256 = 5, block4m = 7,
             old64 __attribute__((deprecated)) = block64,
             old4m __attribute__((deprecated("use block4m"))) = block4m };
enum modes { linked = 0, independent,
             old_linked __attribute__((deprecated)) = linked };
int main(void) {
    return block64 != 4 || block256 != 5 || block4m != 7 || linked != 0 || independent != 1;
}
