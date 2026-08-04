import unittest

import check_competitor_mentions as policy


class CompetitorMentionPolicyTests(unittest.TestCase):
    def test_reviewed_marketing_paths_are_allowed(self) -> None:
        competitor = "print" + "node"
        self.assertTrue(policy.is_marketing_path(f"apps/web/src/routes/compare/{competitor}/+page.svelte"))
        self.assertTrue(policy.is_marketing_path("apps/web/src/lib/marketing/calculator.ts"))

    def test_product_and_contract_paths_are_not_allowed(self) -> None:
        self.assertFalse(policy.is_marketing_path("README.md"))
        self.assertFalse(policy.is_marketing_path("contracts/openapi/piqae-v1.yaml"))


if __name__ == "__main__":
    unittest.main()
