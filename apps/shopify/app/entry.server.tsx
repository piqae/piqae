// Sentry must be initialized before anything else observes the server.
import {
  captureServerException,
  serverSentryEnabled,
} from "./observability/sentry.server";

import { PassThrough } from "node:stream";

import * as Sentry from "@sentry/react-router";
import { createReadableStreamFromReadable } from "@react-router/node";
import type { RenderToPipeableStreamOptions } from "react-dom/server";
import { renderToPipeableStream } from "react-dom/server";
import type {
  AppLoadContext,
  EntryContext,
  HandleErrorFunction,
} from "react-router";
import { ServerRouter } from "react-router";
import { isbot } from "isbot";

import { safeErrorKind, sanitizeSentryUrl } from "./observability/sentry";

export const streamTimeout = 5_000;

async function handleDocumentRequest(
  request: Request,
  responseStatusCode: number,
  responseHeaders: Headers,
  routerContext: EntryContext,
  _loadContext: AppLoadContext,
): Promise<Response> {
  // https://httpwg.org/specs/rfc9110.html#HEAD
  if (request.method.toUpperCase() === "HEAD") {
    return new Response(null, {
      status: responseStatusCode,
      headers: responseHeaders,
    });
  }

  return new Promise<Response>((resolve, reject) => {
    let shellRendered = false;
    const userAgent = request.headers.get("user-agent");

    // Bots and SPA Mode renders wait for all content before responding.
    const readyOption: keyof RenderToPipeableStreamOptions =
      (userAgent && isbot(userAgent)) || routerContext.isSpaMode
        ? "onAllReady"
        : "onShellReady";

    let timeoutId: ReturnType<typeof setTimeout> | undefined = setTimeout(
      () => abort(),
      streamTimeout + 1000,
    );

    const { pipe, abort } = renderToPipeableStream(
      <ServerRouter context={routerContext} url={request.url} />,
      {
        [readyOption]() {
          shellRendered = true;
          const body = new PassThrough({
            final(callback) {
              clearTimeout(timeoutId);
              timeoutId = undefined;
              callback();
            },
          });
          const stream = createReadableStreamFromReadable(body);

          responseHeaders.set("Content-Type", "text/html");

          // Trace meta tags are only injected when Sentry is configured.
          pipe(serverSentryEnabled ? Sentry.getMetaTagTransformer(body) : body);

          resolve(
            new Response(stream, {
              headers: responseHeaders,
              status: responseStatusCode,
            }),
          );
        },
        onShellError(error: unknown) {
          reject(error instanceof Error ? error : new Error("shell render"));
        },
        onError(error: unknown) {
          responseStatusCode = 500;
          // Shell render errors reject and are reported by handleError instead.
          if (shellRendered) {
            captureServerException(error, "document-stream");
            console.error("Shopify app document stream failed", {
              kind: safeErrorKind(error),
            });
          }
        },
      },
    );
  });
}

const sentryHandleError = serverSentryEnabled
  ? Sentry.createSentryHandleError({ logErrors: false })
  : undefined;

export const handleError: HandleErrorFunction = (error, args) => {
  if (args.request.signal.aborted) return;
  sentryHandleError?.(error, args);
  // Never log the raw error: loader failures routinely carry order payloads.
  console.error("Unhandled Shopify app server error", {
    kind: safeErrorKind(error),
    route: sanitizeSentryUrl(args.request.url),
  });
};

export default serverSentryEnabled
  ? Sentry.wrapSentryHandleRequest(handleDocumentRequest)
  : handleDocumentRequest;
