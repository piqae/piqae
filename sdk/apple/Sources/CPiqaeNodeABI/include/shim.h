#include "../../../../native/include/piqae_node.h"

/* Forces the local binary target into the final image when it is present. */
uint16_t piqae_node_link_anchor(void);
PiqaeNodeAbiDescriptor piqae_node_linked_abi_descriptor(void);
PiqaeBuffer piqae_node_linked_create(const uint8_t *data, size_t length);
PiqaeBuffer piqae_node_linked_start(uint64_t handle);
PiqaeBuffer piqae_node_linked_set_host_key_provider(
    uint64_t handle, PiqaeHostKeyProvider provider);
PiqaeBuffer piqae_node_linked_stop(uint64_t handle);
PiqaeBuffer piqae_node_linked_command(
    uint64_t handle, const uint8_t *data, size_t length);
PiqaeBuffer piqae_node_linked_destroy(uint64_t handle);
void piqae_node_linked_free(PiqaeBuffer buffer);
