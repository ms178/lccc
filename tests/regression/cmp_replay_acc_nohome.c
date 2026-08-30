/* cmp-replay vs accumulator-located operand (io_uring kbuf ICE).
 *
 * Extracted from linux 6.18.47 io_uring/kbuf.c io_ring_buffers_peek() with
 * ABI-faithful stub types (same field offsets: buf_ring->tail @14,
 * bl->head @24, bl->mask @28, arg->max_len @16, arg->mode @26, buf->len @8,
 * buf->addr @0, buf->bid @12) so the same GEP/cast/select IR shape and
 * register pressure emerge at -O2.
 *
 * The zext (Cast U16->U64 of nr_avail) is single-use and adjacent to its
 * Cmp, so it is accumulator-located: no register, no slot, home store
 * skipped. The Cmp's single use (the u16 trunc Select) is NOT adjacent —
 * the Cast U64->U16 of `needed` sits between — so compare-replay deferred
 * the entire Cmp to the Select, which then tried to re-read the zext at a
 * distance and hit the operand_to_rax hard gate ("no register home, no
 * stack slot and no acc-cache entry"). Before the IS-09 accumulator
 * pruning fix this was an internal error compiling the kernel's
 * io_uring/kbuf.c at -O2.
 */
#include <stdio.h>
#include <stdlib.h>

typedef unsigned short __u16;
typedef unsigned int __u32;
typedef unsigned long long __u64;
typedef long long __s64;

struct io_uring_buf {
    __u64 addr;      /* @0 */
    __u32 len;       /* @8 */
    __u16 bid;       /* @12 */
    __u16 resv;      /* @14 */
};

struct io_uring_buf_ring {
    __u64 resv[2];   /* 16 bytes of tail-calls struct padding */
    __u16 tail;      /* @16->mapped via pointer at offset 14 below */
};

struct io_buffer_list {
    void *pad0;      /* @0  (bl is not a buf_ring list here) */
    void *buf_ring;  /* @8  -> struct io_uring_buf_ring* */
    __u64 max_len_x; /* @16 */
    __u16 head;      /* @24 */
    __u16 pad;       /* @26 -> mode overlay via arg */
    __u32 mask;      /* @28 */
};

struct buf_sel_arg {
    void *pad0;      /* @0 */
    void *iovs;      /* @8 */
    __u64 max_len;   /* @16 */
    __u32 nr_iovs;   /* @24 */
    __u32 pad1;      /* @28 */
    __u32 buf_group; /* @32 */
    __u32 mode;      /* @36 */
    __u32 out_len;   /* @40 */
    __u32 partial_map; /* @44 */
};

struct io_kiocb {
    __u64 pad0;      /* @0 */
    void *ctx;       /* @8 */
    __u64 pad1;      /* @16 */
    __u32 buf_index; /* @24 */
    __u32 flags;     /* @28 */
};

struct iovec { void *iov_base; __u64 iov_len; };

#define UIO_MAXIOV      1024
#define PEEK_MAX_IMPORT 256
#define ENOBUFS 105
#define ENOMEM  12
#define INT_MAX  2147483647
#define KBUF_MODE_EXPAND 1
#define KBUF_MODE_FREE   2
#define IOBL_INC         4
#define REQ_F_BUFFER_RING 0x1000
#define REQ_F_BL_EMPTY    0x2000

#define READ_ONCE(x)       (*(volatile __typeof__(x) *)&(x))
#define min_not_zero(a, b) ((a) == 0 ? (b) : ((a) < (b) ? (a) : (b)))
#define min_t(t, a, b)     ((t)((a) < (b) ? (a) : (b)))

static inline struct io_uring_buf *io_ring_head_to_buf(struct io_uring_buf_ring *br,
                                                       __u16 idx, __u32 mask)
{
    struct io_uring_buf *bufs = (void *)((char *)br + 16);
    return &bufs[(idx & mask) >> 2];
}

__attribute__((noinline))
static int io_ring_buffers_peek(struct io_kiocb *req, struct buf_sel_arg *arg,
                                struct io_buffer_list *bl)
{
    char *br = (char *)bl->buf_ring;
    volatile __u16 *br_tail = (volatile __u16 *)(br + 14);
    struct iovec *iov = (struct iovec *)arg->iovs;
    int nr_iovs = (int)arg->nr_iovs;
    __u16 nr_avail, tail, head;
    struct io_uring_buf *buf;

