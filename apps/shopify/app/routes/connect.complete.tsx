import { useEffect, useState } from "react";

/**
 * Native Piqae opens this top-level URL only after the one-time invitation has
 * been accepted and persisted. It intentionally contains no merchant data and
 * does not require an embedded Admin ID token: a native browser launch cannot
 * provide the short-lived App Bridge credential used inside Shopify Admin.
 */
export function loader() {
  return Response.json({ connected: true });
}

export default function ConnectComplete() {
  const [closeAttempted, setCloseAttempted] = useState(false);

  useEffect(() => {
    document.title = "Piqae connected";
  }, []);

  return (
    <main className="piqae-connect-complete">
      <section>
        <span className="piqae-connect-complete-mark" aria-hidden="true">
          ✓
        </span>
        <p className="piqae-connect-complete-eyebrow">Piqae node connection</p>
        <h1>Node connected</h1>
        <p>
          Printer access was confirmed. Shopify will update automatically, so
          you can close this tab now.
        </p>
        <button
          type="button"
          onClick={() => {
            setCloseAttempted(true);
            window.close();
          }}
        >
          Close this tab
        </button>
        {closeAttempted ? (
          <small>If the tab stays open, close it with your browser.</small>
        ) : null}
      </section>
    </main>
  );
}
