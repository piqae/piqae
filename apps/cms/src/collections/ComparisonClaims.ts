import type { CollectionConfig } from 'payload'
import {
  authenticated,
  canEditContent,
  canReviewContent,
  publishedOrAuthenticated,
  reviewerFieldOnly,
} from '../access'
import { triggerMarketingRebuild } from '../hooks/publish'

export const ComparisonClaims: CollectionConfig = {
  slug: 'comparison-claims',
  admin: { useAsTitle: 'claim', defaultColumns: ['competitor', 'status', 'reviewDueAt', 'updatedAt'] },
  access: { create: canEditContent, delete: canReviewContent, read: publishedOrAuthenticated, update: authenticated },
  versions: { drafts: true, maxPerDoc: 50 },
  hooks: {
    beforeChange: [
      ({ data }) => {
        if (data.status === 'verified' && data.reviewDueAt && new Date(data.reviewDueAt) < new Date()) {
          return { ...data, status: 'expired' }
        }
        return data
      },
    ],
    afterChange: [triggerMarketingRebuild],
  },
  fields: [
    { name: 'competitor', type: 'relationship', relationTo: 'competitor-profiles', required: true, index: true },
    { name: 'claim', type: 'textarea', required: true, maxLength: 600 },
    { name: 'sourceUrl', type: 'text', required: true },
    { name: 'sourceSummary', type: 'textarea', required: true, maxLength: 1200 },
    { name: 'observedAt', type: 'date', required: true },
    { name: 'reviewDueAt', type: 'date', required: true, index: true },
    {
      name: 'status',
      type: 'select',
      required: true,
      defaultValue: 'draft',
      options: ['draft', 'verified', 'expired'],
      access: { create: reviewerFieldOnly, update: reviewerFieldOnly },
    },
    {
      name: 'reviewer',
      type: 'relationship',
      relationTo: 'users',
      access: { create: reviewerFieldOnly, update: reviewerFieldOnly },
    },
  ],
}
