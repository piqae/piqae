#include "piqae_node.h"

int main(void) {
  PiqaeNodeAbiDescriptor descriptor = piqae_node_abi_descriptor();
  PiqaeBuffer invalid = piqae_node_snapshot(0);
  int failed = descriptor.abi_version != 1 || invalid.data == NULL || invalid.length == 0;
  piqae_node_free(invalid);
  return failed;
}
