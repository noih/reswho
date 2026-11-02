#include <xpc/xpc.h>
#include <stdatomic.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

struct request {
    atomic_uint refs;
    atomic_int done;
    xpc_connection_t connection;
    void *data;
    size_t size;
    uint64_t received_ns;
};

static void release_request(struct request *r) {
    if (atomic_fetch_sub_explicit(&r->refs, 1, memory_order_acq_rel) == 1) {
        free(r->data);
        free(r);
    }
}

static void receive(struct request *r, xpc_object_t reply) {
    int status = -1;
    struct timespec now;
    if (clock_gettime(CLOCK_MONOTONIC, &now) == 0 &&
        xpc_get_type(reply) == XPC_TYPE_DICTIONARY &&
        !xpc_dictionary_get_value(reply, "com.apple.container.xpc.error")) {
        size_t size = 0;
        const void *data = xpc_dictionary_get_data(reply, "statistics", &size);
        if (data && size > 0 && size <= 65536 && (r->data = malloc(size))) {
            memcpy(r->data, data, size);
            r->size = size;
            r->received_ns = (uint64_t)now.tv_sec * 1000000000ULL + now.tv_nsec;
            status = 1;
        }
    }
    atomic_store_explicit(&r->done, status, memory_order_release);
    release_request(r);
}

void *reswho_apple_stats_start(const char *id) {
    struct request *r = calloc(1, sizeof(*r));
    if (!r) return NULL;
    atomic_init(&r->refs, 2);
    atomic_init(&r->done, 0);
    r->connection = xpc_connection_create_mach_service("com.apple.container.apiserver", NULL, 0);
    xpc_connection_set_event_handler(r->connection, ^(xpc_object_t event) { (void)event; });
    xpc_connection_activate(r->connection);
    xpc_object_t message = xpc_dictionary_create_empty();
    // shortcut: 1.6.1 wire keys, validate new versions before widening the Rust version gate.
    xpc_dictionary_set_string(message, "com.apple.container.xpc.route", "containerStats");
    xpc_dictionary_set_string(message, "id", id);
    xpc_connection_send_message_with_reply(r->connection, message, NULL, ^(xpc_object_t reply) {
        receive(r, reply);
    });
    xpc_release(message);
    return r;
}

int reswho_apple_stats_poll(void *handle, const void **data, size_t *size, uint64_t *received_ns) {
    struct request *r = handle;
    int done = atomic_load_explicit(&r->done, memory_order_acquire);
    if (done == 1) {
        *data = r->data;
        *size = r->size;
        *received_ns = r->received_ns;
    }
    return done;
}

void reswho_apple_stats_drop(void *handle) {
    struct request *r = handle;
    // Cancellation delivers pending replies as errors; their retained owner outlives this caller.
    xpc_connection_cancel(r->connection);
    xpc_release(r->connection);
    release_request(r);
}
