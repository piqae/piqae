#!/usr/bin/env ruby
# frozen_string_literal: true

require "json"
require "optparse"
require "set"
require "yaml"

module ProductRelease
  VERSION_KINDS = Set.new(%w[cargo_workspace dotnet npm]).freeze
  DEPLOYMENT_ORDER = %w[migrations control_plane workers web shopify node_canary].freeze

  module_function

  def load_yaml(path)
    value = YAML.safe_load(
      File.read(path, encoding: "UTF-8"),
      permitted_classes: [],
      permitted_symbols: [],
      aliases: false,
      filename: path
    )
    raise "#{path}: root must be an object" unless value.is_a?(Hash)

    value
  rescue Psych::Exception => error
    raise "#{path}: invalid YAML: #{error.message}"
  end

  def version(path, kind)
    case kind
    when "npm"
      JSON.parse(File.read(path, encoding: "UTF-8")).fetch("version")
    when "cargo_workspace"
      text = File.read(path, encoding: "UTF-8")
      section = text[/\[workspace\.package\](.*?)(?=\n\[|\z)/m, 1]
      section&.match(/^version\s*=\s*"([^"]+)"/)&.[](1)
    when "dotnet"
      File.read(path, encoding: "UTF-8")[/<Version>\s*([^<\s]+)\s*<\/Version>/, 1]
    end
  rescue JSON::ParserError, KeyError
    nil
  end

  def check(path, root:, product: false, release_version: nil)
    document = load_yaml(path)
    failures = []
    failures << "#{path}: schema_version must be 1" unless document["schema_version"] == 1

    contracts = document["contracts"]
    unless contracts.is_a?(Hash) && !contracts.empty?
      return failures << "#{path}: contracts must be a non-empty object"
    end
    contracts.each do |name, contract|
      unless contract.is_a?(Hash) && contract["current"].is_a?(Integer) &&
             contract["supported"].is_a?(Array) && contract["supported"].include?(contract["current"])
        failures << "#{path}: contract #{name} must include its integer current version in supported"
      end
    end

    components = document["components"]
    unless components.is_a?(Hash) && !components.empty?
      return failures << "#{path}: components must be a non-empty object"
    end
    components.each do |name, component|
      unless component.is_a?(Hash)
        failures << "#{path}: component #{name} must be an object"
        next
      end
      file = component["version_file"].to_s
      kind = component["version_kind"].to_s
      failures << "#{path}: component #{name} has invalid version_kind" unless VERSION_KINDS.include?(kind)
      if file.empty? || file.start_with?("/") || file.split("/").include?("..")
        failures << "#{path}: component #{name} has unsafe version_file"
        next
      end
      absolute = File.join(root, file)
      if !File.file?(absolute)
        if product
          failures << "#{path}: product release requires #{file} for component #{name}"
        end
      elsif version(absolute, kind).to_s.empty?
        failures << "#{path}: cannot read component #{name} version from #{file}"
      end

      Array(component["depends_on"]).each do |dependency|
        failures << "#{path}: component #{name} depends on unknown component #{dependency}" unless components.key?(dependency)
      end
      (component["consumes"] || {}).each do |contract, required|
        supported = contracts.dig(contract, "supported")
        unless supported.is_a?(Array) && supported.include?(required)
          failures << "#{path}: component #{name} requires unsupported #{contract} contract #{required}"
        end
      end
    end

    order = document.dig("deployment", "order")
    failures << "#{path}: deployment order must be #{DEPLOYMENT_ORDER.join(' -> ')}" unless order == DEPLOYMENT_ORDER
    failures << "#{path}: database rollback must be forward_only" unless document.dig("deployment", "rollback", "database") == "forward_only"

    if release_version
      workspace_version = version(File.join(root, "Cargo.toml"), "cargo_workspace")
      failures << "#{path}: release #{release_version} must match Cargo workspace version #{workspace_version}" unless workspace_version == release_version
      components.each do |name, component|
        next unless component["release_version_locked"]

        component_version = version(File.join(root, component.fetch("version_file")), component.fetch("version_kind"))
        unless component_version == release_version
          failures << "#{path}: release #{release_version} must match #{name} version #{component_version}"
        end
      end
    end
    failures
  rescue Errno::ENOENT => error
    ["#{path}: #{error.message}"]
  end
end

if $PROGRAM_NAME == __FILE__
  options = { product: false }
  OptionParser.new do |parser|
    parser.on("--product", "Require every product component") { options[:product] = true }
    parser.on("--release-version VERSION", "Require the product version") { |value| options[:release_version] = value }
  end.parse!
  root = File.expand_path("../..", __dir__)
  path = File.join(root, "release", "product-release.yaml")
  failures = ProductRelease.check(path, root: root, **options)
  abort failures.join("\n") unless failures.empty?
  puts "product release components and compatibility are valid"
end
