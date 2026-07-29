import type { Block } from 'payload'

export const HeroBlock: Block = {
  slug: 'hero',
  fields: [
    { name: 'eyebrow', type: 'text' },
    { name: 'heading', type: 'textarea', required: true },
    { name: 'lede', type: 'textarea', required: true },
    { name: 'primaryLabel', type: 'text' },
    { name: 'primaryHref', type: 'text' },
    { name: 'secondaryLabel', type: 'text' },
    { name: 'secondaryHref', type: 'text' },
    { name: 'media', type: 'upload', relationTo: 'media' },
  ],
}

export const RichTextBlock: Block = {
  slug: 'richText',
  fields: [{ name: 'content', type: 'richText', required: true }],
}

export const FeatureGridBlock: Block = {
  slug: 'featureGrid',
  fields: [
    { name: 'eyebrow', type: 'text' },
    { name: 'heading', type: 'text', required: true },
    {
      name: 'items',
      type: 'array',
      minRows: 1,
      maxRows: 12,
      fields: [
        { name: 'title', type: 'text', required: true },
        { name: 'body', type: 'textarea', required: true },
      ],
    },
  ],
}

export const CtaBlock: Block = {
  slug: 'cta',
  fields: [
    { name: 'heading', type: 'text', required: true },
    { name: 'body', type: 'textarea' },
    { name: 'label', type: 'text', required: true },
    { name: 'href', type: 'text', required: true },
  ],
}

