#ifndef MCRX_CORE_FFI_H
#define MCRX_CORE_FFI_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define MCRX_STATUS_OK 0
#define MCRX_STATUS_INVALID_ARGUMENT 1
#define MCRX_STATUS_ERROR 2
#define MCRX_STATUS_ALREADY_RUNNING 3
#define MCRX_STATUS_PANIC 4

typedef struct McrxContext McrxContext;

typedef struct McrxPacketView {
    const uint8_t *payload;
    size_t payload_len;

    uint64_t subscription_id;
    const char *source_ip;
    uint16_t source_port;
    const char *group_ip;
    uint16_t dst_port;

    const char *socket_local_ip;
    uint16_t socket_local_port;
    const char *configured_interface_ip;
    uint32_t configured_interface_index;
    uint8_t has_configured_interface_index;
    const char *destination_local_ip;
    uint32_t ingress_interface_index;
    uint8_t has_ingress_interface_index;
} McrxPacketView;

typedef void (*McrxPacketCallback)(const McrxPacketView *packet, void *user_data);

const char *mcrx_ffi_version(void);
const char *mcrx_last_error(void);

McrxContext *mcrx_context_new(void);
void mcrx_context_free(McrxContext *context);
const char *mcrx_context_last_error(const McrxContext *context);
size_t mcrx_context_subscription_count(const McrxContext *context);

int mcrx_context_add_subscription(
    McrxContext *context,
    const char *group,
    uint16_t dst_port,
    const char *source,
    const char *interface,
    uint64_t *subscription_id_out
);

int mcrx_context_join_subscription(McrxContext *context, uint64_t subscription_id);
int mcrx_context_leave_subscription(McrxContext *context, uint64_t subscription_id);
int mcrx_context_remove_subscription(McrxContext *context, uint64_t subscription_id);

int mcrx_context_poll(
    McrxContext *context,
    size_t max_packets,
    McrxPacketCallback callback,
    void *user_data,
    size_t *received_out
);

int mcrx_context_start(
    McrxContext *context,
    McrxPacketCallback callback,
    void *user_data,
    uint32_t idle_sleep_ms
);

int mcrx_context_stop(McrxContext *context);

#ifdef __cplusplus
}
#endif

#endif
