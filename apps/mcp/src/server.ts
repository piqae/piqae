import { randomUUID } from "node:crypto";
import { readFile, readdir, stat } from "node:fs/promises";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { PiqaeClient, PiqaeError } from "@piqae/sdk";
import type {
  ApiKeyScope,
  CreateJob,
  CreateStock,
  CreateTarget,
  CreateTargetBinding,
  CreateUpload,
  JobListOptions,
  JobOptions,
  PatchStock,
  PatchTarget,
  PlatformAccount,
} from "@piqae/sdk";
import { z } from "zod/v4";
import { rawRequest, tenantClient, bearer, type AuthContext } from "./api.js";
import type { McpConfig } from "./config.js";
import {
  assertSecretDeliveryReady,
  deliverSecret,
  type SecretDelivery,
} from "./secrets.js";

const VERSION = "0.1.0";
const apiKeyScopes = [
  "api_keys_read",
  "api_keys_write",
  "agents_read",
  "agents_write",
  "printers_read",
  "printers_write",
  "jobs_read",
  "jobs_write",
  "webhooks_read",
  "webhooks_write",
  "usage_read",
  "audit_read",
] as const;
const selectionShape = {
  workspace_id: z
    .string()
    .min(1)
    .optional()
    .describe("Required with environment_id for a platform credential."),
  environment_id: z
    .string()
    .min(1)
    .optional()
    .describe("Required with workspace_id for a platform credential."),
};
const deliverySchema = z
  .enum(["file", "response"])
  .default("file")
  .describe(
    "file stores the one-time secret outside the model transcript; response requires an explicit server opt-in.",
  );
const confirmSchema = z
  .string()
  .min(1)
  .describe(
    "Must exactly repeat the resource identifier being revoked, removed, or archived.",
  );
const recordSchema = z.record(z.string(), z.unknown());

type ToolExtra = AuthContext;

export function createPiqaeMcpServer(config: McpConfig): McpServer {
  const server = new McpServer(
    { name: "piqae", version: VERSION },
    { capabilities: { logging: {} }, instructions: SERVER_INSTRUCTIONS },
  );

  registerResources(server);
  registerPrompts(server);
  registerContextTool(server, config);
  registerApiKeyTool(server, config);
  registerNodeTools(server, config);
  registerPrinterTool(server, config);
  registerPrintIntentTool(server, config);
  registerStockTool(server, config);
  registerTargetTool(server, config);
  registerUploadTool(server, config);
  registerJobTool(server, config);
  registerWebhookTool(server, config);
  registerPlatformTool(server, config);
  registerDocumentationTool(server);
  return server;
}

function registerContextTool(server: McpServer, config: McpConfig): void {
  server.registerTool(
    "piqae_context",
    {
      title: "Inspect Piqae context",
      description:
        "Read deployment health/capabilities, authenticated identity, workspace, members, billing, usage, or platform-integration status. Start here before operating resources.",
      inputSchema: z.object({
        action: z.enum([
          "health",
          "ready",
          "meta",
          "identity",
          "workspace",
          "members",
          "billing",
          "usage",
          "platform_status",
        ]),
        month: z
          .string()
          .regex(/^\d{4}-\d{2}$/)
          .optional(),
        ...selectionShape,
      }),
      annotations: readOnlyAnnotations("Inspect Piqae context"),
    },
    (input, extra) =>
      result(async () => {
        if (
          input.action === "health" ||
          input.action === "ready" ||
          input.action === "meta"
        ) {
          const client = new PiqaeClient({ baseUrl: config.apiOrigin });
          if (input.action === "health") return client.health();
          if (input.action === "ready") return client.ready();
          return client.meta();
        }
        const client = clientFor(config, extra, input);
        switch (input.action) {
          case "identity":
            return client.identity.me();
          case "workspace":
            return client.workspaces.current();
          case "members":
            return client.workspaces.members();
          case "billing":
            return client.billing.summary();
          case "usage":
            return client.usage.retrieve(input.month);
          case "platform_status":
            return rawRequest(
              config,
              extra,
              "GET",
              "/v1/platform/status",
              undefined,
              selection(input),
            );
          default:
            return unreachable(input.action);
        }
      }),
  );
}