    tail = *br_tail;
    head = bl->head;
    nr_avail = min_t(__u16, tail - head, UIO_MAXIOV);
    if (!nr_avail)
        return -ENOBUFS;

    buf = io_ring_head_to_buf((void *)br, head, bl->mask);
    if (arg->max_len) {
        {
            __u32 len = READ_ONCE(buf->len);
            __u64 needed;

            if (!len)
                return -ENOBUFS;
            needed = (arg->max_len + len - 1) / len;
            needed = min_not_zero(needed, (__u64) PEEK_MAX_IMPORT);
            if (nr_avail > needed)
                nr_avail = (__u16)needed;
        }
    }

    if ((arg->mode & KBUF_MODE_EXPAND) && nr_avail > nr_iovs && arg->max_len) {
        iov = (struct iovec *)calloc(nr_avail, sizeof(struct iovec));
        if (!iov)
            return -ENOMEM;
        if (arg->mode & KBUF_MODE_FREE)
            free(arg->iovs);
        arg->iovs = iov;
        nr_iovs = nr_avail;
    } else if (nr_avail < nr_iovs) {
        nr_iovs = nr_avail;
    }

    if (!arg->max_len)
        arg->max_len = INT_MAX;

    req->buf_index = READ_ONCE(buf->bid);
    do {
        __u32 len = READ_ONCE(buf->len);

        if (len > arg->max_len) {
            len = (__u32)arg->max_len;
            if (!(bl->mask & IOBL_INC)) {
                arg->partial_map = 1;
                if ((void *)iov != arg->iovs)
                    break;
            }
        }

        iov->iov_base = (void *)(__s64)READ_ONCE(buf->addr);
        iov->iov_len = len;
        iov++;

        arg->out_len += len;
        arg->max_len -= len;
        if (!arg->max_len)
            break;

        buf = io_ring_head_to_buf((void *)br, ++head, bl->mask);
    } while (--nr_iovs);

    if (head == tail)
        req->flags |= REQ_F_BL_EMPTY;

    req->flags |= REQ_F_BUFFER_RING;
    return (int)((void *)iov - arg->iovs) / (int)sizeof(struct iovec);
}

int main(void)
{
    /* buf_ring: 14 bytes pad + tail + ring array */
    static unsigned char ring[64] __attribute__((aligned(8)));
    struct io_uring_buf_ring *brp = (void *)ring;
    volatile __u16 *t = (volatile __u16 *)(ring + 14);
    struct io_uring_buf *bufs = (void *)(ring + 16);
    *t = 8;
    for (int i = 0; i < 4; i++) {
        bufs[i].addr = 0x1000 + i * 16;
        bufs[i].len = 4;
        bufs[i].bid = i;
    }

    struct io_kiocb req = { 0 };
    struct iovec iovs[16] = { 0 };
    struct buf_sel_arg arg = { 0 };
    arg.iovs = iovs;
    arg.nr_iovs = 16;
    arg.max_len = 10;
    arg.mode = 0;
    struct io_buffer_list bl = { 0 };
    bl.buf_ring = brp;
    bl.head = 0;
    bl.mask = 0xf0; /* (idx & mask) >> 2 */

    int r1 = io_ring_buffers_peek(&req, &arg, &bl);

    /* max_len == 0 path (sets INT_MAX) */
    struct buf_sel_arg arg2 = { 0 };
    struct iovec iovs2[16] = { 0 };
    arg2.iovs = iovs2;
    arg2.nr_iovs = 16;
    arg2.max_len = 0;
    struct io_kiocb req2 = { 0 };
    int r2 = io_ring_buffers_peek(&req2, &arg2, &bl);

    /* zero available path */
    struct io_buffer_list bl0 = bl;
    bl0.head = 8;
    int r3 = io_ring_buffers_peek(&req, &arg, &bl0);

    /* empty len path */
    bufs[0].len = 0;
    bl.head = 0;
    int r4 = io_ring_buffers_peek(&req, &arg, &bl);
    bufs[0].len = 4;

    printf("%d %d %d %d %u %u %llu\n", r1, r2, r3, r4,
           req.buf_index, req.flags, (unsigned long long)arg.out_len);
    return 0;
}
