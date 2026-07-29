import type { CollectionConfig } from 'payload'
import { canEditContent, canReviewContent, publishedOrAuthenticated } from '../access'
import { triggerMarketingRebuild } from '../hooks/publish'

export const PricingDisplay: CollectionConfig = {
  slug: 'pricing-display',
  admin: { useAsTitle: 'plan', defaultColumns: ['plan', 'headline', '_status', 'updatedAt'] },
  access: { create: canEditContent, delete: canReviewContent, read: publishedOrAuthenticated, update: canEditContent },
  versions: { drafts: { autosave: { interval: 800 } }, maxPerDoc: 100 },
  hooks: { afterChange: [triggerMarketingRebuild] },
  fields: [
    { name: 'plan', type: 'select', required: true, unique: true, options: ['free', 'pro'] },
    { name: 'headline', type: 'textarea', required: true },
  ],
}
