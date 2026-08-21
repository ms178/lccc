/* RA-01 oracle: file-scope data addresses and loop-carried match state.
 * Derived from the access shape of GNU gzip 1.14 longest_match, not its code. */
typedef unsigned char u8;
typedef unsigned short u16;

u8 window[65536];
u16 prev[32768];
unsigned strstart;
unsigned prev_length;
unsigned max_chain_length;
unsigned good_match;
unsigned match_start;

unsigned global_match_probe(unsigned cur_match)
{
    unsigned chain = max_chain_length;
    u8 *scan = window + strstart;
    unsigned best = prev_length;
    unsigned limit = strstart > 32506 ? strstart - 32506 : 0;
    u8 end0 = scan[best - 1];
    u8 end1 = scan[best];

    if (best >= good_match)
        chain >>= 2;
    do {
        u8 *match = window + cur_match;
        if (match[best] == end1 && match[best - 1] == end0
            && match[0] == scan[0] && match[1] == scan[1]) {
            unsigned len = 2;
            while (len < 258 && match[len] == scan[len])
                ++len;
            if (len > best) {
                match_start = cur_match;
                best = len;
            }
        }
        cur_match = prev[cur_match & 32767];
    } while (cur_match > limit && --chain != 0);
    return best;
}
