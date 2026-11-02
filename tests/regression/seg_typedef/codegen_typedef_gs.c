/* Typedef'd and typeof'd __seg_gs objects keep their named address space.
 * Every access to them must use the %gs: segment prefix. Before the H2 fix,
 * the typedef dropped the space and these loads and stores were emitted as
 * plain generic memory (silent miscompile). */
typedef int __seg_gs sint;
typedef sint sarr_t[4];

sint gobj;
sarr_t garr;
typeof(gobj) tobj;

int rd_scalar(void) { return gobj; }
void wr_scalar(int v) { gobj = v; }
int rd_array(int i) { return garr[i]; }
int rd_typeof(void) { return tobj; }
