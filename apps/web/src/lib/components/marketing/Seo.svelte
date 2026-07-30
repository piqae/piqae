<script lang="ts">
  import { env as publicEnv } from '$env/dynamic/public';

  let {
    title,
    description,
    path,
    noindex = false,
    image,
    structuredData
  }: {
    title: string;
    description: string;
    path: string;
    noindex?: boolean;
    image?: string;
    structuredData?: Record<string, unknown> | Record<string, unknown>[];
  } = $props();

  const siteName = 'Piqae';
  const origin = publicEnv.PUBLIC_SITE_URL?.replace(/\/$/, '');
  const canonical = $derived(origin ? `${origin}${path}` : null);
  const indexable = $derived(
    !noindex && publicEnv.PUBLIC_MARKETING_INDEXABLE === 'true' && Boolean(origin)
  );
  const socialImage = $derived(image ?? (origin ? `${origin}/og-default.svg` : null));
  const googleVerification = publicEnv.PUBLIC_GOOGLE_SITE_VERIFICATION;
  const bingVerification = publicEnv.PUBLIC_BING_SITE_VERIFICATION;
  const serializedStructuredData = $derived(
    structuredData ? JSON.stringify(structuredData).replaceAll('<', '\\u003c') : null
  );
</script>

<svelte:head>
  <title>{title}</title>
  <meta name="description" content={description} />
  <meta name="robots" content={indexable ? 'index,follow' : 'noindex,nofollow'} />
  <meta property="og:type" content="website" />
  <meta property="og:site_name" content={siteName} />
  <meta property="og:title" content={title} />
  <meta property="og:description" content={description} />
  <meta name="twitter:card" content="summary_large_image" />
  <meta name="twitter:title" content={title} />
  <meta name="twitter:description" content={description} />
  {#if googleVerification}
    <meta name="google-site-verification" content={googleVerification} />
  {/if}
  {#if bingVerification}
    <meta name="msvalidate.01" content={bingVerification} />
  {/if}
  {#if canonical}
    <link rel="canonical" href={canonical} />
    <meta property="og:url" content={canonical} />
  {/if}
  {#if socialImage}
    <meta property="og:image" content={socialImage} />
    <meta name="twitter:image" content={socialImage} />
  {/if}
  {#if serializedStructuredData}
    <script type="application/ld+json">{serializedStructuredData}</script>
  {/if}
</svelte:head>