function registerApiKeyTool(server: McpServer, config: McpConfig): void {
  server.registerTool(
    "piqae_api_keys",
    {
      title: "Manage Piqae API keys",
      description:
        "List, create, or revoke environment-scoped API keys. Creation cannot grant scopes the caller lacks. One-time secrets default to an owner-only configured file.",
      inputSchema: z.discriminatedUnion("action", [
        z.object({ action: z.literal("list"), ...selectionShape }),
        z.object({
          action: z.literal("create"),
          name: z.string().min(1).max(120),
          scopes: z.array(z.enum(apiKeyScopes)).min(1),
          expires_at: z.iso.datetime().nullable().optional(),
          delivery: deliverySchema,
          ...selectionShape,
        }),
        z.object({
          action: z.literal("revoke"),
          key_id: z.string().uuid(),
          confirm: confirmSchema,
          ...selectionShape,
        }),
      ]),
      annotations: mutationAnnotations("Manage Piqae API keys", true),
    },
    (input, extra) =>
      result(async () => {
        const client = clientFor(config, extra, input);
        if (input.action === "list") return client.apiKeys.list();
        if (input.action === "revoke") {
          requireConfirmation(input.key_id, input.confirm);
          return client.apiKeys.revoke(input.key_id);
        }
        await assertSecretDeliveryReady(config, input.delivery);
        const created = await client.apiKeys.create({
          name: input.name,
          scopes: input.scopes as ApiKeyScope[],
          ...(input.expires_at === undefined
            ? {}
            : { expires_at: input.expires_at }),
        });
        const delivered = await deliverSecret(
          config,
          "api-key",
          created.id,
          created.secret,
          input.delivery,
        );
        const { secret: _secret, ...metadata } = created;
        return { ...metadata, secret_delivery: delivered };
      }),
  );
}

function registerNodeTools(server: McpServer, config: McpConfig): void {
  server.registerTool(
    "piqae_nodes",
    {
      title: "Operate Piqae nodes",
      description:
        "List and inspect nodes; rename, pause, resume, request diagnostics, manage update policy, queue signed updates/rollback, and inspect or revoke tenant connector grants.",
      inputSchema: z.object({
        action: z.enum([
          "list",
          "get",
          "rename",
          "pause",
          "resume",
          "diagnostics",
          "update_status",
          "update_policy",
          "request_update",
          "rollback",
          "connectors",
          "revoke_connector",
          "revoke_node",
        ]),
        node_id: z.string().min(1).optional(),
        name: z.string().min(1).max(120).optional(),
        channel: z.enum(["stable", "canary", "pinned"]).optional(),
        mode: z.enum(["automatic", "prompt", "disabled"]).optional(),
        pinned_version: z.string().nullable().optional(),
        maintenance_window: recordSchema.nullable().optional(),
        version: z.string().min(1).max(80).optional(),
        metadata_url: z.url().optional(),
        connector_id: z.string().min(1).optional(),
        confirm: z.string().optional(),
        ...selectionShape,
      }),
      annotations: mutationAnnotations("Operate Piqae nodes", true),
    },
    (input, extra) =>
      result(async () => {
        const client = clientFor(config, extra, input);
        if (input.action === "list") return client.nodes.list();
        const nodeId = required(input.node_id, "node_id");
        switch (input.action) {
          case "get":
            return client.nodes.retrieve(nodeId);
          case "rename":
            return client.nodes.rename(nodeId, required(input.name, "name"));
          case "pause":
            return client.nodes.pause(nodeId);
          case "resume":
            return client.nodes.resume(nodeId);
          case "diagnostics":
            return client.nodes.diagnostics(nodeId);
          case "update_status":
            return client.nodes.update(nodeId);
          case "update_policy":
            return client.nodes.updatePolicy(nodeId, {
              channel: required(input.channel, "channel"),
              mode: required(input.mode, "mode"),
              pinned_version: input.pinned_version ?? null,
              maintenance_window: input.maintenance_window ?? null,
            });
          case "request_update":
            return client.nodes.requestUpdate(
              nodeId,
              required(input.version, "version"),
              required(input.metadata_url, "metadata_url"),
            );
          case "rollback":
            requireConfirmation(nodeId, input.confirm);
            return client.nodes.rollback(
              nodeId,
              required(input.metadata_url, "metadata_url"),
            );
          case "connectors":
            return client.nodes.connectors(nodeId);
          case "revoke_connector": {
            const connectorId = required(input.connector_id, "connector_id");
            requireConfirmation(connectorId, input.confirm);
            return client.nodes.revokeConnector(nodeId, connectorId);
          }
          case "revoke_node":
            requireConfirmation(nodeId, input.confirm);
            return client.nodes.revoke(nodeId);
          default:
            return unreachable(input.action);
        }
      }),
  );

  server.registerTool(
    "piqae_node_onboarding",
    {
      title: "Set up Piqae nodes",
      description:
        "Create one-time headless enrolments or application-link connect sessions, poll a connect session, and review/approve/deny a node device-authorization request. Device private keys remain node-owned.",
      inputSchema: z.object({
        action: z.enum([
          "create_enrolment",
          "create_connect_session",
          "get_connect_session",
          "review_pairing",
          "approve_pairing",
          "deny_pairing",
        ]),
        name: z.string().min(1).max(120).optional(),
        expires_in_seconds: z.number().int().min(60).max(3600).optional(),
        return_url: z.url().optional(),
        session_id: z.string().min(1).optional(),
        authorization_id: z.string().min(1).optional(),
        user_code: z.string().min(1).max(32).optional(),
        delivery: deliverySchema,
        ...selectionShape,
      }),
      annotations: mutationAnnotations("Set up Piqae nodes", false),
    },
    (input, extra) =>
      result(async () => {
        const client = clientFor(config, extra, input);
        if (input.action === "get_connect_session") {
          return client.connectSessions.retrieve(
            required(input.session_id, "session_id"),
          );
        }
        if (input.action === "review_pairing") {
          return client.pairing.review(
            required(input.authorization_id, "authorization_id"),
          );
        }
        if (input.action === "approve_pairing") {
          return client.pairing.approve(
            required(input.authorization_id, "authorization_id"),
            required(input.user_code, "user_code"),
          );
        }
        if (input.action === "deny_pairing") {
          return client.pairing.deny(
            required(input.authorization_id, "authorization_id"),
            required(input.user_code, "user_code"),
          );
        }
        await assertSecretDeliveryReady(config, input.delivery);
        if (input.action === "create_enrolment") {
          const enrollment = await client.agents.createEnrolment({
            name: required(input.name, "name"),
            ...(input.expires_in_seconds === undefined
              ? {}
              : {
                  expires_in_seconds: Math.min(input.expires_in_seconds, 3600),
                }),
          });
          const delivered = await deliverSecret(
            config,
            "enrolment",
            enrollment.id,
            enrollment.token,
            input.delivery,
          );
          const { token: _token, ...metadata } = enrollment;
          return { ...metadata, token_delivery: delivered };
        }
        if (
          input.expires_in_seconds !== undefined &&
          input.expires_in_seconds > 900
        ) {
          throw new Error(
            "expires_in_seconds must be 900 or less for a connect session.",
          );
        }
        const session = await client.connectSessions.create({
          name: required(input.name, "name"),
          ...(input.return_url ? { return_url: input.return_url } : {}),
          ...(input.expires_in_seconds === undefined
            ? {}
            : { expires_in_seconds: input.expires_in_seconds }),
        });
        const connectUrl = required(
          session.connect_url,
          "connect_url in create response",
        );
        const delivered = await deliverSecret(
          config,
          "enrolment",
          session.id,
          connectUrl,
          input.delivery,
        );
        return {
          ...session,
          connect_url: null,
          connect_url_delivery: delivered,
        };
      }),
  );
}

