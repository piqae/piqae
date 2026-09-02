import type { AdminGraphql } from "./orders.server";
import { pinShopifyTemplateJpeg } from "./template-assets.server";
import type { ExternalAsset, PrintPacket } from "./template-model";

const MEDIA_IMAGE_QUERY = `#graphql
  query PiqaeTemplateMediaImage($id: ID!) {
    node(id: $id) {
      ... on MediaImage {
        id
        image { url(transform: { preferredContentType: JPG }) }
      }
    }
  }
`;

export type ShopifyTemplateImage = {
  asset: ExternalAsset;
  resourceKey: string;
  resource: NonNullable<PrintPacket["resources"]>[string];
};

export async function resolveShopifyTemplateImage(
  admin: AdminGraphql,
  id: string,
  pin: typeof pinShopifyTemplateJpeg = pinShopifyTemplateJpeg,
): Promise<ShopifyTemplateImage> {
  if (!/^gid:\/\/shopify\/MediaImage\/\d{1,30}$/.test(id))
    throw new Error("Choose an image from Shopify files");
  const response = await admin.graphql(MEDIA_IMAGE_QUERY, {
    variables: { id },
  });
  if (!response.ok) throw new Error("Shopify could not load that image");
  const payload = (await response.json()) as {
    data?: { node?: { id?: string; image?: { url?: string } } };
    errors?: unknown[];
  };
  const sourceUrl = payload.data?.node?.image?.url;
  if (payload.errors?.length || typeof sourceUrl !== "string")
    throw new Error("The selected Shopify file is not a ready image");
  const asset = await pin(sourceUrl, id);
  const resourceKey = `shopify_image_${asset.digest.slice(0, 16)}`;
  return {
    asset,
    resourceKey,
    resource: {
      type: "image",
      digest: `sha256:${asset.digest}`,
      media_type: "image/jpeg",
      byte_length: asset.bytes,
    },
  };
}
