#!/usr/bin/env python3
"""Generate deterministic C programs that stress FOLDED-ADDRESS LIVENESS.

Why this exists
---------------
The backend composes a chain of constant-offset GEPs into one addressing
mode: `GEP(GEP(p,+a),+b)` is emitted as `a+b(%p)` (``compose_const_gep_folds``
in ``src/backend/generation.rs``).  The register that the Load/Store actually
reads is therefore the ROOT of the chain, not the intermediate GEP dest --
which is never materialised and whose "last use" in the IR is the GEP that
produced it.

Liveness must extend the root (and every intermediate link, and the variable
index) to every folded access, or the register allocator is free to recycle
the root's register in between -- exactly where an inlined callee body is
hungry for registers.

Before the session-30 fix, ``extend_gep_base_liveness`` stopped at the
immediate base and dropped any GEP dest that fed another GEP from the fold
set.  sqlite3.50 exploited both: ``sqlite3FindIndex`` inlines
``sqlite3HashFind`` -> ``findElementWithHash`` -> ``strHash``, the ``strHash``
loop took the register still holding ``pSchema``, and
``pSchema->idxHash.ht`` then dereferenced the *key pointer* (SIGSEGV at -O1).

The shapes that matter, all varied by seed
------------------------------------------
  * GEP chain depth (2 or 3 links) and the byte offsets involved;
  * an inlined callee whose body burns registers in a loop BEFORE it reads
    the folded fields (string hash / array scan / pointer chase);
  * reads of the folded fields before AND after that loop;
  * a caller-side branch between the definition of the root pointer and the
    inlined call (this is what puts a HOLE in the root's live range: the
    "continue" blocks are laid out after the inlined body);
  * 0..8 values live across the inlined call, so the allocator cannot simply
    keep everything in registers (too much pressure spills the root instead,
    which hides the bug -- the sweep covers the whole range);
  * a caller loop, so the whole thing sits inside a cyclic live range.

Every generated program is strictly well-defined C (unsigned arithmetic only,
no division by zero, no out-of-bounds access, no uninitialised reads): any
divergence against GCC is a real miscompile, never a UB artefact.

Usage:
    gen_gep_chain_stress.py SEED > case.c
"""

import random
import sys

PRELUDE = r"""
#include <stdio.h>
#include <string.h>

typedef struct Bucket { unsigned count; char **chain; } Bucket;
typedef struct Hash   { unsigned htsize; unsigned count; char *first;
                        Bucket *ht; } Hash;

static unsigned long long g_sink;   /* consumes every computed value */
"""

# Hash-loop shapes. Each is a snippet computing `h` from `z` while burning a
# register on the walk; `acc2`/`n` widen the loop so more registers are live.
LOOP_SHAPES = [
    # 0: plain multiplicative string hash (sqlite strHash)
    """    while( z[0] ){
        h += 0xdfu & (unsigned char)*z;
        h *= 2654435761u;
        z++;
    }
""",
    # 1: hash + length accumulator
    """    while( z[0] ){
        h += 0xdfu & (unsigned char)*z;
        h *= 2654435761u;
        n += 1u;
        z++;
    }
    h += n;
""",
    # 2: hash + second rotating accumulator
    """    while( z[0] ){
        unsigned c = 0xdfu & (unsigned char)*z;
        h += c;
        h *= 2654435761u;
        acc2 = acc2 * 33u + c;
        n += 1u;
        z++;
    }
    h ^= acc2 + n;
""",
    # 3: array scan (indexed walk)
    """    {
        unsigned i = 0;
        while( z[i] ){
            h += 0xdfu & (unsigned char)z[i];
            h *= 2654435761u;
            i += 1u;
        }
        n = i;
    }
""",
    # 4: pointer chase with two walkers
    """    {
        const char *q = z;
        while( z[0] ){
            h += 0xdfu & (unsigned char)*z;
            h *= 2654435761u;
            acc2 += (unsigned char)*q;
            q++;
            z++;
        }
        h ^= acc2;
    }
""",
]

