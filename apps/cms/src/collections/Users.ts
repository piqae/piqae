import type { CollectionConfig } from 'payload'
import { adminFieldOnly, adminOnly } from '../access'

export const Users: CollectionConfig = {
  slug: 'users',
  auth: { useAPIKey: true },
  admin: {
    useAsTitle: 'email',
    defaultColumns: ['email', 'name', 'role'],
  },
  access: {
    admin: ({ req }) => Boolean(req.user),
    create: async ({ req }) => {
      if ((req.user as { role?: string } | null)?.role === 'admin') return true
      const users = await req.payload.count({ collection: 'users', overrideAccess: true })
      return users.totalDocs === 0
    },
    delete: adminOnly,
    read: ({ req }) =>
      (req.user as { role?: string } | null)?.role === 'admin'
        ? true
        : req.user
          ? { id: { equals: req.user.id } }
          : false,
    update: ({ req }) =>
      (req.user as { role?: string } | null)?.role === 'admin'
        ? true
        : req.user
          ? { id: { equals: req.user.id } }
          : false,
  },
  hooks: {
    beforeChange: [
      async ({ data, operation, req }) => {
        if (operation === 'create') {
          const users = await req.payload.count({ collection: 'users', overrideAccess: true })
          if (users.totalDocs === 0) return { ...data, role: 'admin' }
        }
        return data
      },
    ],
  },
  fields: [
    { name: 'name', type: 'text', required: true },
    {
      name: 'role',
      type: 'select',
      required: true,
      defaultValue: 'editor',
      options: [
        { label: 'Admin', value: 'admin' },
        { label: 'Editor', value: 'editor' },
        { label: 'Reviewer', value: 'reviewer' },
      ],
      access: { create: adminFieldOnly, update: adminFieldOnly },
    },
  ],
}