function registerPrinterTool(server: McpServer, config: McpConfig): void {
  server.registerTool(
    "piqae_printers",
    {
      title: "Inspect Piqae printers",
      description:
        "List printers with pagination, retrieve exact synced driver capabilities/profiles, or retrieve the node content-encryption public key. This tool never submits a print.",
      inputSchema: z.object({
        action: z.enum(["list", "get", "capabilities", "loaded_media", "content_encryption_key"]),
        printer_id: z.string().min(1).optional(),
        limit: z.number().int().min(1).max(100).optional(),
        after: z.string().optional(),
        ...selectionShape,
      }),
      annotations: readOnlyAnnotations("Inspect Piqae printers"),
    },
    (input, extra) =>
      result(async () => {
        const client = clientFor(config, extra, input);
        if (input.action === "list") {
          return client.printers.list({
            ...(input.limit === undefined ? {} : { limit: input.limit }),
            ...(input.after === undefined ? {} : { after: input.after }),
          });
        }
        const id = required(input.printer_id, "printer_id");
        if (input.action === "get") return client.printers.retrieve(id);
        if (input.action === "capabilities") return client.printers.capabilities(id);
        if (input.action === "loaded_media") return client.printers.loadedMedia(id);
        return client.printers.contentEncryptionKey(id);
      }),
  );
}

function registerPrintIntentTool(server: McpServer, config: McpConfig): void {
  server.registerTool(
    "piqae_validate_print_intent",
    {
      title: "Validate a Piqae print intent",
      description:
        "Validates normalized printer, workflow, stock, document, and job-scoped options. This tool never submits or resolves a print.",
      inputSchema: z.object({ intent: recordSchema, ...selectionShape }),
      annotations: readOnlyAnnotations("Validate a Piqae print intent"),
    },
    (input, extra) => result(async () => {
      const client = clientFor(config, extra, input);
      return client.printIntents.validate(input.intent as import("@piqae/sdk").PrintIntent);
    }),
  );
}

function registerStockTool(server: McpServer, config: McpConfig): void {
  server.registerTool(
    "piqae_stocks",
    {
      title: "Manage Piqae stocks",
      description:
        "List, create, or update logical media/stock definitions used by targets and design specifications.",
      inputSchema: z.object({
        action: z.enum(["list", "create", "update"]),
        stock_id: z.string().min(1).optional(),
        name: z.string().min(1).max(120).optional(),
        sku: z.string().max(120).optional(),
        description: z.string().max(1000).optional(),
        attributes: recordSchema.optional(),
        archived: z.boolean().optional(),
        ...selectionShape,
      }),
      annotations: mutationAnnotations("Manage Piqae stocks", false),
    },
    (input, extra) =>
      result(async () => {
        const client = clientFor(config, extra, input);
        if (input.action === "list") return client.stocks.list();
        const body = withoutUndefined({
          name: input.name,
          sku: input.sku,
          description: input.description,
          attributes: input.attributes,
          archived: input.archived,
        });
        if (input.action === "create") {
          return client.stocks.create({
            ...body,
            name: required(input.name, "name"),
          } as CreateStock);
        }
        return client.stocks.update(
          required(input.stock_id, "stock_id"),
          body as PatchStock,
        );
      }),
  );
}

