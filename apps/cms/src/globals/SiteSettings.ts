import type { GlobalConfig } from 'payload'
import { adminOnly } from '../access'
import { triggerGlobalMarketingRebuild } from '../hooks/publish'

export const SiteSettings: GlobalConfig = {
  slug: 'site-settings',
  access: { read: () => true, update: adminOnly },
  hooks: { afterChange: [triggerGlobalMarketingRebuild] },
  fields: [
    { name: 'announcement', type: 'text' },
    {
      name: 'navigation',
      type: 'array',
      maxRows: 10,
      fields: [
        { name: 'label', type: 'text', required: true },
        { name: 'href', type: 'text', required: true },
      ],
    },
    { name: 'salesEmail', type: 'email' },
    { name: 'securityEmail', type: 'email' },
    { name: 'githubUrl', type: 'text' },
    { name: 'workingNameNotice', type: 'textarea' },
  ],
}
