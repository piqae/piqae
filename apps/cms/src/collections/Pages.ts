import type { CollectionConfig } from 'payload'
import { canEditContent, publishedOrAuthenticated } from '../access'
import { CtaBlock, FeatureGridBlock, HeroBlock, RichTextBlock } from '../blocks'
import { triggerMarketingRebuild } from '../hooks/publish'
import { marketingPreviewUrl } from '../preview'

export const Pages: CollectionConfig = {
  slug: 'pages',
  admin: {
    useAsTitle: 'title',
    defaultColumns: ['title', 'slug', '_status', 'updatedAt'],
    preview: (doc) => marketingPreviewUrl('pages', doc.slug),
  },
  access: {
    create: canEditContent,
    delete: canEditContent,
    read: publishedOrAuthenticated,
    update: canEditContent,
  },
  versions: { drafts: { autosave: { interval: 800 } }, maxPerDoc: 50 },
  hooks: { afterChange: [triggerMarketingRebuild] },
  fields: [
    { name: 'title', type: 'text', required: true },
    { name: 'slug', type: 'text', required: true, unique: true, index: true },
    {
      name: 'seo',
      type: 'group',
      fields: [
        { name: 'title', type: 'text', required: true, maxLength: 70 },
        { name: 'description', type: 'textarea', required: true, maxLength: 180 },
        { name: 'image', type: 'upload', relationTo: 'media' },
        { name: 'noindex', type: 'checkbox', defaultValue: false },
      ],
    },
    { name: 'layout', type: 'blocks', required: true, blocks: [HeroBlock, RichTextBlock, FeatureGridBlock, CtaBlock] },
  ],
}
