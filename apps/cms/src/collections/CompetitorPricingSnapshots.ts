import type { CollectionConfig } from 'payload'
import { canEditContent, canReviewContent, publishedOrAuthenticated } from '../access'
import { triggerMarketingRebuild } from '../hooks/publish'

export const CompetitorPricingSnapshots: CollectionConfig = {
  slug: 'competitor-pricing-snapshots',
  admin: { useAsTitle: 'label' },
  access: { create: canEditContent, delete: canReviewContent, read: publishedOrAuthenticated, update: canEditContent },
  versions: { drafts: true, maxPerDoc: 50 },
  hooks: { afterChange: [triggerMarketingRebuild] },
  fields: [
    { name: 'label', type: 'text', required: true },
    { name: 'competitor', type: 'relationship', relationTo: 'competitor-profiles', required: true, index: true },
    { name: 'currency', type: 'text', required: true, defaultValue: 'USD' },
    { name: 'sourceUrl', type: 'text', required: true },
    { name: 'observedAt', type: 'date', required: true },
    { name: 'reviewDueAt', type: 'date', required: true, index: true },
    {
      name: 'tiers',
      type: 'array',
      required: true,
      fields: [
        { name: 'name', type: 'text', required: true },
        { name: 'monthlyCents', type: 'number', required: true, min: 0 },
        { name: 'annualCents', type: 'number', min: 0 },
        { name: 'includedJobs', type: 'number', required: true, min: 0 },
        { name: 'annualIncludedJobs', type: 'number', min: 0 },
        { name: 'includedComputers', type: 'number', min: 0 },
        { name: 'includedSubaccounts', type: 'number', min: 0 },
        { name: 'extraJobUnit', type: 'number', min: 0 },
        { name: 'extraJobUnitCents', type: 'number', min: 0 },
        { name: 'extraSubaccountCents', type: 'number', min: 0 },
        { name: 'notes', type: 'textarea' },
      ],
    },
  ],
}
