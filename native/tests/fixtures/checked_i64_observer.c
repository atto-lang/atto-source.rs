#include <stdint.h>
#include <inttypes.h>
#include <stdio.h>
extern int64_t atto_xo_guest_main(void);
int main(void) { printf("%" PRId64 "\n", atto_xo_guest_main()); return 0; }
