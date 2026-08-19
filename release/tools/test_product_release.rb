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
end
