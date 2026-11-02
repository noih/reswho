#include "../src/containers/apple/xpc.c"
#include <assert.h>
#include <pthread.h>

static struct request *pending(void) {
    struct request *r = calloc(1, sizeof(*r));
    assert(r);
    atomic_init(&r->refs, 2);
    atomic_init(&r->done, 0);
    return r;
}

static void check_reply(xpc_object_t reply, int expected) {
    struct request *r = pending();
    const void *data = NULL;
    size_t size = 0;
    uint64_t ns = 0;
    assert(reswho_apple_stats_poll(r, &data, &size, &ns) == 0);
    receive(r, reply);
    assert(reswho_apple_stats_poll(r, &data, &size, &ns) == expected);
    if (expected == 1) {
        assert(size == 2 && memcmp(data, "{}", 2) == 0 && ns > 0);
    }
    release_request(r);
}

static void *late_reply(void *r) {
    receive(r, (xpc_object_t)XPC_ERROR_CONNECTION_INVALID);
    return NULL;
}

int main(void) {
    xpc_object_t reply = xpc_dictionary_create_empty();
    check_reply(reply, -1);
    xpc_dictionary_set_string(reply, "statistics", "not data");
    check_reply(reply, -1);
    xpc_dictionary_set_data(reply, "statistics", "{}", 2);
    check_reply(reply, 1);
    xpc_dictionary_set_string(reply, "com.apple.container.xpc.error", "withheld");
    check_reply(reply, -1);
    xpc_dictionary_set_value(reply, "com.apple.container.xpc.error", NULL);
    char oversized[65537] = {0};
    xpc_dictionary_set_data(reply, "statistics", oversized, sizeof(oversized));
    check_reply(reply, -1);
    check_reply((xpc_object_t)XPC_ERROR_CONNECTION_INVALID, -1);
    xpc_release(reply);
    for (int i = 0; i < 100; i++) {
        struct request *r = pending();
        pthread_t thread;
        assert(pthread_create(&thread, NULL, late_reply, r) == 0);
        release_request(r);
        assert(pthread_join(thread, NULL) == 0);
    }
    return 0;
}
