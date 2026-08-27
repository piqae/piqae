#!/usr/bin/env ruby
# frozen_string_literal: true

require "json"
require "yaml"

root = File.expand_path("../../..", __dir__)
openapi_path = File.join(root, "contracts/openapi/piqae-v1.yaml")
output_path = File.join(root, "standards/printpacket/schema/printpacket-v1.schema.json")
openapi = YAML.safe_load(File.read(openapi_path), aliases: true)
schemas = openapi.fetch("components").fetch("schemas")
selected = schemas.select { |name, _| name.start_with?("BusinessDocument") }

rewrite = lambda do |value|
  case value
  when Hash
    value.each_with_object({}) do |(key, child), output|
      output[key] = if key == "$ref"
                      child.sub("#/components/schemas/", "#/$defs/")
                    else
                      rewrite.call(child)
                    end
    end
  when Array
    value.map { |child| rewrite.call(child) }
  else
    value
  end
end

definitions = rewrite.call(selected)
definitions.fetch("BusinessDocumentV1").fetch("properties")["format"] = {
  "enum" => ["printpacket/v1", "piqae.business-document/v1"]
}
region = definitions.fetch("BusinessDocumentRegion")
definitions["PrintPacketHeaderRegion"] = region.merge(
  "properties" => region.fetch("properties").slice("first", "default")
)
definitions["PrintPacketFooterRegion"] = region.merge(
  "properties" => region.fetch("properties").slice("default", "last")
)
document_properties = definitions.fetch("BusinessDocumentV1").fetch("properties")
document_properties["header"] = { "$ref" => "#/$defs/PrintPacketHeaderRegion" }
document_properties["footer"] = { "$ref" => "#/$defs/PrintPacketFooterRegion" }

schema = {
  "$schema" => "https://json-schema.org/draft/2020-12/schema",
  "$id" => "urn:printpacket:schema:v1",
  "title" => "PrintPacket v1",
  "$ref" => "#/$defs/BusinessDocumentV1",
  "$defs" => definitions
}

File.write(output_path, JSON.pretty_generate(schema) + "\n")
