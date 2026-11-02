/* Register-offset symbol fold probe (oracle-corpus entry).
 *
 * The exact shape the fold exists for: a derived pointer `win_window + cur`
 * whose only uses are two VARIABLE-index accesses, so a compiler that can put
 * the symbol in the memory operand's displacement with BOTH the offset and
 * the index in registers emits four instructions where a materialising one
 * emits six.  Compared against GCC/Clang/ICX on the Godbolt oracle under the
 * pinned corpus flags; `check_reg_off_sym_fold.sh` pins the local behaviour
 * (kill switch, RA-01 dormancy, PIC refusal, GCC-identical output).
 */
unsigned char win_window[8192];

unsigned
reg_off_window_probe(unsigned cur, unsigned i, unsigned j)
{
    const unsigned char *m = win_window + cur;
    return m[i] + m[j];
}
