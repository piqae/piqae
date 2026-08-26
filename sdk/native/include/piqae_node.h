#ifndef PIQAE_NODE_H
#define PIQAE_NODE_H

#include <stddef.h>
#include <stdint.h>

#if defined(_WIN32) && !defined(PIQAE_NODE_STATIC)
#  if defined(PIQAE_NODE_BUILD)
#    define PIQAE_NODE_API __declspec(dllexport)
#  else
#    define PIQAE_NODE_API __declspec(dllimport)
#  endif
#else
#  define PIQAE_NODE_API
#endif

#ifdef __cplusplus
extern "C" {
#endif

typedef struct PiqaeNodeAbiDescriptor {
  uint16_t abi_version;
  uint16_t contract_min;
  uint16_t contract_max;
} PiqaeNodeAbiDescriptor;

typedef struct PiqaeBuffer {
  uint8_t *data;
  size_t length;
} PiqaeBuffer;

typedef int32_t (*PiqaeHmacSha256Callback)(
    void *context,
    const uint8_t *key_scope,
    size_t key_scope_length,
    const uint8_t *message,
    size_t message_length,
    uint8_t *output,
    size_t output_length);

/*
 * context and callback must remain valid and thread-safe until destroy.
 * Calls are synchronous on the invoking thread. Input/output pointers are
 * borrowed only for the call and must not be retained. The callback writes
 * exactly 32 output bytes and returns 0, or returns non-zero without exposing
 * key material. The host owns secure generation and persistence.
 */
typedef struct PiqaeHostKeyProvider {
  void *context;
  PiqaeHmacSha256Callback hmac_sha256;
} PiqaeHostKeyProvider;
typedef int32_t (*PiqaeGenerateConnectorKeyCallback)(void *, const uint8_t *, size_t, uint8_t *, size_t, size_t *, uint8_t *, size_t);
typedef int32_t (*PiqaeSignConnectorCallback)(void *, const uint8_t *, size_t, const uint8_t *, size_t, uint8_t *, size_t);
typedef int32_t (*PiqaeDeleteConnectorKeyCallback)(void *, const uint8_t *, size_t);
typedef struct PiqaeConnectorKeyProvider { void *context; PiqaeGenerateConnectorKeyCallback generate; PiqaeSignConnectorCallback sign; PiqaeDeleteConnectorKeyCallback delete_key; } PiqaeConnectorKeyProvider;

PIQAE_NODE_API PiqaeNodeAbiDescriptor piqae_node_abi_descriptor(void);
PIQAE_NODE_API PiqaeBuffer piqae_node_create(const uint8_t *data, size_t length);
PIQAE_NODE_API PiqaeBuffer piqae_node_start(uint64_t handle);
PIQAE_NODE_API PiqaeBuffer piqae_node_set_host_key_provider(uint64_t handle, PiqaeHostKeyProvider provider);
PIQAE_NODE_API PiqaeBuffer piqae_node_set_connector_key_provider(uint64_t handle, PiqaeConnectorKeyProvider provider);
PIQAE_NODE_API PiqaeBuffer piqae_node_stop(uint64_t handle);
PIQAE_NODE_API PiqaeBuffer piqae_node_snapshot(uint64_t handle);
PIQAE_NODE_API PiqaeBuffer piqae_node_broker_execute(const uint8_t *endpoint_data, size_t endpoint_length, const uint8_t *credential_json, size_t credential_length, const uint8_t *capability_json, size_t capability_length, const uint8_t *operation_json, size_t operation_length);
PIQAE_NODE_API PiqaeBuffer piqae_node_command(uint64_t handle, const uint8_t *data, size_t length);
PIQAE_NODE_API PiqaeBuffer piqae_node_destroy(uint64_t handle);
PIQAE_NODE_API void piqae_node_free(PiqaeBuffer buffer);

#ifdef __cplusplus
}
#endif

#endif
