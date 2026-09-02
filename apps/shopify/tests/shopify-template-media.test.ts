import { describe, expect, it, vi } from "vitest";
import { resolveShopifyTemplateImage } from "../app/core/shopify-template-media.server";

describe("Shopify template media", () => {
  it("resolves a native MediaImage into a pinned PrintPacket resource", async () => {
    const digest = "a".repeat(64);
    const admin = {
      graphql: vi.fn(async () =>
        Response.json({
          data: {
            node: {
              id: "gid://shopify/MediaImage/123",
              image: { url: "https://cdn.shopify.com/example.jpg" },
            },
          },
        }),
      ),
    };
    const pin = vi.fn(async (sourceUrl: string, id: string) => ({
      id,
      digest,
      mediaType: "image/jpeg" as const,
      bytes: 321,
      sourceUrl,
    }));

    await expect(
      resolveShopifyTemplateImage(admin, "gid://shopify/MediaImage/123", pin),
    ).resolves.toEqual({
      asset: {
        id: "gid://shopify/MediaImage/123",
        digest,
        mediaType: "image/jpeg",
        bytes: 321,
        sourceUrl: "https://cdn.shopify.com/example.jpg",
      },
      resourceKey: "shopify_image_aaaaaaaaaaaaaaaa",
      resource: {
        type: "image",
        digest: `sha256:${digest}`,
        media_type: "image/jpeg",
        byte_length: 321,
      },
    });
    expect(pin).toHaveBeenCalledWith(
      "https://cdn.shopify.com/example.jpg",
      "gid://shopify/MediaImage/123",
    );
  });

  it("rejects non-image Shopify file identifiers before querying", async () => {
    const admin = { graphql: vi.fn() };
    await expect(
      resolveShopifyTemplateImage(admin, "gid://shopify/GenericFile/123"),
    ).rejects.toThrow("Choose an image");
    expect(admin.graphql).not.toHaveBeenCalled();
  });
});
