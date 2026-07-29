import type { CollectionConfig } from 'payload'
import { canEditContent, publishedOrAuthenticated } from '../access'

function isPublishing(data: unknown): boolean {
  return Boolean(data && typeof data === 'object' && '_status' in data && data._status === 'published')
}

export const Media: CollectionConfig = {
  slug: 'media',
  versions: { drafts: true, maxPerDoc: 30 },
  access: {
    create: canEditContent,
    delete: canEditContent,
    read: publishedOrAuthenticated,
    update: canEditContent,
  },
  upload: {
    focalPoint: true,
    mimeTypes: ['image/*'],
    imageSizes: [
      { name: 'card', width: 900, height: 600, position: 'centre' },
      { name: 'hero', width: 1800, height: 1100, position: 'centre' },
    ],
  },
  fields: [
    { name: 'alt', type: 'text', required: true, maxLength: 220 },
    { name: 'rightsOwner', type: 'text', required: true, maxLength: 160 },
    {
      name: 'rightsConfirmed',
      type: 'checkbox',
      required: true,
      defaultValue: false,
      validate: (value, { data }) =>
        !isPublishing(data) || value === true
          ? true
          : 'Rights must be confirmed before media can be published.',
    },
    {
      name: 'rightsExpiresAt',
      type: 'date',
      validate: (value, { data }) =>
        !isPublishing(data) || !value || new Date(value).getTime() > Date.now()
          ? true
          : 'Media rights have expired and must be renewed before publication.',
    },
    {
      name: 'privacyReviewed',
      type: 'checkbox',
      required: true,
      defaultValue: false,
      admin: { description: 'Confirms labels and images contain no addresses, order IDs, or customer data.' },
      validate: (value, { data }) =>
        !isPublishing(data) || value === true
          ? true
          : 'Privacy review must pass before media can be published.',
    },
  ],
}