# What the callee reads through the folded address, and when.
#   pre  : reads before the register-hungry loop
#   post : reads after it
BODY_TAIL = r"""    if( pH->ht ){
        Bucket *b = &pH->ht[h % pH->htsize];
        count = b->count;
        elem = count ? b->chain[0] : (char*)0;
@POST@    }else{
        elem = pH->first;
        count = pH->count;
        if( count==0 ) elem = (char*)0;
    }
    return elem;
}
"""

POST_VARIANTS = [
    "",
    "        if( count ) g_sink += (unsigned long long)(pH->count);\n",
    "        g_sink += (unsigned long long)pH->htsize;\n"
    "        if( count ) g_sink += (unsigned long long)(pH->count);\n",
    "        if( count && elem ) g_sink += (unsigned long long)pH->htsize"
    " + (unsigned long long)(pH->count);\n",
]


def struct_block(inner, pad_schema, pad_hash, ndb):
    """Emit the type declarations with the layout chosen for this seed."""
    out = []
    if inner:
        out.append(
            "typedef struct Inner  { char ipad[%d]; Hash idxHash; } Inner;\n"
            % pad_hash
        )
        out.append(
            "typedef struct Schema { char pad0[8]; const char *zName;"
            " char pad1[%d]; Inner in; } Schema;\n" % pad_schema
        )
    else:
        out.append(
            "typedef struct Schema { char pad0[8]; const char *zName;"
            " char pad1[%d]; Hash idxHash; } Schema;\n" % pad_schema
        )
    out.append("typedef struct Db     { char dpad[24]; Schema *pSchema; } Db;\n")
    out.append(
        "typedef struct DbList { char lpad[32]; Db *aDb; int nDb;"
        " long x[%d]; } DbList;\n" % ndb
    )
    return "".join(out)


def access_expr(inner, field):
    """`&s->...idxHash.field` spelled through the chosen number of GEP links."""
    if inner:
        return "pSchema->in.idxHash." + field
    return "pSchema->idxHash." + field