function registerTargetTool(server: McpServer, config: McpConfig): void {
  server.registerTool(
    "piqae_targets",
    {
      title: "Manage Piqae routing targets",
      description:
        "List/create/update logical print targets, manage immutable printer-profile bindings, and inspect readiness or a complete design specification.",
      inputSchema: z.object({
        action: z.enum([
          "list",
          "create",
          "update",
          "bindings",
          "bind",
          "unbind",
          "readiness",
          "design_specification",
        ]),
        target_id: z.string().min(1).optional(),
        binding_id: z.string().min(1).optional(),
        name: z.string().min(1).max(120).optional(),
        description: z.string().max(1000).optional(),
        stock_id: z.string().min(1).optional(),
        clear_stock: z.boolean().optional(),
        enabled: z.boolean().optional(),
        printer_id: z.string().min(1).optional(),
        profile_id: z.string().min(1).optional(),
        profile_revision: z.number().int().positive().optional(),
        role: z.enum(["primary", "standby"]).optional(),
        confirm: z.string().optional(),
        ...selectionShape,
      }),
      annotations: mutationAnnotations("Manage Piqae routing targets", true),
    },
    (input, extra) =>
      result(async () => {
        const client = clientFor(config, extra, input);
        if (input.action === "list") return client.targets.list();
        if (input.action === "create") {
          return client.targets.create(
            withoutUndefined({
              name: required(input.name, "name"),
              description: input.description,
              stock_id: input.stock_id,
              enabled: input.enabled,
              routing_policy: "primary_then_standby",
            }) as CreateTarget,
          );
        }
        const targetId = required(input.target_id, "target_id");
        switch (input.action) {
          case "update":
            return client.targets.update(
              targetId,
              withoutUndefined({
                name: input.name,
                description: input.description,
                stock_id: input.stock_id,
                clear_stock: input.clear_stock,
                enabled: input.enabled,
              }) as PatchTarget,
            );
          case "bindings":
            return client.targets.bindings(targetId);
          case "bind":
            return client.targets.bind(
              targetId,
              withoutUndefined({
                printer_id: required(input.printer_id, "printer_id"),
                profile_id: required(input.profile_id, "profile_id"),
                profile_revision: required(
                  input.profile_revision,
                  "profile_revision",
                ),
                role: required(input.role, "role"),
                enabled: input.enabled,
              }) as CreateTargetBinding,
            );
          case "unbind": {
            const bindingId = required(input.binding_id, "binding_id");
            requireConfirmation(bindingId, input.confirm);
            return client.targets.unbind(targetId, bindingId);
          }
          case "readiness":
            return client.targets.readiness(targetId);
          case "design_specification":
            return client.targets.designSpecification(targetId);
          default:
            return unreachable(input.action);
        }
      }),
  );
}

function registerUploadTool(server: McpServer, config: McpConfig): void {
  server.registerTool(
    "piqae_uploads",
    {
      title: "Manage Piqae upload metadata",
      description:
        "Create/retrieve/complete upload objects. Binary PUT is intentionally not tunneled through the model; use returned upload_url/upload_headers from trusted application code.",
      inputSchema: z.object({
        action: z.enum(["create", "get", "complete"]),
        upload_id: z.string().min(1).optional(),
        media_type: z
          .enum(["application/pdf", "application/octet-stream"])
          .optional(),
        byte_length: z
          .number()
          .int()
          .positive()
          .max(50 * 1024 * 1024)
          .optional(),
        sha256: z
          .string()
          .regex(/^[A-Fa-f0-9]{64}$/)
          .optional(),
        ...selectionShape,
      }),
      annotations: mutationAnnotations("Manage Piqae uploads", false),
    },
    (input, extra) =>
      result(async () => {
        const client = clientFor(config, extra, input);
        if (input.action === "create") {
          return client.uploads.create({
            media_type: required(input.media_type, "media_type"),
            byte_length: required(input.byte_length, "byte_length"),
            sha256: required(input.sha256, "sha256"),
          } as CreateUpload);
        }
        const id = required(input.upload_id, "upload_id");
        if (input.action === "get") return client.uploads.retrieve(id);
        return client.uploads.complete(
          id,
          required(input.sha256, "sha256"),
          required(input.byte_length, "byte_length"),
        );
      }),
  );
}

