/*
 * lccc_icount.c — deterministic guest instruction counter (QEMU TCG plugin).
 *
 * WHY THIS EXISTS
 * ---------------
 * The research box has no hardware PMU (virtualised, no perf event
 * passthrough, and wall-clock measurements under TCG are dominated by host
 * jitter).  Runtime is therefore useless as a code-quality signal here.
 *
 * Under TCG with `-icount`, guest execution is *deterministic*: the same
 * binary executing the same workload retires exactly the same instruction
 * sequence, every run.  Counting those instructions gives an exact,
 * zero-variance metric that is directly proportional to code quality for a
 * fixed workload — which is precisely what is needed to compare a kernel (or
 * a benchmark) built by LCCC against the same source built by GCC/Clang.
 *
 * WHAT IT MEASURES
 * ----------------
 *   * total guest instructions retired (whole VM)
 *   * instructions per optionally-named virtual address range, e.g. one
 *     function or one object file's text, passed as name=START:END in hex
 *
 * Cost model note: a per-instruction callback is far too slow.  The plugin
 * registers ONE execution callback on the LAST instruction of each translated
 * block and credits the block's whole instruction count when it executes —
 * the standard TCG-plugin accounting trick.  A block is attributed to the
 * range containing its first instruction; blocks spanning a boundary are
 * attributed by entry address (documented, and exact for range boundaries
 * placed on function symbols, which is the intended usage).
 *
 * USAGE
 * -----
 *   qemu-system-x86_64 -plugin ./lccc_icount.so,out=/tmp/counts.json \
 *       -plugin-arg ...            # (args are passed after the comma)
 *
 * Build:
 *   gcc -shared -fPIC -O2 -o lccc_icount.so lccc_icount.c \
 *       -I<dir containing qemu-plugin.h>
 *
 * This file is MIT-licensed; qemu-plugin.h itself is GPLv2+ (QEMU).
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <inttypes.h>

#include "qemu-plugin.h"

QEMU_PLUGIN_EXPORT int qemu_plugin_version = QEMU_PLUGIN_VERSION;

#define MAX_RANGES 32

typedef struct {
    char     name[64];
    uint64_t start;
    uint64_t end;
} range_t;

static range_t   ranges[MAX_RANGES];
static int       nranges;

static uint64_t  g_total_insns;
static uint64_t  g_total_tbs;
static uint64_t  g_range_insns[MAX_RANGES];
static const char *g_out_path;

/* Accounting payload for one translated block. */
typedef struct {
    uint32_t n_insns;
    int32_t  range_idx;   /* -1 = outside every named range */
} tb_info_t;

static void
tb_exec(unsigned int vcpu_index, void *userdata)
{
    tb_info_t *ti = (tb_info_t *)userdata;
    (void)vcpu_index;

    __atomic_add_fetch(&g_total_insns, ti->n_insns, __ATOMIC_RELAXED);
    __atomic_add_fetch(&g_total_tbs, 1, __ATOMIC_RELAXED);
    if (ti->range_idx >= 0) {
        __atomic_add_fetch(&g_range_insns[ti->range_idx], ti->n_insns,
                           __ATOMIC_RELAXED);
    }
}

static void
tb_trans(qemu_plugin_id_t id, struct qemu_plugin_tb *tb)
{
    size_t n;
    uint64_t vaddr;
    tb_info_t *ti;
    int i;

    (void)id;
    n = qemu_plugin_tb_n_insns(tb);
    if (n == 0) {
        return;
    }

    vaddr = qemu_plugin_insn_vaddr(qemu_plugin_tb_get_insn(tb, 0));

    ti = (tb_info_t *)malloc(sizeof(*ti));
    if (!ti) {
        return;
    }
    ti->n_insns = (uint32_t)n;
    ti->range_idx = -1;
    for (i = 0; i < nranges; i++) {
        if (vaddr >= ranges[i].start && vaddr < ranges[i].end) {
            ti->range_idx = i;
            break;
        }
    }

    qemu_plugin_register_vcpu_insn_exec_cb(
        qemu_plugin_tb_get_insn(tb, n - 1),
        tb_exec, QEMU_PLUGIN_CB_NO_REGS, ti);
}

static void
report(qemu_plugin_id_t id, void *userdata)
{
    FILE *f;
    int i;
    char line[256];

    (void)id;
    (void)userdata;

    f = g_out_path ? fopen(g_out_path, "w") : NULL;
    if (!f) {
        f = stdout;
    }

    fprintf(f, "{\n");
    fprintf(f, "  \"total_insns\": %" PRIu64 ",\n", g_total_insns);
    fprintf(f, "  \"total_blocks\": %" PRIu64 ",\n", g_total_tbs);
    fprintf(f, "  \"ranges\": {\n");
    for (i = 0; i < nranges; i++) {
        fprintf(f, "    \"%s\": {\"start\": \"0x%" PRIx64 "\", "
                   "\"end\": \"0x%" PRIx64 "\", \"insns\": %" PRIu64 "}%s\n",
                ranges[i].name, ranges[i].start, ranges[i].end,
                g_range_insns[i], (i + 1 < nranges) ? "," : "");
    }
    fprintf(f, "  }\n");
    fprintf(f, "}\n");

    /* Human-readable mirror on the console/monitor. */
    snprintf(line, sizeof(line),
             "LCCC_ICOUNT total_insns=%" PRIu64 " blocks=%" PRIu64 "\n",
             g_total_insns, g_total_tbs);
    qemu_plugin_outs(line);
    for (i = 0; i < nranges; i++) {
        /* %.63s bounds the copy to the 64-byte name field, so the 256-byte
         * buffer can never be overrun regardless of the range count. */
        snprintf(line, sizeof(line),
                 "LCCC_ICOUNT range %.63s [0x%" PRIx64 ",0x%" PRIx64 ") = %"
                 PRIu64 "\n",
                 ranges[i].name, ranges[i].start, ranges[i].end,
                 g_range_insns[i]);
        qemu_plugin_outs(line);
    }

    if (f != stdout) {
        fclose(f);
    }
}

QEMU_PLUGIN_EXPORT int
qemu_plugin_install(qemu_plugin_id_t id, const qemu_info_t *info,
                    int argc, char **argv)
{
    int i;

    (void)info;

    for (i = 0; i < argc; i++) {
        if (strncmp(argv[i], "out=", 4) == 0) {
            g_out_path = argv[i] + 4;
        } else if (strchr(argv[i], '=') && strchr(argv[i], ':')) {
            /* name=0xSTART:0xEND */
            const char *eq = strchr(argv[i], '=');
            const char *colon = strchr(eq + 1, ':');
            if (nranges < MAX_RANGES && eq && colon) {
                range_t *r = &ranges[nranges++];
                size_t nlen = (size_t)(eq - argv[i]);
                if (nlen >= sizeof(r->name)) {
                    nlen = sizeof(r->name) - 1;
                }
                memcpy(r->name, argv[i], nlen);
                r->name[nlen] = '\0';
                r->start = strtoull(eq + 1, NULL, 0);
                r->end   = strtoull(colon + 1, NULL, 0);
            }
        }
    }

    g_total_insns = 0;
    g_total_tbs = 0;
    memset(g_range_insns, 0, sizeof(g_range_insns));

    qemu_plugin_register_vcpu_tb_trans_cb(id, tb_trans);
    qemu_plugin_register_atexit_cb(id, report, NULL);
    return 0;
}
