/* univsr must not drop a variable initial displacement of a pointer IV.
 *
 * SQLite 3.53.4 pragmaVtabConnect:
 *
 *   for(i=0, j=pPragma->iPragCName; i<pPragma->nPragCName; i++, j++){
 *     sqlite3_str_appendf(&acc, "%c\"%s\"", cSep, pragCName[j]);
 *   }
 *
 * IVSR turns the pragCName[j] walk into a moving pointer whose init is
 * GEP(pragCName, j0*8) with j0 loaded at runtime. univsr (un-IVSR) then
 * reverted it to indexed form, but extract_base_from_init treated the
 * VARIABLE init offset as 0 and peeled the base to pragCName — every
 * pragma_* eponymous virtual table was declared with foreign_key_list's
 * column names (speedtest1 --testset app: "no such column: name").
 *
 * The fixed pass uses the init pointer value itself as the SIB base.
 */
#include <stdio.h>

typedef unsigned char u8;
typedef unsigned long long u64;

typedef struct PragmaName {
    const char *const zName;
    u8 ePragTyp;
    u8 mPragFlg;
    u8 iPragCName;
    u8 nPragCName;
    u64 iArg;
} PragmaName;

static const char *const pragCName[] = {
    "id",     "seq",   "table", "from",       "to", "on_update",
    "on_delete", "match", "cid",   "name",       "type", "notnull",
    "dflt_value", "pk",   "schema", "tname",     "ttype", "ncol",
    "wr",     "strict",
};

static const PragmaName aPragmaName[] = {
    { "foreign_key_list", 1, 0x01, 0, 8, 0 },
    { "table_info", 2, 0x02, 8, 6, 1 },
    { "table_list", 3, 0x03, 14, 6, 2 },
};

static int connect_vtab(void *pAux, char *out, int cap) {
    const PragmaName *pPragma = (const PragmaName *)pAux;
    int i, j, n = 0;
    char cSep = '(';
    for (i = 0, j = pPragma->iPragCName; i < pPragma->nPragCName; i++, j++) {
        n += snprintf(out + n, cap - n, "%c\"%s\"", cSep, pragCName[j]);
        cSep = ',';
    }
    if (i == 0)
        n += snprintf(out + n, cap - n, "(\"%s\"", pPragma->zName);
    n += snprintf(out + n, cap - n, ")");
    return n;
}

int main(void) {
    char buf[256];
    int k;
    for (k = 0; k < 3; k++) {
        connect_vtab((void *)&aPragmaName[k], buf, sizeof buf);
        printf("%s => %s\n", aPragmaName[k].zName, buf);
    }
    return 0;
}
