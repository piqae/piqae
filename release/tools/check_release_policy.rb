#!/usr/bin/env ruby
# frozen_string_literal: true

require "json"
require "set"
require "time"
require "yaml"

module ReleasePolicy
  TIERS = Set.new(%w[disabled preview supported]).freeze
  REQUIRED_EXTERNAL_GATES = Set.new(
    %w[physical_windows code_signing regional_dr external_security_review production_soak]
  ).freeze
  REQUIRED_PLATFORM_EVIDENCE = Set.new(
    %w[package_audit clean_install code_signing physical_print update_rollback]
  ).freeze

  module_function

  def load_yaml(path)
    value = YAML.safe_load(File.read(path, encoding: "UTF-8"), [], [], false)
    raise "#{path}: root must be an object" unless value.is_a?(Hash)

    value
  rescue Psych::Exception => error
    raise "#{path}: invalid YAML: #{error.message}"
  end

  def check_matrix(path, evidence_directory, now: Time.now.utc)
    matrix = load_yaml(path)
    failures = []
    failures << "#{path}: version must be 1" unless matrix["version"] == 1

    %w[platforms features].each do |section|
      entries = matrix[section]
      unless entries.is_a?(Hash) && !entries.empty?
        failures << "#{path}: #{section} must be a non-empty object"
        next
      end

      entries.each do |identifier, entry|
        unless identifier.match?(/\A[a-z0-9_]+\z/) && entry.is_a?(Hash)
          failures << "#{path}: invalid #{section} entry #{identifier.inspect}"
          next
        end
        tier = entry["tier"]
        failures << "#{path}: #{identifier} has invalid tier #{tier.inspect}" unless TIERS.include?(tier)
        reason = entry["reason"]
        if tier != "supported" && (!reason.is_a?(String) || reason.strip.empty?)
          failures << "#{path}: #{identifier} #{tier} tier requires a reason"
        end
        next unless section == "platforms" && tier == "supported"

        failures.concat(check_platform_evidence(identifier, evidence_directory, now: now))
      end
    end
    failures
  end

  def check_platform_evidence(platform, directory, now:)
    path = File.join(directory, "#{platform}.json")
    return ["#{platform}: Supported tier requires #{path}"] unless File.file?(path)

    evidence = JSON.parse(File.read(path, encoding: "UTF-8"))
    failures = []
    failures << "#{path}: schema_version must be 1" unless evidence["schema_version"] == 1
    failures << "#{path}: platform does not match" unless evidence["platform"] == platform
    failures << "#{path}: release must be non-empty" unless non_empty?(evidence["release"])
    failures << "#{path}: commit must be a full lowercase Git SHA" unless evidence["commit"].to_s.match?(/\A[0-9a-f]{40}\z/)

    begin
      valid_until = Time.iso8601(evidence.fetch("valid_until"))
      failures << "#{path}: evidence expired at #{valid_until.utc.iso8601}" unless valid_until > now
    rescue KeyError, ArgumentError
      failures << "#{path}: valid_until must be an RFC 3339 timestamp"
    end

    gates = evidence["gates"]
    unless gates.is_a?(Hash)
      failures << "#{path}: gates must be an object"
      return failures
    end
    REQUIRED_PLATFORM_EVIDENCE.each do |gate|
      record = gates[gate]
      unless record.is_a?(Hash) && record["status"] == "passed" && non_empty?(record["reference"])
        failures << "#{path}: #{gate} requires passed status and an external evidence reference"
      end
    end
    failures
  rescue JSON::ParserError => error
    ["#{path}: invalid JSON: #{error.message}"]
  end

  def check_gates(path)
    gates = load_yaml(path)
    failures = []
    required = gates["required"]
    manual = gates["manual"]
    unless required.is_a?(Array) && manual.is_a?(Array)
      return ["#{path}: required and manual must be arrays"]
    end

    identifiers = []
    required.each do |gate|
      failures << "#{path}: invalid automated gate" unless valid_gate?(gate, "command")
      identifiers << gate["id"] if gate.is_a?(Hash)
      command = gate["command"].to_s
      if command.include?("SPOOL_ALLOW_PHYSICAL_TESTS") || command.match?(/\bprint(?:er)?\s+test\b/i)
        failures << "#{path}: automated gate #{gate["id"]} may invoke physical printing"
      end
    end
    manual.each do |gate|
      failures << "#{path}: invalid manual gate" unless valid_gate?(gate, "evidence")
      identifiers << gate["id"] if gate.is_a?(Hash)
    end
    duplicates = identifiers.group_by(&:itself).select { |_id, values| values.length > 1 }.keys
    failures << "#{path}: duplicate gate IDs: #{duplicates.sort.join(", ")}" unless duplicates.empty?
    manual_identifiers = manual.each_with_object(Set.new) do |gate, result|
      result << gate["id"] if gate.is_a?(Hash)
    end
    missing = REQUIRED_EXTERNAL_GATES - manual_identifiers
    failures << "#{path}: missing external gates: #{missing.to_a.sort.join(", ")}" unless missing.empty?
    failures
  end

  def valid_gate?(gate, field)
    gate.is_a?(Hash) &&
      gate["id"].to_s.match?(/\A[a-z0-9_]+\z/) &&
      non_empty?(gate[field])
  end

  def non_empty?(value)
    value.is_a?(String) && !value.strip.empty?
  end
end

if $PROGRAM_NAME == __FILE__
  root = File.expand_path("..", __dir__)
  failures = ReleasePolicy.check_matrix(
    File.join(root, "support-matrix.yaml"),
    File.join(root, "platform-evidence")
  )
  failures.concat(ReleasePolicy.check_gates(File.join(root, "v1-gates.yaml")))
  abort failures.join("\n") unless failures.empty?

  puts "release support tiers and evidence policy are valid"
end
