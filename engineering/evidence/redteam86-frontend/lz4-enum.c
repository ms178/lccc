#define LZ4F_ENABLE_OBSOLETE_ENUMS
#include <stdio.h>
#include <lz4.h>
#include <lz4frame.h>
int main(void) { printf("%d %d %d %d %d %d %zu\n",LZ4_VERSION_NUMBER,LZ4F_default,LZ4F_max64KB,LZ4F_max4MB,LZ4F_blockLinked,blockLinked,sizeof(LZ4F_preferences_t)); }