function registerJobTool(server: McpServer, config: McpConfig): void {
  server.registerTool(
    "piqae_jobs",
    {
      title: "Inspect and operate Piqae jobs",
      description:
        "List/get job lifecycle and events, cancel a durable job, or register a URI/upload-backed job. Registration means accepted, never proof that ink reached paper. Submission is server-policy gated and requires an exact destination confirmation plus fixture description.",
      inputSchema: z.object({
        action: z.enum(["list", "get", "events", "create", "cancel"]),
        job_id: z.string().min(1).optional(),
        state: z.string().optional(),
        printer_id: z.string().min(1).optional(),
        target_id: z.string().min(1).optional(),
        metadata_key: z.string().optional(),
        metadata_value: z.string().optional(),
        limit: z.number().int().min(1).max(100).optional(),
        after: z.string().optional(),
        title: z.string().min(1).max(300).optional(),
        content_type: z.enum(["pdf", "raw"]).optional(),
        upload_id: z.string().min(1).optional(),
        uri: z.url().optional(),
        options: recordSchema.optional(),
        deliveries: z.number().int().min(1).max(100).optional(),
        expire_after_seconds: z.number().int().positive().optional(),
        metadata: z.record(z.string(), z.string()).optional(),
        idempotency_key: z.string().min(8).max(255).optional(),
        confirm_destination: z.string().optional(),
        fixture: z.string().min(3).max(300).optional(),
        confirm: z.string().optional(),
        ...selectionShape,
      }),
      annotations: mutationAnnotations("Inspect and operate Piqae jobs", true),
    },
    (input, extra) =>
      result(async () => {
        const client = clientFor(config, extra, input);
        if (input.action === "list") {
          return client.jobs.list(
            withoutUndefined({
              state: input.state,
              printer_id: input.printer_id,
              target_id: input.target_id,
              metadata_key: input.metadata_key,
              metadata_value: input.metadata_value,
              limit: input.limit,
              after: input.after,
            }) as JobListOptions,
          );
        }
        if (input.action === "create") {
          enforceJobPolicy(config, bearer(config, extra));
          const destination = input.printer_id ?? input.target_id;
          if (!destination || (input.printer_id && input.target_id)) {
            throw new Error(
              "Exactly one of printer_id or target_id is required.",
            );
          }
          requireConfirmation(destination, input.confirm_destination);
          required(input.fixture, "fixture");
          if ((input.upload_id === undefined) === (input.uri === undefined)) {
            throw new Error("Exactly one of upload_id or uri is required.");
          }
          const content = input.upload_id
            ? { type: "upload" as const, upload_id: input.upload_id }
            : { type: "uri" as const, uri: required(input.uri, "uri") };
          const job = withoutUndefined({
            ...(input.printer_id
              ? { printer_id: input.printer_id }
              : { target_id: input.target_id }),
            title: required(input.title, "title"),
            content_type: required(input.content_type, "content_type"),
            content,
            options: input.options as JobOptions | undefined,
            deliveries: input.deliveries,
            expire_after_seconds: input.expire_after_seconds,
            metadata: input.metadata,
          }) as CreateJob;
          return client.jobs.create(
            job,
            required(input.idempotency_key, "idempotency_key"),
          );
        }
        const jobId = required(input.job_id, "job_id");
        if (input.action === "get") return client.jobs.retrieve(jobId);
        if (input.action === "events") return client.jobs.events(jobId);
        requireConfirmation(jobId, input.confirm);
        return client.jobs.cancel(jobId);
      }),
  );
}

function registerWebhookTool(server: McpServer, config: McpConfig): void {
  server.registerTool(
    "piqae_webhooks",
    {
      title: "Manage Piqae webhooks",
      description:
        "List/create/remove signed webhook endpoints, inspect delivery attempts, or replay a delivery. Creation stores the one-time signing secret out of band by default.",
      inputSchema: z.object({
        action: z.enum(["list", "create", "remove", "deliveries", "replay"]),
        webhook_id: z.string().min(1).optional(),
        delivery_id: z.string().min(1).optional(),
        url: z.url().optional(),
        events: z.array(z.string().min(1)).min(1).optional(),
        delivery: deliverySchema,
        confirm: z.string().optional(),
        ...selectionShape,
      }),
      annotations: mutationAnnotations("Manage Piqae webhooks", true),
    },
    (input, extra) =>
      result(async () => {
        const client = clientFor(config, extra, input);
        if (input.action === "list") return client.webhooks.list();
        if (input.action === "create") {
          await assertSecretDeliveryReady(config, input.delivery);
          const webhook = await client.webhooks.create({
            url: required(input.url, "url"),
            events: required(input.events, "events"),
          });
          const delivered = await deliverSecret(
            config,
            "webhook",
            webhook.id,
            webhook.secret,
            input.delivery,
          );
          const { secret: _secret, ...metadata } = webhook;
          return { ...metadata, secret_delivery: delivered };
        }
        if (input.action === "replay") {
          return client.webhooks.replay(
            required(input.delivery_id, "delivery_id"),
          );
        }
        const webhookId = required(input.webhook_id, "webhook_id");
        if (input.action === "deliveries")
          return client.webhooks.deliveries(webhookId);
        requireConfirmation(webhookId, input.confirm);
        return client.webhooks.remove(webhookId);
      }),
  );
}

