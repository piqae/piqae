import type { ActionFunctionArgs } from "react-router";
import shopify from "../shopify.server";
import { createProductionServices } from "../services.server";
import {
  markInstallationUninstalled,
  redactInstallation,
} from "../core/installations.server";

export async function action({ request }: ActionFunctionArgs) {
  const { topic, shop, payload, webhookId, resourceId } =
    await shopify.authenticate.webhook(request);
  const services = createProductionServices();
  const synchronousLifecycle = ["APP_UNINSTALLED", "SHOP_REDACT", "CUSTOMERS_REDACT", "CUSTOMERS_DATA_REQUEST"].includes(topic);
  if (!synchronousLifecycle &&
    !(await services.repository.claimWebhook(webhookId, {
      shop,
      topic,
      resourceId,
    }))
  )
    return new Response(null, { status: 200 });
  switch (topic) {
    case "APP_UNINSTALLED":
      await services.repository.deleteShop(shop);
      await markInstallationUninstalled(shop);
      await shopify.sessionStorage.deleteSessions(
        (await shopify.sessionStorage.findSessionsByShop(shop)).map(
          (session) => session.id,
        ),
      );
      break;
    case "SHOP_REDACT":
      await redactInstallation(shop);
      await shopify.sessionStorage.deleteSessions(
        (await shopify.sessionStorage.findSessionsByShop(shop)).map(
          (session) => session.id,
        ),
      );
      break;
    case "CUSTOMERS_REDACT":
      await services.repository.redactCustomer(
        shop,
        String((payload as { customer?: { id?: unknown } }).customer?.id ?? ""),
      );
      break;
    case "CUSTOMERS_DATA_REQUEST":
      // Piqae stores no customer profiles: order data exists only in encrypted,
      // retention-bounded render inputs and print jobs governed by the linked tenant.
      break;
    default:
      // Operational topics are durably deduplicated before this acknowledgement.
      // A worker must refetch the resource with GraphQL rather than trusting payload data.
      break;
  }
  return new Response(null, { status: 200 });
}
