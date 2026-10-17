typedef ELEM T __attribute__((ext_vector_type(LANES)));
T sinkv __asm__("sink");
const unsigned sink_size = sizeof(T), lane_size = LSIZE;
void put(T v) { sinkv = v; }
T get(void) { return sinkv; }
