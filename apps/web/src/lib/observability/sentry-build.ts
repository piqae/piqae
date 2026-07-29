export interface SentryBuildConfiguration {
  uploadSourceMaps: boolean;
  authToken?: string;
  organization?: string;
  project?: string;
  release?: string;
}

type BuildEnvironment = Record<string, string | undefined>;

function present(value: string | undefined): string | undefined {
  const normalized = value?.trim();
  return normalized ? normalized : undefined;
}

export function resolveSentryBuildConfiguration(
  environment: BuildEnvironment
): SentryBuildConfiguration {
  const authToken = present(environment.SENTRY_AUTH_TOKEN);
  const organization = present(environment.SENTRY_ORG);
  const project = present(environment.SENTRY_PROJECT);
  const release = present(environment.SENTRY_RELEASE);
  const uploadRequested = Boolean(authToken || organization || project);

  if (!uploadRequested) return { uploadSourceMaps: false };

  const missing = [
    ['SENTRY_AUTH_TOKEN', authToken],
    ['SENTRY_ORG', organization],
    ['SENTRY_PROJECT', project],
    ['SENTRY_RELEASE', release]
  ]
    .filter(([, value]) => !value)
    .map(([name]) => name);

  if (missing.length > 0) {
    throw new Error(
      `Sentry source-map upload configuration is incomplete; missing ${missing.join(', ')}`
    );
  }

  if (release && (release.length > 200 || /[\r\n\0]/.test(release))) {
    throw new Error('SENTRY_RELEASE must be a single release identifier of at most 200 characters');
  }

  return {
    uploadSourceMaps: true,
    authToken,
    organization,
    project,
    release
  };
}
