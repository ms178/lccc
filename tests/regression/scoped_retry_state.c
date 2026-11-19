/* Model a scoped seqlock read: the loop guard excludes done, even across
 * a memory barrier and a call. The default null pointer fields are filled
 * as part of aggregate initialization; they must not block promotion. */
enum mode { done, held, held_saved, lockless };
struct state {
    enum mode mode;
    unsigned long data;
    int *lock;
    int *lock_saved;
};

/* Undefined by design: linking requires eliminating only the impossible
 * case. The two executable paths below still test the cleanup and retry. */
extern void impossible_done_state(void);
static volatile int need_retry;
static volatile unsigned observations;

static __attribute__((noinline)) unsigned long begin(int *lock)
{
    return (unsigned long)*lock;
}

static __attribute__((noinline)) int retry(int *lock, unsigned long data)
{
    return need_retry && data == (unsigned long)*lock;
}

static __attribute__((noinline)) unsigned long take(int *lock)
{
    *lock += 10;
    return (unsigned long)*lock;
}

static __attribute__((noinline)) void give(int *lock, unsigned long data)
{
    *lock += data == (unsigned long)*lock ? 100 : 1;
}

static __attribute__((noinline)) void observe(void)
{
    ++observations;
}

static inline __attribute__((always_inline)) void cleanup(struct state *s)
{
    if (s->lock)
        give(s->lock, s->data);
    if (s->lock_saved)
        give(s->lock_saved, s->data);
}

static inline __attribute__((always_inline)) void advance(struct state *s, int *lock)
{
    switch (s->mode) {
    case done:
        impossible_done_state();
        return;
    case held:
    case held_saved:
        s->mode = done;
        return;
    case lockless:
        if (!retry(lock, s->data)) {
            s->mode = done;
            return;
        }
        break;
    }
    s->lock_saved = lock;
    s->data = take(lock);
    s->mode = held_saved;
}

__attribute__((noinline)) void run(int *lock)
{
    for (struct state s __attribute__((cleanup(cleanup))) = {
             .mode = lockless, .data = begin(lock),
         };
         s.mode != done; advance(&s, lock)) {
        asm volatile("" ::: "memory");
        observe();
    }
}

int main(void)
{
    int lock = 7;
    run(&lock);
    if (lock != 7 || observations != 1)
        return 1;
    need_retry = 1;
    run(&lock);
    return lock != 117 || observations != 3;
}
