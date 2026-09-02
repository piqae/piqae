import { describe, expect, it, vi } from "vitest";
import { resolveShopifyTemplateImage } from "../app/core/shopify-template-media.server";
import { convertTemplateImageToJpeg } from "../app/core/template-assets.server";

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
      undefined,
    );
  });

  it("resolves a Shopify SVG GenericFile for print-safe conversion", async () => {
    const digest = "b".repeat(64);
    const admin = {
      graphql: vi.fn(async () =>
        Response.json({
          data: {
            node: {
              id: "gid://shopify/GenericFile/123",
              url: "https://cdn.shopify.com/files/logo.svg",
              mimeType: "image/svg+xml",
            },
          },
        }),
      ),
    };
    const pin = vi.fn(async (sourceUrl: string, id: string) => ({
      id,
      digest,
      mediaType: "image/jpeg" as const,
      bytes: 456,
      sourceUrl,
      sourceMediaType: "image/svg+xml" as const,
      sourceTransform: "piqae-jpeg-v1" as const,
    }));

    await expect(
      resolveShopifyTemplateImage(admin, "gid://shopify/GenericFile/123", pin),
    ).resolves.toMatchObject({
      asset: { mediaType: "image/jpeg", sourceMediaType: "image/svg+xml" },
      resource: { media_type: "image/jpeg", byte_length: 456 },
    });
    expect(pin).toHaveBeenCalledWith(
      "https://cdn.shopify.com/files/logo.svg",
      "gid://shopify/GenericFile/123",
      "image/svg+xml",
    );
  });

  it("rejects non-image Shopify files", async () => {
    const admin = {
      graphql: vi.fn(async () =>
        Response.json({
          data: {
            node: {
              id: "gid://shopify/GenericFile/123",
              url: "https://cdn.shopify.com/files/manual.pdf",
              mimeType: "application/pdf",
            },
          },
        }),
      ),
    };
    await expect(
      resolveShopifyTemplateImage(admin, "gid://shopify/GenericFile/123"),
    ).rejects.toThrow("Choose an image");
  });

  it.each([
    [
      "image/png" as const,
      Buffer.from(
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=",
        "base64",
      ),
    ],
    [
      "image/svg+xml" as const,
      Buffer.from(
        '<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><rect width="40" height="20" fill="#1677ff"/></svg>',
      ),
    ],
  ])("converts %s media to bounded JPEG bytes", async (mediaType, input) => {
    const output = await convertTemplateImageToJpeg(input, mediaType);
    expect(output.byteLength).toBeGreaterThan(100);
    expect([...output.slice(0, 2)]).toEqual([0xff, 0xd8]);
  });

  it("rejects SVGs that reference external content", async () => {
    await expect(
      convertTemplateImageToJpeg(
        Buffer.from(
          '<svg xmlns="http://www.w3.org/2000/svg"><image href="https://example.com/a.png"/></svg>',
        ),
        "image/svg+xml",
      ),
    ).rejects.toThrow("unsupported external content");
  });
});