function registerPlatformTool(server: McpServer, config: McpConfig): void {
  server.registerTool(
    "piqae_platform_accounts",
    {
      title: "Manage Piqae integrator accounts",
      description:
        "Enable the current workspace platform integration, or list/get/idempotently upsert/archive isolated customer accounts. Works with an owner session or platform service-account bearer as permitted by the API.",
      inputSchema: z.object({
        action: z.enum(["enable", "list", "get", "upsert", "archive"]),
        external_id: z
          .string()
          .regex(/^[A-Za-z0-9][A-Za-z0-9_.:-]{0,119}$/)
          .optional(),
        name: z.string().min(1).max(120).optional(),
        metadata: z.record(z.string(), z.string().max(500)).optional(),
        delivery: deliverySchema,
        confirm: z.string().optional(),
      }),
      annotations: mutationAnnotations(
        "Manage Piqae integrator accounts",
        true,
      ),
    },
    (input, extra) =>
      result(async () => {
        if (input.action === "enable") {
          await assertSecretDeliveryReady(config, input.delivery);
          const enabled = await rawRequest<{ enabled: true; secret: string }>(
            config,
            extra,
            "POST",
            "/v1/platform/enable",
          );
          if (
            typeof enabled?.secret !== "string" ||
            enabled.secret.length === 0
          ) {
            throw new Error(
              "Platform enable response did not include a secret.",
            );
          }
          const delivered = await deliverSecret(
            config,
            "platform",
            `workspace-platform-integration-${randomUUID()}`,
            enabled.secret,
            input.delivery,
          );
          const { secret: _secret, ...metadata } = enabled;
          return { ...metadata, secret_delivery: delivered };
        }
        if (input.action === "list") {
          return rawRequest<PlatformAccount[]>(
            config,
            extra,
            "GET",
            "/v1/platform/accounts",
          );
        }
        const externalId = required(input.external_id, "external_id");
        const path = `/v1/platform/accounts/${encodeURIComponent(externalId)}`;
        if (input.action === "get")
          return rawRequest(config, extra, "GET", path);
        if (input.action === "upsert") {
          return rawRequest(config, extra, "PUT", path, {
            name: required(input.name, "name"),
            ...(input.metadata ? { metadata: input.metadata } : {}),
          });
        }
        requireConfirmation(externalId, input.confirm);
        await rawRequest(config, extra, "DELETE", path);
        return { archived: true, external_id: externalId };
      }),
  );
}

function registerDocumentationTool(server: McpServer): void {
  server.registerTool(
    "piqae_search_docs",
    {
      title: "Search Piqae documentation",
      description:
        "Search checked-in Piqae API, operations, node, printing, SDK, and OpenAPI documentation. Returns bounded excerpts and repository-relative paths.",
      inputSchema: z.object({
        query: z.string().min(2).max(160),
        limit: z.number().int().min(1).max(20).default(8),
      }),
      annotations: readOnlyAnnotations("Search Piqae documentation"),
    },
    (input) => result(() => searchDocumentation(input.query, input.limit)),
  );
}

function registerResources(server: McpServer): void {
  for (const resource of KNOWLEDGE_RESOURCES) {
    server.registerResource(
      resource.name,
      resource.uri,
      {
        title: resource.title,
        description: resource.description,
        mimeType: resource.mimeType,
      },
      async () => ({
        contents: [
          {
            uri: resource.uri,
            mimeType: resource.mimeType,
            text: (await readKnowledgeFile(resource.path)) ?? resource.fallback,
          },
        ],
      }),
    );
  }
}

function registerPrompts(server: McpServer): void {
  server.registerPrompt(
    "piqae_operator",
    {
      title: "Operate Piqae safely",
      description:
        "Establish context and perform a Piqae administration or integration task safely.",
      argsSchema: {
        task: z.string().min(1).max(1000),
        environment: z.enum(["test", "live", "unknown"]).default("unknown"),
      },
    },
    ({ task, environment }) => ({
      messages: [
        {
          role: "user",
          content: {
            type: "text",
            text: `You are operating Piqae (${environment} environment). Read piqae://guide/operator and piqae://guide/security, then inspect piqae_context before acting. Complete this task: ${task}. Use least privilege, preserve tenant boundaries, use stable idempotency keys, never treat durable registration or spooler acceptance as physical delivery, and ask for explicit destination/fixture authorization before any print-producing action.`,
          },
        },
      ],
    }),
  );
}

function clientFor(
  config: McpConfig,
  extra: ToolExtra,
  input: {
    workspace_id?: string | undefined;
    environment_id?: string | undefined;
  },
): PiqaeClient {
  return tenantClient(config, extra, selection(input));
}

function selection(input: {
  workspace_id?: string | undefined;
  environment_id?: string | undefined;
}): { workspaceId: string; environmentId: string } | undefined {
  if (
    (input.workspace_id === undefined) !==
    (input.environment_id === undefined)
  ) {
    throw new Error(
      "workspace_id and environment_id must be provided together.",
    );
  }
  return input.workspace_id && input.environment_id
    ? { workspaceId: input.workspace_id, environmentId: input.environment_id }
    : undefined;
}

