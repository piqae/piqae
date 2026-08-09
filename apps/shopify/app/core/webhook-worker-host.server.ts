import pg from "pg";
import shopify from "../shopify.server";
import { createProductionServices } from "../services.server";
import { workflows } from "./workflows.server";
import { WebhookReconciliationWorker } from "./webhook-worker.server";

let worker: WebhookReconciliationWorker | undefined;
export function productionWebhookWorker(): WebhookReconciliationWorker {
  if (process.env.NODE_ENV !== "production")
    throw new Error("production webhook worker requires NODE_ENV=production");
  if (worker) return worker;
  const connectionString = process.env.DATABASE_URL;
  if (!connectionString) throw new Error("DATABASE_URL is required");
  const services = createProductionServices();
  if (!services.automationDelivery)
    throw new Error("automation delivery is not configured");
  worker = new WebhookReconciliationWorker(
    new pg.Pool({ connectionString, max: 5, statement_timeout: 10_000 }),
    {
      forShop: async (shop) =>
        (await shopify.unauthenticated.admin(shop)).admin,
    },
    workflows(),
    services.automationDelivery,
  );
  return worker;
}

export async function runWebhookWorkerBatch(limit = 100): Promise<number> {
  if (!Number.isInteger(limit) || limit < 1 || limit > 1000)
    throw new Error("worker batch limit must be 1..1000");
  const current = productionWebhookWorker();
  let processed = 0;
  while (processed < limit && (await current.runOnce())) processed += 1;
  return processed;
}
