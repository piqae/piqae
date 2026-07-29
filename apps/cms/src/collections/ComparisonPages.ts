import type { CollectionConfig } from 'payload'
import { canEditContent, publishedOrAuthenticated } from '../access'
import { CtaBlock, RichTextBlock } from '../blocks'
import { triggerMarketingRebuild } from '../hooks/publish'
import { marketingPreviewUrl } from '../preview'

export const ComparisonPages: CollectionConfig = {
  slug: 'comparison-pages',
  admin: {
    useAsTitle: 'title',
    defaultColumns: ['title', 'intent', '_status', 'updatedAt'],
    preview: (doc) => marketingPreviewUrl('comparison-pages', doc.slug),
  },
  access: { create: canEditContent, delete: canEditContent, read: publishedOrAuthenticated, update: canEditContent },
  versions: { drafts: { autosave: { interval: 800 } }, maxPerDoc: 50 },
  hooks: { afterChange: [triggerMarketingRebuild] },
  fields: [
    { name: 'title', type: 'text', required: true },
    { name: 'slug', type: 'text', required: true, unique: true, index: true },
    { name: 'competitor', type: 'relationship', relationTo: 'competitor-profiles', required: true },
    {
      name: 'intent',
      type: 'select',
      required: true,
      options: ['comparison', 'alternative', 'migration', 'calculator-support'],
    },
    { name: 'summary', type: 'textarea', required: true },
    { name: 'claims', type: 'relationship', relationTo: 'comparison-claims', hasMany: true },
    { name: 'layout', type: 'blocks', blocks: [RichTextBlock, CtaBlock] },
    { name: 'lastVerifiedAt', type: 'date', required: true },
    { name: 'noindex', type: 'checkbox', defaultValue: true },
  ],
}