function enforceJobPolicy(config: McpConfig, token: string): void {
  if (config.jobSubmission === "disabled") {
    throw new Error("Job submission is disabled by PIQAE_MCP_JOB_SUBMISSION.");
  }
  if (
    config.jobSubmission === "test_only" &&
    !token.startsWith("piq_test_") &&
    !token.startsWith("spl_test_")
  ) {
    throw new Error(
      "test_only job submission accepts only a Test API key, whose environment is cryptographically evident from its prefix. Use a test-scoped key or explicitly configure policy=all.",
    );
  }
}

async function result(operation: () => Promise<unknown> | unknown) {
  try {
    const value = (await operation()) ?? { ok: true };
    const structuredContent = isRecord(value) ? value : { result: value };
    return {
      content: [
        { type: "text" as const, text: JSON.stringify(value, null, 2) },
      ],
      structuredContent,
    };
  } catch (error) {
    const safe = safeError(error);
    return {
      isError: true,
      content: [{ type: "text" as const, text: JSON.stringify(safe, null, 2) }],
      structuredContent: safe,
    };
  }
}

function safeError(error: unknown): Record<string, unknown> {
  let payload: Record<string, unknown>;
  if (error instanceof PiqaeError) {
    payload = {
      error: {
        code: error.code,
        message: error.message,
        status: error.status,
        request_id: error.requestId,
        retryable: error.retryable,
        details: error.details,
      },
    };
  } else {
    const message =
      error instanceof Error
        ? error.message
        : "Unexpected MCP operation failure";
    payload = {
      error: {
        code: "mcp_operation_failed",
        message,
        retryable: false,
      },
    };
  }
  return redactValue(payload) as Record<string, unknown>;
}

function redact(value: string): string {
  return value.replace(
    /(?:(?:piq|spl)_[A-Za-z0-9]+_|whsec_)[A-Za-z0-9._-]{8,}/g,
    "[REDACTED_PIQAE_SECRET]",
  );
}

function redactValue(value: unknown, key?: string): unknown {
  if (
    key &&
    /(?:secret|token|password|authorization|credential|private[_-]?key)/i.test(
      key,
    ) &&
    value !== null
  ) {
    return "[REDACTED_PIQAE_SECRET]";
  }
  if (typeof value === "string") return redact(value);
  if (Array.isArray(value)) return value.map((item) => redactValue(item));
  if (isRecord(value)) {
    return Object.fromEntries(
      Object.entries(value).map(([childKey, child]) => [
        childKey,
        redactValue(child, childKey),
      ]),
    );
  }
  return value;
}

function readOnlyAnnotations(title: string) {
  return {
    title,
    readOnlyHint: true,
    destructiveHint: false,
    idempotentHint: true,
    openWorldHint: true,
  };
}

function mutationAnnotations(title: string, destructive: boolean) {
  return {
    title,
    readOnlyHint: false,
    destructiveHint: destructive,
    idempotentHint: false,
    openWorldHint: true,
  };
}

function required<T>(value: T | undefined | null, name: string): T {
  if (value === undefined || value === null)
    throw new Error(`${name} is required for this action.`);
  return value;
}

function requireConfirmation(
  identifier: string,
  confirmation: string | undefined,
): void {
  if (confirmation !== identifier) {
    throw new Error(`confirm must exactly equal ${identifier}.`);
  }
}

