import { createHmac, timingSafeEqual } from 'node:crypto';
import { env } from '$env/dynamic/private';
import { error } from '@sveltejs/kit';
import type { PageServerLoad } from './$types';

type PreviewCollection = 'comparison-pages' | 'pages';
type PreviewToken = { collection: PreviewCollection; slug: string; exp: number };
type PreviewBlock = {
  type: string;
  eyebrow?: string;
  heading?: string;
  body?: string;
  label?: string;
  href?: string | null;
  items?: Array<{ title: string; body: string }>;
};

function verifiedToken(raw: string | null): PreviewToken {
  const secret = env.CMS_PREVIEW_SECRET;
  if (!secret || !raw) error(401, 'Preview is not configured');
  const [encoded, supplied] = raw.split('.');
  if (!encoded || !supplied) error(401, 'Invalid preview token');
  const expected = createHmac('sha256', secret).update(encoded).digest();
  let received: Buffer;
  try {
    received = Buffer.from(supplied, 'base64url');
  } catch {
    error(401, 'Invalid preview token');
  }
  if (received.length !== expected.length || !timingSafeEqual(received, expected)) {
    error(401, 'Invalid preview token');
  }
  let token: unknown;
  try {
    token = JSON.parse(Buffer.from(encoded, 'base64url').toString('utf8'));
  } catch {
    error(401, 'Invalid preview token');
  }
  if (!token || typeof token !== 'object') error(401, 'Invalid preview token');
  const item = token as Record<string, unknown>;
  if (
    (item.collection !== 'pages' && item.collection !== 'comparison-pages') ||
    typeof item.slug !== 'string' ||
    !/^[a-z0-9][a-z0-9/-]{0,119}$/.test(item.slug) ||
    typeof item.exp !== 'number' ||
    !Number.isInteger(item.exp) ||
    item.exp <= Math.floor(Date.now() / 1000)
  ) {
    error(401, 'Expired or invalid preview token');
  }
  return item as PreviewToken;
}

function text(value: unknown, limit = 2_000): string {
  return typeof value === 'string' ? value.slice(0, limit) : '';
}

function lexicalText(value: unknown): string {
  const output: string[] = [];
  const visit = (node: unknown) => {
    if (!node || typeof node !== 'object' || output.join(' ').length >= 8_000) return;
    const item = node as Record<string, unknown>;
    if (typeof item.text === 'string') output.push(item.text.slice(0, 2_000));
    if (Array.isArray(item.children)) item.children.forEach(visit);
    if (item.root) visit(item.root);
  };
  visit(value);
  return output.join(' ').slice(0, 8_000);
}

function safeHref(value: unknown): string | null {
  if (typeof value !== 'string') return null;
  if (value.startsWith('/') && !value.startsWith('//')) return value.slice(0, 300);
  try {
    const url = new URL(value);
    return url.protocol === 'https:' ? url.toString().slice(0, 300) : null;
  } catch {
    return null;
  }
}

function previewBlocks(value: unknown): PreviewBlock[] {
  if (!Array.isArray(value)) return [];
  return value.slice(0, 40).map((raw) => {
    const block = raw && typeof raw === 'object' ? (raw as Record<string, unknown>) : {};
    const blockType = text(block.blockType, 40);
    if (blockType === 'richText') {
      return { type: blockType, body: lexicalText(block.content) };
    }
    if (blockType === 'featureGrid') {
      const items = Array.isArray(block.items)
        ? block.items.slice(0, 12).map((item) => ({
            title: text((item as Record<string, unknown>)?.title, 160),
            body: text((item as Record<string, unknown>)?.body)
          }))
        : [];
      return { type: blockType, heading: text(block.heading, 300), items };
    }
    return {
      type: blockType,
      eyebrow: text(block.eyebrow, 120),
      heading: text(block.heading, 400),
      body: text(block.body ?? block.lede),
      label: text(block.label ?? block.primaryLabel, 120),
      href: safeHref(block.href ?? block.primaryHref)
    };
  });
}

export const load: PageServerLoad = async ({ url, fetch, setHeaders }) => {
  setHeaders({ 'cache-control': 'no-store, private', 'x-robots-tag': 'noindex, nofollow' });
  const token = verifiedToken(url.searchParams.get('token'));
  const baseUrl = env.PAYLOAD_CMS_URL?.replace(/\/$/, '');
  if (!baseUrl) error(503, 'CMS preview is not configured');
  const query = new URLSearchParams({
    'where[slug][equals]': token.slug,
    draft: 'true',
    depth: '1',
    limit: '1'
  });
  const response = await fetch(`${baseUrl}/api/${token.collection}?${query}`, {
    headers: env.PAYLOAD_CMS_READ_TOKEN
      ? { authorization: `users API-Key ${env.PAYLOAD_CMS_READ_TOKEN}` }
      : {}
  });
  if (!response.ok) error(502, 'CMS preview could not be loaded');
  const body = (await response.json()) as { docs?: unknown[] };
  const raw = body.docs?.[0];
  if (!raw || typeof raw !== 'object') error(404, 'Draft not found');
  const doc = raw as Record<string, unknown>;
  return {
    collection: token.collection,
    title: text(doc.title, 240),
    slug: token.slug,
    summary: text(doc.summary),
    blocks: previewBlocks(doc.layout)
  };
};
