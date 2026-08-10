import { createHash, randomUUID } from "node:crypto";
import type { Pool } from "pg";
import type { AdminGraphql } from "./orders.server";
import type { WorkflowRepository, AutomationRule } from "./workflows.server";
import { EmailDeliveryError } from "./cloudflare-email.server";

export interface WebhookAdminFactory {
  forShop(shop: string): Promise<AdminGraphql>;
}
export interface AutomationDelivery {
  print(input: {
    shop: string;
    resourceId: string;
    templateId: string;
    printerId: string;
    idempotencyKey: string;
    admin: AdminGraphql;
  }): Promise<{
    activityId: string;
    orderName: string;
    state?: "accepted" | "printing" | "reported_complete" | "uncertain";
  }>;
  enqueueEmail?(input: {
    shop: string;
    resourceId: string;
    templateId: string;
    recipient: string;
    idempotencyKey: string;
    admin: AdminGraphql;
  }): Promise<{
    activityId: string;
    orderName: string;
    state?: "accepted" | "reported_complete";
  }>;
}

export class WebhookReconciliationWorker {
  constructor(
    private readonly pool: Pool,
    private readonly admins: WebhookAdminFactory,
    private readonly workflow?: WorkflowRepository,
    private readonly delivery?: AutomationDelivery,
  ) {}
  async runOnce(): Promise<boolean> {
    const token = randomUUID();
    const claimed = await this.pool.query(
      `WITH candidate AS (
      SELECT webhook_id FROM shopify_webhook_inbox WHERE processed_at IS NULL AND available_at <= now()
        AND attempts < 12 AND (lease_expires_at IS NULL OR lease_expires_at < now()) ORDER BY received_at FOR UPDATE SKIP LOCKED LIMIT 1
    ) UPDATE shopify_webhook_inbox i SET lease_token=$1, lease_expires_at=now()+interval '30 seconds', attempts=attempts+1
      FROM candidate WHERE i.webhook_id=candidate.webhook_id RETURNING i.webhook_id,i.shop,i.topic,i.resource_id,i.attempts`,
      [token],
    );
    const event = claimed.rows[0];
    if (!event) return false;
    try {
      let admin: AdminGraphql | undefined;
      if (event.shop && event.resource_id && !privacyTopic(event.topic)) {
        admin = await this.admins.forShop(event.shop);
        const response = await admin.graphql(
          `query PiqaeWebhookReconcile($id: ID!) { node(id: $id) { id } }`,
          { variables: { id: event.resource_id } },
        );
        if (!response.ok)
          throw new Error(`GraphQL refetch failed (${response.status})`);
        const body = (await response.json()) as any;
        if (body.errors?.length) throw new Error("GraphQL refetch rejected");
        await this.runAutomations(event, admin);
      }
      await this.pool.query(
        "UPDATE shopify_webhook_inbox SET processed_at=now(),lease_token=NULL,lease_expires_at=NULL,last_error=NULL WHERE webhook_id=$1 AND lease_token=$2",
        [event.webhook_id, token],
      );
    } catch (error) {
      if (error instanceof EmailDeliveryError && !error.retryable) {
        await this.pool.query(
          "UPDATE shopify_webhook_inbox SET processed_at=now(),lease_token=NULL,lease_expires_at=NULL,last_error=$3 WHERE webhook_id=$1 AND lease_token=$2",
          [event.webhook_id, token, error.message.slice(0, 500)],
        );
        return true;
      }
      if (Number(event.attempts) >= 12) {
        await this.pool.query(
          "UPDATE shopify_webhook_inbox SET processed_at=now(),lease_token=NULL,lease_expires_at=NULL,last_error=$3 WHERE webhook_id=$1 AND lease_token=$2",
          [
            event.webhook_id,
            token,
            `retry exhausted: ${error instanceof Error ? error.message.slice(0, 480) : "unknown"}`,
          ],
        );
        return true;
      }
      const delaySeconds =
        Math.min(3600, 2 ** Math.min(10, Number(event.attempts))) *
        (0.75 + Math.random() * 0.5);
      await this.pool.query(
        "UPDATE shopify_webhook_inbox SET available_at=now()+($3*interval '1 second'),lease_token=NULL,lease_expires_at=NULL,last_error=$4 WHERE webhook_id=$1 AND lease_token=$2",
        [
          event.webhook_id,
          token,
          delaySeconds,
          error instanceof Error
            ? error.message.slice(0, 500)
            : "unknown error",
        ],
      );
    }
    return true;
  }
  private async runAutomations(event: any, admin: AdminGraphql): Promise<void> {
    const trigger = triggerForTopic(event.topic);
    if (!trigger) return;
    if (!this.workflow || !this.delivery)
      throw new Error("AUTOMATION_DELIVERY_NOT_CONFIGURED");
    const rules = (await this.workflow.listAutomations(event.shop)).filter(
      (rule) => rule.enabled && rule.trigger === trigger,
    );
    const failures: unknown[] = [];
    for (const rule of rules) {
      const key = `shopify-webhook:${event.webhook_id}:automation:${rule.id}`;
      try {
        const result =
          rule.delivery === "printer"
            ? await this.delivery.print({
                shop: event.shop,
                resourceId: event.resource_id,
                templateId: rule.templateId,
                printerId: rule.destination,
                idempotencyKey: key,
                admin,
              })
            : await this.enqueueEmail(rule, event, key, admin);
        await this.workflow.recordActivity(event.shop, {
          id: result.activityId,
          orderName: result.orderName,
          documentName: rule.name,
          destination: rule.destination,
          state: result.state ?? "accepted",
        });
      } catch (error) {
        await this.workflow.recordActivity(event.shop, {
          id: stableActivityId(key),
          orderName: String(event.resource_id),
          documentName: rule.name,
          destination: rule.destination,
          state: "failed",
        });
        failures.push(error);
      }
    }
    if (failures.length) {
      const retryable = failures.find(
        (error) => !(error instanceof EmailDeliveryError) || error.retryable,
      );
      throw retryable ?? failures[0];
    }
  }
  private async enqueueEmail(
    rule: AutomationRule,
    event: any,
    idempotencyKey: string,
    admin: AdminGraphql,
  ) {
    if (!this.delivery?.enqueueEmail)
      throw new Error("EMAIL_PROVIDER_NOT_CONFIGURED");
    return this.delivery.enqueueEmail({
      shop: event.shop,
      resourceId: event.resource_id,
      templateId: rule.templateId,
      recipient: rule.destination,
      idempotencyKey,
      admin,
    });
  }
}

function privacyTopic(topic: string) {
  return [
    "APP_UNINSTALLED",
    "SHOP_REDACT",
    "CUSTOMERS_REDACT",
    "CUSTOMERS_DATA_REQUEST",
  ].includes(topic);
}
function triggerForTopic(topic: string): AutomationRule["trigger"] | null {
  return (
    (
      {
        ORDERS_PAID: "order_paid",
        ORDERS_CREATE: "order_created",
        FULFILLMENTS_CREATE: "fulfillment_created",
        REFUNDS_CREATE: "refund_created",
      } as const
    )[topic as "ORDERS_PAID"] ?? null
  );
}
function stableActivityId(value: string) {
  const h = createHash("sha256")
    .update(value)
    .digest("hex")
    .slice(0, 32)
    .split("");
  h[12] = "4";
  h[16] = ((parseInt(h[16]!, 16) & 3) | 8).toString(16);
  return `${h.slice(0, 8).join("")}-${h.slice(8, 12).join("")}-${h.slice(12, 16).join("")}-${h.slice(16, 20).join("")}-${h.slice(20).join("")}`;
}
