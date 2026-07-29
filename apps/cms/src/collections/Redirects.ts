import type { CollectionConfig } from 'payload'
import { adminOnly, authenticated } from '../access'
import { triggerMarketingRebuildAlways } from '../hooks/publish'

export const Redirects: CollectionConfig = {
  slug: 'redirects',
  admin: { useAsTitle: 'from' },
  access: { create: authenticated, delete: adminOnly, read: () => true, update: authenticated },
  hooks: { afterChange: [triggerMarketingRebuildAlways] },
  fields: [
    { name: 'from', type: 'text', required: true, unique: true, index: true },
    { name: 'to', type: 'text', required: true },
    { name: 'permanent', type: 'checkbox', required: true, defaultValue: true },
  ],
}