def gen(seed):
    rnd = random.Random(seed)
    inner = rnd.choice([False, True])
    pad_schema = rnd.choice([8, 16, 24, 32])
    pad_hash = rnd.choice([0, 8, 16, 32])
    shape = rnd.randrange(len(LOOP_SHAPES))
    post = rnd.choice(POST_VARIANTS)
    branch = rnd.choice([False, True])          # zDb `continue` arm
    pressure = rnd.choice([0, 1, 2, 3, 4, 5, 6, 8])
    two_sided = rnd.choice([False, True])       # read pH BEFORE the loop too
    ndb = rnd.choice([2, 3])
    nkeys = rnd.choice([3, 4])
    ntab = rnd.choice([8, 16])

    s = [PRELUDE]
    s.append(struct_block(inner, pad_schema, pad_hash, max(pressure, 2)))

    # ---- the inlined callee ------------------------------------------------
    s.append(
        """
/* Inlined into findIndex: burns a register walking the key, THEN reads the
 * folded fields of *pH.  `pH` is a chain of constant-offset GEPs from the
 * caller's Schema pointer, so every one of these reads must keep that root
 * register alive. */
static char *findElem(const Hash *pH, const char *pKey){
    char *elem = (char*)0;
    unsigned count = 0;
    unsigned h = 0;
    unsigned acc2 = 7u;
    unsigned n = 0;
    const char *z = pKey;
%(pre)s%(loop)s"""
        % {
            "pre": ("    if( pH->htsize==0 ) return (char*)0;\n" if two_sided else ""),
            "loop": LOOP_SHAPES[shape],
        }
    )
    s.append(BODY_TAIL.replace("@POST@", post))
    s.append("\n")

    # ---- the caller --------------------------------------------------------
    keep = "".join(
        "    long v%d = db->x[%d];\n" % (i, i % max(pressure, 1))
        for i in range(pressure)
    )
    sink = "".join("+v%d" % i for i in range(pressure))
    named = (
        """
    if( zDb && strcmp(db->aDb[j].pSchema->zName, zDb)!=0 ) continue;
"""
        if branch
        else ""
    )
    s.append(
        """
static int dbIsNamed(DbList *db, int j, const char *zDb){
    return strcmp(db->aDb[j].pSchema->zName, zDb)==0;
}

/* sqlite3FindIndex(): the Schema pointer is the ROOT of the GEP chain and is
 * live across the inlined findElem() body. */
char *findIndex(DbList *db, const char *zName, const char *zDb){
    char *p = (char*)0;
    int i;
%(keep)s
    (void)dbIsNamed;
    for(i=0; i<db->nDb; i++){
        int j = (i<2) ? i^1 : i;                 /* search TEMP before MAIN */
        Schema *pSchema = db->aDb[j].pSchema;
%(named)s        p = findElem(&%(acc)s, zName);
        if( p ) break;
    }
    g_sink += (unsigned long long)(0%(sink)s);
    return p;
}
"""
        % {
            "keep": keep,
            "sink": sink,
            "named": named
            if not branch
            else "\n        if( zDb && dbIsNamed(db, j, zDb)==0 ) continue;",
            "acc": access_expr(inner, "idxHash") if False else (
                "pSchema->in.idxHash" if inner else "pSchema->idxHash"
            ),
        }
    )

    # ---- driver ------------------------------------------------------------
    keys = ["alpha", "beta", "gamma", "delta", "epsilon"][:nkeys]
    setup = []
    for k in range(ndb):
        setup.append(
            """    s[%(k)d].zName = "%(name)s";
    s[%(k)d].%(mid)sidxHash.htsize = %(ntab)d;
    s[%(k)d].%(mid)sidxHash.count = 1;
    s[%(k)d].%(mid)sidxHash.first = val[%(k)d];
    s[%(k)d].%(mid)sidxHash.ht = tab%(k)d;
"""
            % {
                "k": k,
                "name": ("temp", "main", "aux")[k % 3],
                "mid": "in." if inner else "",
                "ntab": ntab,
            }
        )
    fill = []
    for k in range(ndb):
        fill.append(
            """    {
        unsigned hh = strHash("%(key)s") %% %(ntab)d;
        tab%(k)d[hh].count = 1;
        tab%(k)d[hh].chain = &val[%(k)d];
    }
"""
            % {"k": k, "key": keys[k % len(keys)], "ntab": ntab}
        )

    probes = []
    for key in keys:
        for zdb in ("0", '"main"', '"temp"'):
            probes.append(
                '    r = findIndex(&dbl, "%s", %s);\n'
                '    printf("%%s|%%s=%%s\\n", "%s", %s, r ? r : "-");\n'
                % (key, zdb, key, zdb)
            )
    for i in range(ndb):
        probes.append(
            '    r = findIndex(&dbl, "%s", "aux");\n'
            '    printf("aux%d=%%s\\n", r ? r : "-");\n' % (keys[i % len(keys)], i)
        )

    s.append(
        """
static unsigned strHash(const char *z){
    unsigned h = 0;
    while( z[0] ){ h += 0xdfu & (unsigned char)*(z++); h *= 2654435761u; }
    return h;
}

int main(void){
    static char *val[3];
    static Schema s[3];
    static Db dbs[3];
    static Bucket tab0[%(ntab)d], tab1[%(ntab)d], tab2[%(ntab)d];
    static DbList dbl;
    char *r;
    int i;

    val[0] = "ZERO"; val[1] = "ONE"; val[2] = "TWO";
    for(i=0;i<%(ntab)d;i++){
        tab0[i].count = 0; tab0[i].chain = (char**)0;
        tab1[i].count = 0; tab1[i].chain = (char**)0;
        tab2[i].count = 0; tab2[i].chain = (char**)0;
    }
%(setup)s    dbs[0].pSchema = &s[0]; dbs[1].pSchema = &s[1]; dbs[2].pSchema = &s[2];
%(fill)s    dbl.aDb = dbs;
    dbl.nDb = %(ndb)d;
    for(i=0;i<%(nx)d;i++) dbl.x[i] = (long)i + 1;

%(probes)s    printf("sink=%%llu\\n", g_sink);
    return 0;
}
"""
        % {
            "ntab": ntab,
            "setup": "".join(setup),
            "fill": "".join(fill),
            "ndb": ndb,
            "nx": max(pressure, 2),
            "probes": "".join(probes),
        }
    )
    return "".join(s)


def main():
    seed = int(sys.argv[1]) if len(sys.argv) > 1 else 1
    sys.stdout.write(gen(seed))


if __name__ == "__main__":
    main()
