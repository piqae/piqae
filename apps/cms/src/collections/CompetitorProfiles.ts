import type { CollectionConfig } from 'payload'
import { canEditContent, publishedOrAuthenticated } from '../access'
import { triggerMarketingRebuild } from '../hooks/publish'

export const CompetitorProfiles: CollectionConfig = {
  slug: 'competitor-profiles',
  admin: { useAsTitle: 'name' },
  access: { create: canEditContent, delete: canEditContent, read: publishedOrAuthenticated, update: canEditContent },
  versions: { drafts: true, maxPerDoc: 30 },
  hooks: { afterChange: [triggerMarketingRebuild] },
  fields: [
    { name: 'name', type: 'text', required: true, unique: true },
    { name: 'slug', type: 'text', required: true, unique: true, index: true },
    { name: 'officialUrl', type: 'text', required: true },
    { name: 'pricingUrl', type: 'text' },
    { name: 'docsUrl', type: 'text' },
    { name: 'summary', type: 'textarea', required: true },
  ],
}

