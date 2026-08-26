#include "shim.h"

uint16_t piqae_node_link_anchor(void) {
#if PIQAE_NODE_HAS_NATIVE_ARTIFACT
  return piqae_node_abi_descriptor().abi_version;
#else
  return 0;
#endif
}

PiqaeNodeAbiDescriptor piqae_node_linked_abi_descriptor(void) {
#if PIQAE_NODE_HAS_NATIVE_ARTIFACT
  return piqae_node_abi_descriptor();
#else
  return (PiqaeNodeAbiDescriptor){0, 0, 0};
#endif
}

PiqaeBuffer piqae_node_linked_create(const uint8_t *data, size_t length) {
#if PIQAE_NODE_HAS_NATIVE_ARTIFACT
  return piqae_node_create(data, length);
#else
  (void)data;
  (void)length;
  return (PiqaeBuffer){0, 0};
#endif
}

PiqaeBuffer piqae_node_linked_start(uint64_t handle) {
#if PIQAE_NODE_HAS_NATIVE_ARTIFACT
  return piqae_node_start(handle);
#else
  (void)handle;
  return (PiqaeBuffer){0, 0};
#endif
}

PiqaeBuffer piqae_node_linked_set_host_key_provider(
    uint64_t handle, PiqaeHostKeyProvider provider) {
#if PIQAE_NODE_HAS_NATIVE_ARTIFACT
  return piqae_node_set_host_key_provider(handle, provider);
#else
  (void)handle;
  (void)provider;
  return (PiqaeBuffer){0, 0};
#endif
}

PiqaeBuffer piqae_node_linked_set_connector_key_provider(
    uint64_t handle, PiqaeConnectorKeyProvider provider) {
#if PIQAE_NODE_HAS_NATIVE_ARTIFACT
  return piqae_node_set_connector_key_provider(handle, provider);
#else
  (void)handle;
  (void)provider;
  return (PiqaeBuffer){0, 0};
#endif
}

PiqaeBuffer piqae_node_linked_stop(uint64_t handle) {
#if PIQAE_NODE_HAS_NATIVE_ARTIFACT
  return piqae_node_stop(handle);
#else
  (void)handle;
  return (PiqaeBuffer){0, 0};
#endif
}

PiqaeBuffer piqae_node_linked_command(
    uint64_t handle, const uint8_t *data, size_t length) {
#if PIQAE_NODE_HAS_NATIVE_ARTIFACT
  return piqae_node_command(handle, data, length);
#else
  (void)handle;
  (void)data;
  (void)length;
  return (PiqaeBuffer){0, 0};
#endif
}

PiqaeBuffer piqae_node_linked_destroy(uint64_t handle) {
#if PIQAE_NODE_HAS_NATIVE_ARTIFACT
  return piqae_node_destroy(handle);
#else
  (void)handle;
  return (PiqaeBuffer){0, 0};
#endif
}

PiqaeBuffer piqae_node_linked_broker_request(
    const uint8_t *endpoint,
    size_t endpoint_length,
    const uint8_t *request,
    size_t request_length) {
#if PIQAE_NODE_HAS_NATIVE_ARTIFACT
  return piqae_node_broker_request(
      endpoint, endpoint_length, request, request_length);
#else
  (void)endpoint;
  (void)endpoint_length;
  (void)request;
  (void)request_length;
  return (PiqaeBuffer){0, 0};
#endif
}

PiqaeBuffer piqae_node_linked_broker_execute(
    const uint8_t *endpoint, size_t endpoint_length,
    const uint8_t *credential, size_t credential_length,
    const uint8_t *capability, size_t capability_length,
    const uint8_t *operation, size_t operation_length) {
#if PIQAE_NODE_HAS_NATIVE_ARTIFACT
  return piqae_node_broker_execute(
      endpoint, endpoint_length, credential, credential_length,
      capability, capability_length, operation, operation_length);
#else
  (void)endpoint;
  (void)endpoint_length;
  (void)credential;
  (void)credential_length;
  (void)capability;
  (void)capability_length;
  (void)operation;
  (void)operation_length;
  return (PiqaeBuffer){0, 0};
#endif
}

void piqae_node_linked_free(PiqaeBuffer buffer) {
#if PIQAE_NODE_HAS_NATIVE_ARTIFACT
  piqae_node_free(buffer);
#else
  (void)buffer;
#endif
}
