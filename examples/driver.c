#include <stdint.h>
#include <stdio.h>
extern int64_t answer(void);
int main(void) {
    int64_t result = answer();
    printf("Cranelift returned %lld\n", (long long)result);
    return result == 42 ? 0 : 1;
}
