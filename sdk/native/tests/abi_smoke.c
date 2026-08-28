#include "piqae_node.h"
#include <stdlib.h>
#include <string.h>

static char *copy_buffer(PiqaeBuffer buffer) {
  if (buffer.data == NULL || buffer.length == 0 || buffer.length == SIZE_MAX) {
    piqae_node_free(buffer);
    return NULL;
  }
  char *value = (char *)malloc(buffer.length + 1);
  if (value == NULL) {
    piqae_node_free(buffer);
    return NULL;
  }
  memcpy(value, buffer.data, buffer.length);
  value[buffer.length] = '\0';
  piqae_node_free(buffer);
  return value;
}

static uint64_t created_handle(const char *json) {
  const char *field = strstr(json, "\"handle\":");
  return field == NULL ? 0 : (uint64_t)strtoull(field + 9, NULL, 10);
}

int main(void) {
  PiqaeNodeAbiDescriptor descriptor = piqae_node_abi_descriptor();
  const char configuration[] =
      "{\"contract\":2,\"host_mode\":\"embedded_application\","
      "\"availability\":\"foreground_only\",\"local_only\":true,"
      "\"application_id\":\"com.piqae.native-release-smoke\","
      "\"data_directory\":\"runtime\"}";
  char *created = copy_buffer(piqae_node_create(
      (const uint8_t *)configuration, sizeof(configuration) - 1));
  uint64_t handle = created == NULL ? 0 : created_handle(created);
  free(created);

  char *started = handle == 0 ? NULL : copy_buffer(piqae_node_start(handle));
  const char command[] = "{\"type\":\"print_packet_capabilities\"}";
  char *capabilities = started == NULL ? NULL : copy_buffer(piqae_node_command(
      handle, (const uint8_t *)command, sizeof(command) - 1));
  int failed = descriptor.abi_version != PIQAE_NODE_ABI_VERSION ||
      descriptor.contract_min != PIQAE_NODE_CONTRACT_VERSION ||
      descriptor.contract_max != PIQAE_NODE_CONTRACT_VERSION || started == NULL ||
      strstr(started, "\"started\":true") == NULL || capabilities == NULL ||
      strstr(capabilities, "\"contract\":\"printpacket/v1\"") == NULL ||
      strstr(capabilities, "\"direct_offline_rendering\":true") == NULL;
  free(started);
  free(capabilities);
  if (handle != 0) {
    piqae_node_free(piqae_node_destroy(handle));
  }
  return failed;
}
