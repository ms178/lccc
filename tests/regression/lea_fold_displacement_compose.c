/* LCCC-SQLITE-COLNAMES-O1: the peephole LEA->memory fold must SUM
 * displacements, never glue their text.
 *
 * Reduced from SQLite 3.53.4 sqlite3GenerateColumnNames (select.c) built at
 * -O1: the AS-clause branch loads pEList->a[i].zEName.  Pre-peephole asm
 *     leaq 8(%r13), %r8; leaq (%r10,%r10,2), %r10; shlq $3, %r10
 *     leaq (%r8,%r10), %r9; movq 8(%r9), %r10 ...
 * was composed to `leaq 8(%r13, %r10), %r9; movq 8(%r9), %rcx`, and the
 * final fold spliced the consumer's `8` in front of the producer's `8(`:
 *     movq 88(%r13,%r10), %rcx        (correct: 16(%r13,%r10))
 * so every aliased column got the name of a neighbouring item's field. */
#include <stdio.h>
#include <string.h>

typedef void (*destructor)(void *);
typedef struct Column { char *zCnName; long pad; } Column;
typedef struct Table {
  char *zName; Column *aCol; char pad[36]; short iPKey;
} Table;
typedef struct Expr {
  unsigned char op; char pad0[47]; short iColumn; char pad1[22];
  union { Table *pTab; } y;
} Expr;
struct ExprList_item {
  Expr *pExpr;
  char *zEName;
  struct { unsigned char sortFlags; unsigned eEName : 2; unsigned done : 1; } fg;
  union { struct { unsigned short iOrderByCol, iAlias; } x; int iConstExprReg; } u;
};
typedef struct ExprList { int nExpr; int nAlloc; struct ExprList_item a[]; } ExprList;
typedef struct SrcList SrcList;
typedef struct Select {
  char pad0[24]; ExprList *pEList; SrcList *pSrc; char pad1[32]; struct Select *pPrior;
} Select;
typedef struct Vdbe { int nCol; const char *azName[8]; } Vdbe;
typedef struct sqlite3 { char pad[48]; unsigned long long flags; } sqlite3;
typedef struct Parse {
  sqlite3 *db; char *zErrMsg; Vdbe *pVdbe; int rc; short nQueryLoop;
  unsigned char b[9];
  unsigned disableTriggers : 1, mayAbort : 1, hasCompound : 1, bReturning : 1,
      bHasExists : 1, colNamesSet : 1;
} Parse;

static char out[8][32];
__attribute__((noinline)) void sqlite3VdbeSetNumCols(Vdbe *v, int n) { v->nCol = n; }
__attribute__((noinline)) int sqlite3VdbeSetColName(Vdbe *v, int i, int k, const char *z,
                                                    destructor d) {
  (void)k; (void)d;
  snprintf(out[i], sizeof out[i], "%s", z ? z : "(null)");
  v->azName[i] = out[i];
  return 0;
}
__attribute__((noinline)) char *sqlite3MPrintf(sqlite3 *db, const char *f, ...) {
  (void)db; return (char *)f;
}
__attribute__((noinline)) char *sqlite3DbStrDup(sqlite3 *db, const char *z) {
  (void)db; return (char *)z;
}
__attribute__((noinline)) void sqlite3RowSetClear(void *p) { (void)p; }
__attribute__((noinline)) void generateColumnTypes(Parse *p, SrcList *s, ExprList *e) {
  (void)p; (void)s; (void)e;
}

__attribute__((noinline)) void sqlite3GenerateColumnNames(Parse *pParse, Select *pSelect) {
  Vdbe *v = pParse->pVdbe;
  int i;
  Table *pTab;
  SrcList *pTabList;
  ExprList *pEList;
  sqlite3 *db = pParse->db;
  int fullName;
  int srcName;
  if (pParse->colNamesSet) return;
  while (pSelect->pPrior) pSelect = pSelect->pPrior;
  pTabList = pSelect->pSrc;
  pEList = pSelect->pEList;
  pParse->colNamesSet = 1;
  fullName = (db->flags & 0x00000004) != 0;
  srcName = (db->flags & 0x00000040) != 0 || fullName;
  sqlite3VdbeSetNumCols(v, pEList->nExpr);
  for (i = 0; i < pEList->nExpr; i++) {
    Expr *p = pEList->a[i].pExpr;
    if (pEList->a[i].zEName && pEList->a[i].fg.eEName == 0) {
      char *zName = pEList->a[i].zEName;
      sqlite3VdbeSetColName(v, i, 0, zName, ((destructor)-1));
    } else if (srcName && p->op == 168) {
      char *zCol;
      int iCol = p->iColumn;
      pTab = p->y.pTab;
      if (iCol < 0) iCol = pTab->iPKey;
      if (iCol < 0) {
        zCol = "rowid";
      } else {
        zCol = pTab->aCol[iCol].zCnName;
      }
      if (fullName) {
        char *zName = 0;
        zName = sqlite3MPrintf(db, "%s.%s", pTab->zName, zCol);
        sqlite3VdbeSetColName(v, i, 0, zName, ((destructor)sqlite3RowSetClear));
      } else {
        sqlite3VdbeSetColName(v, i, 0, zCol, ((destructor)-1));
      }
    } else {
      const char *z = pEList->a[i].zEName;
      z = z == 0 ? sqlite3MPrintf(db, "column%d", i + 1) : sqlite3DbStrDup(db, z);
      sqlite3VdbeSetColName(v, i, 0, z, ((destructor)sqlite3RowSetClear));
    }
  }
  generateColumnTypes(pParse, pTabList, pEList);
}

static Column cols[2] = {{"id", 0}, {"val", 0}};
static Table tab = {"t1", cols, {0}, -1};
static union { ExprList l; char raw[sizeof(ExprList) + 4 * sizeof(struct ExprList_item)]; } el;

int main(void) {
  Expr e0, e1;
  memset(&e0, 0, sizeof e0); memset(&e1, 0, sizeof e1);
  e0.op = 168; e0.iColumn = 1; e0.y.pTab = &tab;
  e1.op = 1;
  el.l.nExpr = 4;
  el.l.a[0].pExpr = &e1; el.l.a[0].zEName = "alias_a";
  el.l.a[1].pExpr = &e0; el.l.a[1].zEName = 0;
  el.l.a[2].pExpr = &e1; el.l.a[2].zEName = "span_c"; el.l.a[2].fg.eEName = 1;
  el.l.a[3].pExpr = &e1; el.l.a[3].zEName = "alias_d";
  Select s; memset(&s, 0, sizeof s); s.pEList = &el.l;
  sqlite3 db; memset(&db, 0, sizeof db); db.flags = 0x40;
  Vdbe v; memset(&v, 0, sizeof v);
  Parse p; memset(&p, 0, sizeof p); p.db = &db; p.pVdbe = &v;
  sqlite3GenerateColumnNames(&p, &s);
  for (int i = 0; i < v.nCol; i++) printf("%d %s\n", i, v.azName[i]);
  return 0;
}
