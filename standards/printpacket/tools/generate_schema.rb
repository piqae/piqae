#!/usr/bin/env ruby
# frozen_string_literal: true

require "json"
require "yaml"

root = File.expand_path("../../..", __dir__)
openapi_path = File.join(root, "contracts/openapi/piqae-v1.yaml")
output_path = File.join(root, "standards/printpacket/schema/printpacket-v1.schema.json")
openapi = YAML.safe_load(File.read(openapi_path), aliases: true)
schemas = openapi.fetch("components").fetch("schemas")
selected = schemas.select { |name, _| name.start_with?("PrintPacket") }

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
definitions.fetch("PrintPacketV1").fetch("properties")["format"] = {
  "const" => "printpacket/v1"
}

nodes = definitions.fetch("PrintPacketNode").fetch("oneOf")
flow = nodes.find { |node| node.dig("properties", "type", "enum") == ["section", "stack", "row"] }
raise "missing section/stack/row node schema" unless flow
flow.fetch("properties").fetch("type")["enum"] = ["section", "stack"]
flow.fetch("properties").fetch("gap_mm")["maximum"] = 2000
row = Marshal.load(Marshal.dump(flow))
row.fetch("properties")["type"] = { "const" => "row" }
row.fetch("properties").fetch("children")["maxItems"] = 32
nodes.insert(nodes.index(flow) + 1, row)

node_for = lambda do |type|
  nodes.find do |node|
    node.dig("properties", "type", "const") == type ||
      Array(node.dig("properties", "type", "enum")).include?(type)
  end || raise("missing #{type} node schema")
end

%w[grid repeat].each do |type|
  node_for.call(type).fetch("properties").fetch("gap_mm")["maximum"] = 2000
end
node_for.call("grid").fetch("properties").fetch("children")["maxItems"] = 32
qr_size = node_for.call("qr").fetch("properties").fetch("size_mm")
qr_size["minimum"] = 8
qr_size["maximum"] = 2000
divider_width = node_for.call("divider").fetch("properties").fetch("width_pt")
divider_width.delete("exclusiveMinimum")
divider_width["minimum"] = 0.1

region = definitions.fetch("PrintPacketRegion")
definitions["PrintPacketHeaderRegion"] = region.merge(
  "properties" => region.fetch("properties").slice("first", "default")
)
definitions["PrintPacketFooterRegion"] = region.merge(
  "properties" => region.fetch("properties").slice("default", "last")
)
document_properties = definitions.fetch("PrintPacketV1").fetch("properties")
document_properties["header"] = { "$ref" => "#/$defs/PrintPacketHeaderRegion" }
document_properties["footer"] = { "$ref" => "#/$defs/PrintPacketFooterRegion" }

schema = {
  "$schema" => "https://json-schema.org/draft/2020-12/schema",
  "$id" => "urn:printpacket:schema:v1",
  "title" => "PrintPacket v1",
  "$ref" => "#/$defs/PrintPacketV1",
  "$defs" => definitions
}

File.write(output_path, JSON.pretty_generate(schema) + "\n")