function withoutUndefined<T extends Record<string, unknown>>(
  value: T,
): Partial<T> {
  return Object.fromEntries(
    Object.entries(value).filter(([, item]) => item !== undefined),
  ) as Partial<T>;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function unreachable(value: never): never {
  throw new Error(`Unsupported action: ${String(value)}`);
}

async function searchDocumentation(
  query: string,
  limit: number,
): Promise<Record<string, unknown>> {
  const root = await findRepositoryRoot();
  if (!root) {
    return {
      query,
      matches: [],
      note: "The published MCP knowledge resources are available, but repository-wide search requires running from a Piqae checkout or setting PIQAE_MCP_KNOWLEDGE_ROOT.",
    };
  }
  const candidates = [
    join(root, "docs"),
    join(root, "sdk", "typescript", "README.md"),
  ];
  const files: string[] = [];
  for (const candidate of candidates)
    files.push(...(await markdownFiles(candidate)));
  const words = query.toLocaleLowerCase().split(/\s+/).filter(Boolean);
  const matches: Array<{
    path: string;
    line: number;
    excerpt: string;
    score: number;
  }> = [];
  for (const file of files.slice(0, 500)) {
    const metadata = await stat(file).catch(() => undefined);
    if (!metadata?.isFile() || metadata.size > 2 * 1024 * 1024) continue;
    const text = await readFile(file, "utf8").catch(() => "");
    const lines = text.split("\n");
    for (let index = 0; index < lines.length; index += 1) {
      const line = lines[index] ?? "";
      const lower = line.toLocaleLowerCase();
      const score = words.reduce(
        (total, word) => total + (lower.includes(word) ? 1 : 0),
        0,
      );
      if (score === 0) continue;
      matches.push({
        path: relative(root, file),
        line: index + 1,
        excerpt: line.trim().slice(0, 500),
        score,
      });
      matches.sort(
        (left, right) =>
          right.score - left.score || left.path.localeCompare(right.path),
      );
      if (matches.length > limit) matches.length = limit;
    }
  }
  return { query, matches };
}

async function markdownFiles(path: string): Promise<string[]> {
  const stat = await readdir(path, { withFileTypes: true }).catch(
    () => undefined,
  );
  if (!stat) return path.endsWith(".md") ? [path] : [];
  const files: string[] = [];
  for (const entry of stat) {
    const child = join(path, entry.name);
    if (entry.isDirectory()) files.push(...(await markdownFiles(child)));
    else if (entry.isFile() && entry.name.endsWith(".md")) files.push(child);
  }
  return files;
}

async function readKnowledgeFile(path: string): Promise<string | undefined> {
  const root = await findRepositoryRoot();
  if (root) return readFile(join(root, path), "utf8").catch(() => undefined);
  const packageRoot = resolve(
    dirname(fileURLToPath(import.meta.url)),
    "knowledge",
  );
  return readFile(join(packageRoot, path.replaceAll("/", "__")), "utf8").catch(
    () => undefined,
  );
}

async function findRepositoryRoot(): Promise<string | undefined> {
  const configured = process.env.PIQAE_MCP_KNOWLEDGE_ROOT;
  let current = resolve(configured ?? process.cwd());
  for (let depth = 0; depth < 8; depth += 1) {
    const marker = join(current, "contracts", "openapi", "piqae-v1.yaml");
    if (
      await readFile(marker)
        .then(() => true)
        .catch(() => false)
    )
      return current;
    const parent = dirname(current);
    if (parent === current) break;
    current = parent;
  }
  return undefined;
}

const SERVER_INSTRUCTIONS = `Piqae is local-first printing infrastructure. The durable node owns identity, queueing, recovery, and cloud synchronization. Installed OS drivers remain authoritative for vendor options. Read context before mutation. Never infer a tenant or silently fall back from a pinned profile. A registered or spooler-accepted job is not proof of physical delivery; preserve accepted, printing, reported-complete, and delivery-uncertain states. Use least-privilege credentials and explicit confirmations for destructive or print-producing actions.`;

const OPERATOR_FALLBACK = `# Piqae operator model\n\n${SERVER_INSTRUCTIONS}\n\nUse piqae_context first. Nodes are durable agents; shells are disposable. Prefer targets with pinned profile revisions for routing. Use uploads for private PDFs, stable idempotency keys for jobs, signed webhooks for events, and Test environments for onboarding. Platform credentials require exact workspace/environment context and may never accept tenant selectors from untrusted input.`;

const SECURITY_FALLBACK = `# Piqae MCP security\n\nRemote Streamable HTTP uses an OAuth bearer and RFC 9728 protected-resource metadata when configured. Stdio reads one server-side credential from environment. Piqae remains the authorization authority and enforces workspace, environment, and scope boundaries. One-time API-key, webhook, enrolment, and platform secrets are written to PIQAE_MCP_SECRET_DIRECTORY with owner-only permissions by default. Transcript output requires PIQAE_MCP_ALLOW_SECRET_OUTPUT=true plus per-call delivery=response. Job submission defaults disabled.`;

const KNOWLEDGE_RESOURCES = [
  {
    name: "piqae-operator-guide",
    uri: "piqae://guide/operator",
    title: "Piqae operator guide",
    description:
      "Product invariants, lifecycle semantics, and operating model.",
    mimeType: "text/markdown",
    path: "docs/operations/reliability-and-job-lifecycle.md",
    fallback: OPERATOR_FALLBACK,
  },
  {
    name: "piqae-security-guide",
    uri: "piqae://guide/security",
    title: "Piqae authentication and security",
    description:
      "Credential types, tenant isolation, scopes, and secret-handling guidance.",
    mimeType: "text/markdown",
    path: "docs/api/authentication.md",
    fallback: SECURITY_FALLBACK,
  },
  {
    name: "piqae-api-guide",
    uri: "piqae://guide/api",
    title: "Piqae API guide",
    description: "Index of the native API and integration guides.",
    mimeType: "text/markdown",
    path: "docs/api/README.md",
    fallback: OPERATOR_FALLBACK,
  },
  {
    name: "piqae-typescript-sdk",
    uri: "piqae://sdk/typescript",
    title: "Piqae TypeScript SDK",
    description: "Typed SDK setup and examples.",
    mimeType: "text/markdown",
    path: "sdk/typescript/README.md",
    fallback: OPERATOR_FALLBACK,
  },
  {
    name: "piqae-openapi",
    uri: "piqae://openapi/v1",
    title: "Piqae OpenAPI V1",
    description:
      "Authoritative machine-readable request and response contract.",
    mimeType: "application/yaml",
    path: "contracts/openapi/piqae-v1.yaml",
    fallback:
      "# OpenAPI contract is included when built or run from the Piqae repository.",
  },
] as const;
