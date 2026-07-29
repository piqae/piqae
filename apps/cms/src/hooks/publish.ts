import type { CollectionAfterChangeHook, GlobalAfterChangeHook } from 'payload'

async function postQuietly(url: string | undefined, body: Record<string, unknown>, label: string) {
  if (!url) return
  try {
    const response = await fetch(url, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(body),
      signal: AbortSignal.timeout(8_000),
    })
    if (!response.ok) console.error(`${label} returned ${response.status}`)
  } catch {
    console.error(`${label} failed`)
  }
}

export const triggerMarketingRebuild: CollectionAfterChangeHook = async ({
  collection,
  doc,
  operation,
}) => {
  if (doc?._status !== 'published') return doc
  await postQuietly(
    process.env.MARKETING_DEPLOY_HOOK_URL,
    { collection: collection.slug, id: doc.id, operation },
    'Marketing deploy hook',
  )
  return doc
}

export const triggerMarketingRebuildAlways: CollectionAfterChangeHook = async ({
  collection,
  doc,
  operation,
}) => {
  await postQuietly(
    process.env.MARKETING_DEPLOY_HOOK_URL,
    { collection: collection.slug, id: doc.id, operation },
    'Marketing deploy hook',
  )
  return doc
}

export const triggerGlobalMarketingRebuild: GlobalAfterChangeHook = async ({ global, doc }) => {
  await postQuietly(
    process.env.MARKETING_DEPLOY_HOOK_URL,
    { global: global.slug, operation: 'update' },
    'Marketing deploy hook',
  )
  return doc
}

export async function runPricingDriftCheck(source = 'payload') {
  const checkUrl = process.env.PRICING_DRIFT_CHECK_URL
  if (!checkUrl) return { configured: false, drift: false }
  try {
    const response = await fetch(checkUrl, {
      headers: {
        accept: 'application/json',
        ...(process.env.PRICING_DRIFT_SHARED_SECRET
          ? { authorization: `Bearer ${process.env.PRICING_DRIFT_SHARED_SECRET}` }
          : {}),
      },
      signal: AbortSignal.timeout(8_000),
    })
    if (!response.ok && response.status !== 409) {
      throw new Error(`price check returned ${response.status}`)
    }
    const result = (await response.json()) as { drift?: boolean; details?: unknown }
    if (result.drift) {
      await postQuietly(
        process.env.PRICING_DRIFT_WEBHOOK_URL,
        { source, detectedAt: new Date().toISOString(), details: result.details ?? null },
        'Pricing drift webhook',
      )
    }
    return { configured: true, drift: result.drift === true }
  } catch {
    await postQuietly(
      process.env.PRICING_DRIFT_WEBHOOK_URL,
      { source, detectedAt: new Date().toISOString(), error: 'pricing_check_failed' },
      'Pricing drift webhook',
    )
    console.error('Pricing drift check failed')
    return { configured: true, drift: true }
  }
}

export const checkPricingAfterPublish: CollectionAfterChangeHook = async ({ doc }) => {
  if (doc?._status === 'published') await runPricingDriftCheck('payload_publish')
  return doc
}
