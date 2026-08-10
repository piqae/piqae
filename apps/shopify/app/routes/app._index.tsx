import type { LoaderFunctionArgs } from "react-router";
import { useLoaderData } from "react-router";
import { EmptyHint, StatusBadge } from "../components/shopify-ui";
import shopify from "../shopify.server";
import { createProductionServices } from "../services.server";

export async function loader({ request }: LoaderFunctionArgs) {
  const { session } = await shopify.authenticate.admin(request);
  const link = await createProductionServices().repository.get(session.shop);
  return {
    connected: Boolean(link),
    templateRevisionId: link?.templateRevisionId ?? null,
    entitlementMode: link?.entitlementMode ?? null,
    planHandle: link?.planHandle ?? null,
  };
}

export default function Home() {
  const state = useLoaderData<typeof loader>();
  return (
    <s-page heading="Order printing">
      <s-button slot="primary-action" href="/app/print" variant="primary">
        Print orders
      </s-button>
      <s-section>
        <s-stack direction="block" gap="base">
          <s-banner tone="info">
            Print directly when a Piqae node is online. A downloadable PDF is
            always available as a fallback.
          </s-banner>
          <div className="piqae-grid">
            <div className="piqae-card">
              <s-stack direction="block" gap="base">
                <s-heading>Printer connection</s-heading>
                <StatusBadge
                  state={state.connected ? "ready" : "not_connected"}
                />
                <s-paragraph>
                  Connect a printer once, then print from Shopify Admin or POS
                  without downloading files.
                </s-paragraph>
                <s-button href="/app/settings">
                  {state.connected ? "Manage connection" : "Connect Piqae"}
                </s-button>
              </s-stack>
            </div>
            <div className="piqae-card">
              <s-stack direction="block" gap="base">
                <s-heading>Default documents</s-heading>
                <s-paragraph>
                  {state.templateRevisionId
                    ? `Published revision ${state.templateRevisionId}`
                    : "No default template published"}
                </s-paragraph>
                <s-button href="/app/templates">Manage templates</s-button>
              </s-stack>
            </div>
            <div className="piqae-card">
              <s-stack direction="block" gap="base">
                <s-heading>This month</s-heading>
                <s-heading>
                  {state.planHandle ??
                    (state.entitlementMode === "existing_piqae"
                      ? "Existing Piqae plan"
                      : "No active plan")}
                </s-heading>
                <s-paragraph>
                  {state.connected
                    ? "View authoritative usage and billing details."
                    : "Connect an account to begin."}
                </s-paragraph>
                <s-button href="/app/billing">View plan</s-button>
              </s-stack>
            </div>
          </div>
          <s-heading>Get ready to print</s-heading>
          <div className="piqae-grid">
            <EmptyHint
              heading="1. Connect a printer"
              action="Connect Piqae"
              href="/app/settings"
            >
              Use an existing Piqae account or activate the included
              Shopify-only account.
            </EmptyHint>
            <EmptyHint
              heading="2. Choose templates"
              action="View templates"
              href="/app/templates"
            >
              Start with accessible invoices, packing slips, returns forms and
              receipts.
            </EmptyHint>
            <EmptyHint
              heading="3. Print an order"
              action="Choose orders"
              href="/app/print"
            >
              Direct print is preferred after you explicitly choose a printer;
              PDF download is always a separate option.
            </EmptyHint>
          </div>
        </s-stack>
      </s-section>
    </s-page>
  );
}
