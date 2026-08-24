import { startTransition, StrictMode } from "react";
import { hydrateRoot } from "react-dom/client";
import { HydratedRouter } from "react-router/dom";
import { initializeBrowserSentry } from "./observability/sentry.client";

// Resolves immediately when browser reporting is not configured, so hydration
// is not delayed and the Sentry SDK is never fetched.
void initializeBrowserSentry().then((onError) => {
  startTransition(() => {
    hydrateRoot(
      document,
      <StrictMode>
        <HydratedRouter onError={onError} />
      </StrictMode>,
    );
  });
});
