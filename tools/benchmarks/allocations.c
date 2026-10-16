/* Linux/glibc-only allocation request counter for LD_PRELOAD profiling.
 * Count the entire process, including setup and I/O. Do not use these runs
 * for timing: atomic counters add overhead to every intercepted call.
 */
#include <stdatomic.h>
#include <stddef.h>
#include <stdio.h>
#include <unistd.h>

extern void *__libc_malloc(size_t);
extern void *__libc_calloc(size_t, size_t);
extern void *__libc_realloc(void *, size_t);
extern void __libc_free(void *);

static _Atomic unsigned long requests, requested_bytes, frees;

void *malloc(size_t size) {
    atomic_fetch_add_explicit(&requests, 1, memory_order_relaxed);
    atomic_fetch_add_explicit(&requested_bytes, size, memory_order_relaxed);
    return __libc_malloc(size);
}

void *calloc(size_t count, size_t size) {
    atomic_fetch_add_explicit(&requests, 1, memory_order_relaxed);
    atomic_fetch_add_explicit(&requested_bytes, count * size, memory_order_relaxed);
    return __libc_calloc(count, size);
}

void *realloc(void *pointer, size_t size) {
    atomic_fetch_add_explicit(&requests, 1, memory_order_relaxed);
    atomic_fetch_add_explicit(&requested_bytes, size, memory_order_relaxed);
    return __libc_realloc(pointer, size);
}

void free(void *pointer) {
    if (pointer)
        atomic_fetch_add_explicit(&frees, 1, memory_order_relaxed);
    __libc_free(pointer);
}

__attribute__((destructor)) static void report(void) {
    unsigned long count = requests, bytes = requested_bytes, released = frees;
    char line[256];
    int length = snprintf(line, sizeof(line), "allocation-count\t%lu\t%lu\t%lu\n",
                          count, bytes, released);
    write(STDERR_FILENO, line, length);
}
