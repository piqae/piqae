export function preparePiqaeConnectionWindow(
  openWindow: typeof window.open = window.open.bind(window),
) {
  const connectionWindow = openWindow(
    "",
    "piqae-node-connection",
    "popup,width=560,height=720",
  );
  if (!connectionWindow) return null;
  try {
    connectionWindow.opener = null;
    connectionWindow.document.title = "Opening Piqae…";
    const status = connectionWindow.document.createElement("p");
    status.textContent = "Preparing your secure Piqae node connection…";
    status.style.cssText =
      "margin:25vh auto;max-width:24rem;padding:2rem;color:#202223;font:600 18px system-ui;text-align:center";
    connectionWindow.document.body.replaceChildren(status);
  } catch {
    // The reserved window may already be navigating. The handoff can still use it.
  }
  return connectionWindow;
}

export function openPreparedPiqaeConnection(
  connectionWindow: Window | null,
  connectUrl: string,
) {
  if (!connectionWindow || connectionWindow.closed) return false;
  try {
    const url = new URL(connectUrl);
    if (url.protocol !== "https:" || url.hostname !== "app.piqae.com")
      return false;
    connectionWindow.location.replace(url.toString());
    return true;
  } catch {
    return false;
  }
}
