import { createHmac } from 'node:crypto'

type PreviewCollection = 'comparison-pages' | 'pages'

export function marketingPreviewUrl(
  collection: PreviewCollection,
  slug: unknown,
): string | null {
  const origin = process.env.MARKETING_PREVIEW_URL?.replace(/\/$/, '')
  const secret = process.env.CMS_PREVIEW_SECRET
  if (!origin || !secret || typeof slug !== 'string' || !slug) return null
  const payload = Buffer.from(
    JSON.stringify({
      collection,
      slug,
      exp: Math.floor(Date.now() / 1000) + 10 * 60,
    }),
  ).toString('base64url')
  const signature = createHmac('sha256', secret).update(payload).digest('base64url')
  return `${origin}/preview/cms?token=${payload}.${signature}`
}
