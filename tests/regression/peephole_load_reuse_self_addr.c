/* Peephole `load_reuse`: a load whose destination is ONE OF ITS OWN ADDRESS
 * REGISTERS must never be used as a reuse source.
 *
 * `dead_writes::reuse_redundant_loads` replaces a later load with a copy from
 * an earlier load's destination register when the two have the same opcode and
 * the same textual memory operand. It validated every line in between -- but
 * not the FIRST LOAD ITSELF. For
 *
 *     leaq 8(%r11), %r15     # &b->chain
 *     movq (%r15), %r15      # chain      <- dst == address register
 *     movq (%r15), %r12      # chain[0]
 *
 * the second load reads a DIFFERENT address than the first (the register was
 * overwritten by the first load's result), yet it was rewritten to
 * `movq %r15, %r12` and then coalesced away: the hash lookup returned the
 * `char **` instead of the string it points at.
 *
 * Found by scripts/gen_gep_chain_stress.py seed 74 (-O2); this case crashes
 * or prints garbage on a compiler without the fix.
 */

#include <stdio.h>
#include <string.h>

typedef struct Bucket { unsigned count; char **chain; } Bucket;
typedef struct Hash   { unsigned htsize; unsigned count; char *first;
                        Bucket *ht; } Hash;

static unsigned long long g_sink;   /* consumes every computed value */
typedef struct Schema { char pad0[8]; const char *zName; char pad1[24]; Hash idxHash; } Schema;
typedef struct Db     { char dpad[24]; Schema *pSchema; } Db;
typedef struct DbList { char lpad[32]; Db *aDb; int nDb; long x[5]; } DbList;

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
    while( z[0] ){
        unsigned c = 0xdfu & (unsigned char)*z;
        h += c;
        h *= 2654435761u;
        acc2 = acc2 * 33u + c;
        n += 1u;
        z++;
    }
    h ^= acc2 + n;
    if( pH->ht ){
        Bucket *b = &pH->ht[h % pH->htsize];
        count = b->count;
        elem = count ? b->chain[0] : (char*)0;
        if( count ) g_sink += (unsigned long long)(pH->count);
    }else{
        elem = pH->first;
        count = pH->count;
        if( count==0 ) elem = (char*)0;
    }
    return elem;
}


static int dbIsNamed(DbList *db, int j, const char *zDb){
    return strcmp(db->aDb[j].pSchema->zName, zDb)==0;
}

/* sqlite3FindIndex(): the Schema pointer is the ROOT of the GEP chain and is
 * live across the inlined findElem() body. */
char *findIndex(DbList *db, const char *zName, const char *zDb){
    char *p = (char*)0;
    int i;
    long v0 = db->x[0];
    long v1 = db->x[1];
    long v2 = db->x[2];
    long v3 = db->x[3];
    long v4 = db->x[4];

    (void)dbIsNamed;
    for(i=0; i<db->nDb; i++){
        int j = (i<2) ? i^1 : i;                 /* search TEMP before MAIN */
        Schema *pSchema = db->aDb[j].pSchema;
        p = findElem(&pSchema->idxHash, zName);
        if( p ) break;
    }
    g_sink += (unsigned long long)(0+v0+v1+v2+v3+v4);
    return p;
}

static unsigned strHash(const char *z){
    unsigned h = 0;
    while( z[0] ){ h += 0xdfu & (unsigned char)*(z++); h *= 2654435761u; }
    return h;
}

int main(void){
    static char *val[3];
    static Schema s[3];
    static Db dbs[3];
    static Bucket tab0[16], tab1[16], tab2[16];
    static DbList dbl;
    char *r;
    int i;

    val[0] = "ZERO"; val[1] = "ONE"; val[2] = "TWO";
    for(i=0;i<16;i++){
        tab0[i].count = 0; tab0[i].chain = (char**)0;
        tab1[i].count = 0; tab1[i].chain = (char**)0;
        tab2[i].count = 0; tab2[i].chain = (char**)0;
    }
    s[0].zName = "temp";
    s[0].idxHash.htsize = 16;
    s[0].idxHash.count = 1;
    s[0].idxHash.first = val[0];
    s[0].idxHash.ht = tab0;
    s[1].zName = "main";
    s[1].idxHash.htsize = 16;
    s[1].idxHash.count = 1;
    s[1].idxHash.first = val[1];
    s[1].idxHash.ht = tab1;
    s[2].zName = "aux";
    s[2].idxHash.htsize = 16;
    s[2].idxHash.count = 1;
    s[2].idxHash.first = val[2];
    s[2].idxHash.ht = tab2;
    dbs[0].pSchema = &s[0]; dbs[1].pSchema = &s[1]; dbs[2].pSchema = &s[2];
    {
        unsigned hh = strHash("alpha") % 16;
        tab0[hh].count = 1;
        tab0[hh].chain = &val[0];
    }
    {
        unsigned hh = strHash("beta") % 16;
        tab1[hh].count = 1;
        tab1[hh].chain = &val[1];
    }
    {
        unsigned hh = strHash("gamma") % 16;
        tab2[hh].count = 1;
        tab2[hh].chain = &val[2];
    }
    dbl.aDb = dbs;
    dbl.nDb = 3;
    for(i=0;i<5;i++) dbl.x[i] = (long)i + 1;

    r = findIndex(&dbl, "alpha", 0);
    printf("%s|%s=%s\n", "alpha", 0, r ? r : "-");
    r = findIndex(&dbl, "alpha", "main");
    printf("%s|%s=%s\n", "alpha", "main", r ? r : "-");
    r = findIndex(&dbl, "alpha", "temp");
    printf("%s|%s=%s\n", "alpha", "temp", r ? r : "-");
    r = findIndex(&dbl, "beta", 0);
    printf("%s|%s=%s\n", "beta", 0, r ? r : "-");
    r = findIndex(&dbl, "beta", "main");
    printf("%s|%s=%s\n", "beta", "main", r ? r : "-");
    r = findIndex(&dbl, "beta", "temp");
    printf("%s|%s=%s\n", "beta", "temp", r ? r : "-");
    r = findIndex(&dbl, "gamma", 0);
    printf("%s|%s=%s\n", "gamma", 0, r ? r : "-");
    r = findIndex(&dbl, "gamma", "main");
    printf("%s|%s=%s\n", "gamma", "main", r ? r : "-");
    r = findIndex(&dbl, "gamma", "temp");
    printf("%s|%s=%s\n", "gamma", "temp", r ? r : "-");
    r = findIndex(&dbl, "delta", 0);
    printf("%s|%s=%s\n", "delta", 0, r ? r : "-");
    r = findIndex(&dbl, "delta", "main");
    printf("%s|%s=%s\n", "delta", "main", r ? r : "-");
    r = findIndex(&dbl, "delta", "temp");
    printf("%s|%s=%s\n", "delta", "temp", r ? r : "-");
    r = findIndex(&dbl, "alpha", "aux");
    printf("aux0=%s\n", r ? r : "-");
    r = findIndex(&dbl, "beta", "aux");
    printf("aux1=%s\n", r ? r : "-");
    r = findIndex(&dbl, "gamma", "aux");
    printf("aux2=%s\n", r ? r : "-");
    printf("sink=%llu\n", g_sink);
    return 0;
}
