# frozen_string_literal: true

require "minitest/autorun"
require_relative "check_product_release"

class ProductReleaseTest < Minitest::Test
  def test_repository_manifest_is_structurally_valid
    root = File.expand_path("../..", __dir__)
    failures = ProductRelease.check(File.join(root, "release/product-release.yaml"), root: root)
    assert_empty failures
  end

  def test_product_release_fails_closed_without_shopify_component
    root = File.expand_path("../..", __dir__)
    failures = ProductRelease.check(
      File.join(root, "release/product-release.yaml"), root: root, product: true
    )
    unless File.file?(File.join(root, "apps/shopify/package.json"))
      assert failures.any? { |failure| failure.include?("product release requires apps/shopify/package.json") }
    end
  end

  def test_release_version_must_match_workspace
    root = File.expand_path("../..", __dir__)
    failures = ProductRelease.check(
      File.join(root, "release/product-release.yaml"), root: root, release_version: "99.0.0"
    )
    assert failures.any? { |failure| failure.include?("must match Cargo workspace version") }
  end

  def test_dotnet_version_is_read_from_project
    root = File.expand_path("../..", __dir__)
    project = File.join(root, "sdk/dotnet/src/Piqae.Node/Piqae.Node.csproj")
    workspace_version = ProductRelease.version(File.join(root, "Cargo.toml"), "cargo_workspace")
    assert_equal workspace_version, ProductRelease.version(project, "dotnet")
  end

  def test_embedded_sdks_require_only_native_contract_two
    root = File.expand_path("../..", __dir__)
    document = ProductRelease.load_yaml(File.join(root, "release/product-release.yaml"))
    assert_equal({"current" => 2, "supported" => [2]}, document.dig("contracts", "native_sdk"))
    assert_equal 2, document.dig("components", "apple_node_sdk", "consumes", "native_sdk")
    assert_equal 2, document.dig("components", "dotnet_node_sdk", "consumes", "native_sdk")
  end

  def test_release_locked_sdk_version_must_match_product
    root = File.expand_path("../..", __dir__)
    failures = ProductRelease.check(
      File.join(root, "release/product-release.yaml"), root: root, release_version: "99.0.0"
    )
    assert failures.any? { |failure| failure.include?("dotnet_node_sdk version") }
  end
end
