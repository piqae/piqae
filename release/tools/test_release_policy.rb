# frozen_string_literal: true

require "json"
require "minitest/autorun"
require "tmpdir"
require_relative "check_release_policy"

class ReleasePolicyTest < Minitest::Test
  def test_repository_policy_is_truthful
    root = File.expand_path("..", __dir__)
    failures = ReleasePolicy.check_matrix(
      File.join(root, "support-matrix.yaml"),
      File.join(root, "platform-evidence")
    )
    failures.concat(ReleasePolicy.check_gates(File.join(root, "v1-gates.yaml")))
    assert_empty failures
  end

  def test_supported_platform_fails_closed_without_external_evidence
    Dir.mktmpdir do |directory|
      matrix = File.join(directory, "matrix.yaml")
      File.write(
        matrix,
        JSON.generate(
          "version" => 1,
          "platforms" => { "macos" => { "tier" => "supported" } },
          "features" => { "queue" => { "tier" => "preview", "reason" => "soak pending" } }
        )
      )
      failures = ReleasePolicy.check_matrix(matrix, File.join(directory, "evidence"))
      assert failures.any? { |failure| failure.include?("Supported tier requires") }
    end
  end

  def test_expired_or_incomplete_evidence_cannot_enable_support
    Dir.mktmpdir do |directory|
      evidence_directory = File.join(directory, "evidence")
      Dir.mkdir(evidence_directory)
      matrix = File.join(directory, "matrix.yaml")
      File.write(
        matrix,
        JSON.generate(
          "version" => 1,
          "platforms" => { "macos" => { "tier" => "supported" } },
          "features" => { "queue" => { "tier" => "disabled", "reason" => "not built" } }
        )
      )
      File.write(
        File.join(evidence_directory, "macos.json"),
        JSON.generate(
          "schema_version" => 1,
          "platform" => "macos",
          "release" => "v1.0.0",
          "commit" => "1" * 40,
          "valid_until" => "2025-01-01T00:00:00Z",
          "gates" => {}
        )
      )
      failures = ReleasePolicy.check_matrix(
        matrix,
        evidence_directory,
        now: Time.iso8601("2026-01-01T00:00:00Z")
      )
      assert failures.any? { |failure| failure.include?("evidence expired") }
      assert failures.any? { |failure| failure.include?("physical_print") }
    end
  end
end
