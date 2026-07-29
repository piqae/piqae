import { postgresAdapter } from '@payloadcms/db-postgres'
import { lexicalEditor } from '@payloadcms/richtext-lexical'
import { s3Storage } from '@payloadcms/storage-s3'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { buildConfig } from 'payload'
import sharp from 'sharp'
import { ComparisonClaims } from './collections/ComparisonClaims'
import { ComparisonPages } from './collections/ComparisonPages'
import { CompetitorPricingSnapshots } from './collections/CompetitorPricingSnapshots'
import { CompetitorProfiles } from './collections/CompetitorProfiles'
import { Media } from './collections/Media'
import { Pages } from './collections/Pages'
import { PricingDisplay } from './collections/PricingDisplay'
import { Redirects } from './collections/Redirects'
import { Users } from './collections/Users'
import { SiteSettings } from './globals/SiteSettings'

const filename = fileURLToPath(import.meta.url)
const dirname = path.dirname(filename)
const r2Configured = [
  process.env.R2_BUCKET,
  process.env.R2_ENDPOINT,
  process.env.R2_ACCESS_KEY_ID,
  process.env.R2_SECRET_ACCESS_KEY,
].every(Boolean)

export default buildConfig({
  serverURL: process.env.PAYLOAD_PUBLIC_SERVER_URL,
  admin: {
    user: Users.slug,
    importMap: { baseDir: path.resolve(dirname) },
    meta: { titleSuffix: ' · Spool CMS' },
  },
  collections: [
    Users,
    Media,
    Pages,
    ComparisonPages,
    CompetitorProfiles,
    CompetitorPricingSnapshots,
    ComparisonClaims,
    PricingDisplay,
    Redirects,
  ],
  globals: [SiteSettings],
  editor: lexicalEditor(),
  secret: process.env.PAYLOAD_SECRET || '',
  db: postgresAdapter({
    pool: { connectionString: process.env.DATABASE_URL || '' },
  }),
  sharp,
  telemetry: false,
  typescript: { outputFile: path.resolve(dirname, 'payload-types.ts') },
  plugins: r2Configured
    ? [
        s3Storage({
          bucket: process.env.R2_BUCKET!,
          collections: { media: true },
          config: {
            endpoint: process.env.R2_ENDPOINT!,
            region: 'auto',
            credentials: {
              accessKeyId: process.env.R2_ACCESS_KEY_ID!,
              secretAccessKey: process.env.R2_SECRET_ACCESS_KEY!,
            },
          },
        }),
      ]
    : [],
})

