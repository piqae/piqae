import { randomUUID } from "node:crypto";
import pg, { type Pool, type PoolClient } from "pg";
import { normalizeShopDomain } from "./model";
import { parseTemplateEnvelope, type PrintPacket } from "./template-model";
import { resolveShopifyStorage } from "./piqae-runtime.server";
import {
  DEFAULT_PRINT_ORDER_SETTINGS,
  parsePrintOrderSettings,
  type PrintOrderSettings,
} from "./print-order";

export type MerchantSettings = {
  defaultPrinterId: string;
  defaultTemplateId: string;
  preferDirect: boolean;
  offerPdf: boolean;
  metafieldAllowlist: string[];
  retentionDays: number;
  renderExecutionPolicy: RenderExecutionPolicy;
  printOrder: PrintOrderSettings;
};
export type RenderExecutionPolicy =
  | "automatic"
  | "cloud_only"
  | "prefer_node"
  | "require_node";
export type MerchantTemplate = {
  id: string;
  name: string;
  kind: string;
  pageSize: string;
  state: "draft" | "published";
  source: string;
  revision: number;
  draftRevision: number;
  designTargetId?: string | null;
  designSpecificationRevision?: string | null;
  published: PublishedTemplateRevision | null;
  updatedAt: string;
};
export type PublishedTemplateRevision = {
  revision: number;
  name: string;
  kind: string;
  pageSize: string;
  source: string;
  designTargetId: string | null;
  designSpecificationRevision: string | null;
  media: PrintPacket["media"];
};
export type SaveMerchantTemplate = Omit<
  MerchantTemplate,
  "updatedAt" | "draftRevision" | "published"
> & { expectedDraftRevision?: number | null };

export class WorkflowConflictError extends Error {
  constructor() {
    super(
      "This document changed in another session. Reload it before saving again.",
    );
    this.name = "WorkflowConflictError";
  }
}
export type AutomationRule = {
  id: string;
  name: string;
  trigger:
    | "order_paid"
    | "order_created"
    | "fulfillment_created"
    | "refund_created";
  delivery: "printer" | "email";
  templateId: string;
  destination: string;
  enabled: boolean;
  updatedAt: string;
};
export type ActivityEntry = {
  id: string;
  orderName: string;
  documentName: string;
  destination: string;
  state: "accepted" | "printing" | "reported_complete" | "uncertain" | "failed";
  createdAt: string;
};
export type BillingState = {
  mode: "existing_piqae" | "shopify_child";
  plan: "free" | "starter" | "growth" | "scale";
  used: number;
  limit: number;
  status: "active" | "approval_required";
};

const DEFAULT_SETTINGS: MerchantSettings = {
  defaultPrinterId: "",
  defaultTemplateId: "",
  preferDirect: true,
  offerPdf: true,
  metafieldAllowlist: [],
  retentionDays: 30,
  renderExecutionPolicy: "automatic",
  printOrder: DEFAULT_PRINT_ORDER_SETTINGS,
};
const DEFAULT_BILLING: BillingState = {
  mode: "existing_piqae",
  plan: "free",
  used: 0,
  limit: 50,
  status: "active",
};

export interface WorkflowRepository {
  getSettings(shop: string): Promise<MerchantSettings>;
  saveSettings(shop: string, settings: MerchantSettings): Promise<void>;
  updateSettings(
    shop: string,
    settings: Partial<MerchantSettings>,
  ): Promise<void>;
  listTemplates(shop: string): Promise<MerchantTemplate[]>;
  getTemplate(shop: string, id: string): Promise<MerchantTemplate | null>;
  saveTemplate(
    shop: string,
    value: SaveMerchantTemplate,
  ): Promise<MerchantTemplate>;
  saveTemplatesAtomically(
    shop: string,
    values: SaveMerchantTemplate[],
  ): Promise<MerchantTemplate[]>;
  deleteTemplate(shop: string, id: string): Promise<boolean>;
  listAutomations(shop: string): Promise<AutomationRule[]>;
  saveAutomation(
    shop: string,
    value: Omit<AutomationRule, "updatedAt">,
  ): Promise<AutomationRule>;
  deleteAutomation(shop: string, id: string): Promise<boolean>;
  listActivity(
    shop: string,
    query?: string,
    state?: string,
  ): Promise<ActivityEntry[]>;
  recordActivity(
    shop: string,
    value: Omit<ActivityEntry, "createdAt">,
  ): Promise<void>;
  getBilling(shop: string): Promise<BillingState>;
  saveBilling(shop: string, value: BillingState): Promise<void>;
  recordUsage(
    shop: string,
    eventKey: string,
    documents: number,
  ): Promise<boolean>;
}

export class MemoryWorkflowRepository implements WorkflowRepository {
  private settings = new Map<string, MerchantSettings>();
  private templates = new Map<string, MerchantTemplate[]>();
  private automations = new Map<string, AutomationRule[]>();
  private activity = new Map<string, ActivityEntry[]>();
  private billing = new Map<string, BillingState>();
  private usage = new Map<
    string,
    Map<string, { documents: number; createdAt: Date }>
  >();
  async getSettings(shop: string) {
    return structuredClone(
      this.settings.get(normalizeShopDomain(shop)) ?? DEFAULT_SETTINGS,
    );
  }
  async saveSettings(shop: string, value: MerchantSettings) {
    this.settings.set(normalizeShopDomain(shop), structuredClone(value));
  }
  async updateSettings(shop: string, value: Partial<MerchantSettings>) {
    const key = normalizeShopDomain(shop);
    const current = this.settings.get(key) ?? DEFAULT_SETTINGS;
    this.settings.set(key, {
      ...structuredClone(current),
      ...structuredClone(value),
    });
  }
  async listTemplates(shop: string) {
    return structuredClone(this.templates.get(normalizeShopDomain(shop)) ?? []);
  }
  async getTemplate(shop: string, id: string) {
    return (
      (await this.listTemplates(shop)).find((value) => value.id === id) ?? null
    );
  }
  async saveTemplate(shop: string, value: SaveMerchantTemplate) {
    return this.saveTemplateNow(shop, value);
  }
  async saveTemplatesAtomically(shop: string, values: SaveMerchantTemplate[]) {
    const key = normalizeShopDomain(shop);
    const before = structuredClone(this.templates.get(key) ?? []);
    try {
      return values.map((value) => this.saveTemplateNow(shop, value));
    } catch (error) {
      this.templates.set(key, before);
      throw error;
    }
  }
  private saveTemplateNow(shop: string, value: SaveMerchantTemplate) {
    const key = normalizeShopDomain(shop);
    const all = this.templates.get(key) ?? [];
    const previous = all.find((item) => item.id === value.id);
    if (
      (previous && value.expectedDraftRevision !== previous.draftRevision) ||
      (!previous && value.expectedDraftRevision != null)
    )
      throw new WorkflowConflictError();
    const draftRevision = previous ? previous.draftRevision + 1 : 1;
    const publishedRevision =
      value.state === "published"
        ? {
            revision: (previous?.published?.revision ?? 0) + 1,
            name: value.name,
            kind: value.kind,
            pageSize: value.pageSize,
            source: value.source,
            designTargetId: value.designTargetId ?? null,
            designSpecificationRevision:
              value.designSpecificationRevision ?? null,
            media: structuredClone(
              parseTemplateEnvelope(value.source).document.media,
            ),
          }
        : (previous?.published ?? null);
    const saved: MerchantTemplate = {
      id: value.id,
      name: value.name,
      kind: value.kind,
      pageSize: value.pageSize,
      state: publishedRevision ? "published" : "draft",
      source: value.source,
      revision: publishedRevision?.revision ?? draftRevision,
      draftRevision,
      designTargetId: value.designTargetId ?? null,
      designSpecificationRevision: value.designSpecificationRevision ?? null,
      published: publishedRevision,
      updatedAt: new Date().toISOString(),
    };
    this.templates.set(key, [
      ...all.filter((item) => item.id !== value.id),
      saved,
    ]);
    return structuredClone(saved);
  }
  async deleteTemplate(shop: string, id: string) {
    const key = normalizeShopDomain(shop);
    const all = this.templates.get(key) ?? [];
    if (all.find((value) => value.id === id)?.published) return false;
    this.templates.set(
      key,
      all.filter((value) => value.id !== id),
    );
    return all.length !== (this.templates.get(key)?.length ?? 0);
  }
  async listAutomations(shop: string) {
    return structuredClone(
      this.automations.get(normalizeShopDomain(shop)) ?? [],
    );
  }
  async saveAutomation(shop: string, value: Omit<AutomationRule, "updatedAt">) {
    const key = normalizeShopDomain(shop);
    const saved = { ...value, updatedAt: new Date().toISOString() };
    const all = this.automations.get(key) ?? [];
    this.automations.set(key, [
      ...all.filter((item) => item.id !== value.id),
      saved,
    ]);
    return structuredClone(saved);
  }
  async deleteAutomation(shop: string, id: string) {
    const key = normalizeShopDomain(shop);
    const all = this.automations.get(key) ?? [];
    this.automations.set(
      key,
      all.filter((value) => value.id !== id),
    );
    return all.length !== (this.automations.get(key)?.length ?? 0);
  }
  async listActivity(shop: string, query = "", state = "") {
    const term = query.toLowerCase();
    return structuredClone(
      (this.activity.get(normalizeShopDomain(shop)) ?? [])
        .filter(
          (value) =>
            (!term ||
              `${value.orderName} ${value.documentName}`
                .toLowerCase()
                .includes(term)) &&
            (!state || value.state === state),
        )
        .slice(0, 100),
    );
  }
  async recordActivity(shop: string, value: Omit<ActivityEntry, "createdAt">) {
    const key = normalizeShopDomain(shop);
    const entries = this.activity.get(key) ?? [];
    this.activity.set(
      key,
      [{ ...value, createdAt: new Date().toISOString() }, ...entries].slice(
        0,
        100,
      ),
    );
  }
  async getBilling(shop: string) {
    const key = normalizeShopDomain(shop);
    const billing = structuredClone(this.billing.get(key) ?? DEFAULT_BILLING);
    const start = new Date();
    start.setUTCDate(1);
    start.setUTCHours(0, 0, 0, 0);
    billing.used = [...(this.usage.get(key)?.values() ?? [])]
      .filter((item) => item.createdAt >= start)
      .reduce((sum, item) => sum + item.documents, 0);
    return billing;
  }
  async saveBilling(shop: string, value: BillingState) {
    this.billing.set(normalizeShopDomain(shop), structuredClone(value));
  }
  async recordUsage(shop: string, eventKey: string, documents: number) {
    if (
      !eventKey ||
      eventKey.length > 200 ||
      !Number.isInteger(documents) ||
      documents < 1 ||
      documents > 10_000
    )
      throw new Error("Document usage event is invalid");
    const key = normalizeShopDomain(shop);
    const events = this.usage.get(key) ?? new Map();
    if (events.has(eventKey)) return false;
    events.set(eventKey, { documents, createdAt: new Date() });
    this.usage.set(key, events);
    return true;
  }
}

export class PostgresWorkflowRepository implements WorkflowRepository {
  constructor(private pool: Pool) {}
  async getSettings(shop: string) {
    const result = await this.pool.query(
      "SELECT default_printer_id,default_template_id,prefer_direct,offer_pdf,metafield_allowlist,retention_days,render_execution_policy,print_order FROM shopify_merchant_settings WHERE shop=$1",
      [normalizeShopDomain(shop)],
    );
    const r = result.rows[0];
    return r
      ? {
          defaultPrinterId: r.default_printer_id ?? "",
          defaultTemplateId: r.default_template_id ?? "",
          preferDirect: r.prefer_direct,
          offerPdf: r.offer_pdf,
          metafieldAllowlist: r.metafield_allowlist ?? [],
          retentionDays: r.retention_days,
          renderExecutionPolicy: parseRenderExecutionPolicy(
            r.render_execution_policy,
          ),
          printOrder: parsePrintOrderSettings(r.print_order),
        }
      : structuredClone(DEFAULT_SETTINGS);
  }
  async saveSettings(shop: string, v: MerchantSettings) {
    await this.pool.query(
      "INSERT INTO shopify_merchant_settings(shop,default_printer_id,default_template_id,prefer_direct,offer_pdf,metafield_allowlist,retention_days,render_execution_policy,print_order) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9) ON CONFLICT(shop) DO UPDATE SET default_printer_id=EXCLUDED.default_printer_id,default_template_id=EXCLUDED.default_template_id,prefer_direct=EXCLUDED.prefer_direct,offer_pdf=EXCLUDED.offer_pdf,metafield_allowlist=EXCLUDED.metafield_allowlist,retention_days=EXCLUDED.retention_days,render_execution_policy=EXCLUDED.render_execution_policy,print_order=EXCLUDED.print_order,updated_at=now()",
      [
        normalizeShopDomain(shop),
        v.defaultPrinterId || null,
        v.defaultTemplateId || null,
        v.preferDirect,
        v.offerPdf,
        v.metafieldAllowlist,
        v.retentionDays,
        v.renderExecutionPolicy,
        JSON.stringify(v.printOrder),
      ],
    );
  }
  async updateSettings(shop: string, value: Partial<MerchantSettings>) {
    const candidates: Array<[keyof MerchantSettings, string, unknown]> = [
      [
        "defaultPrinterId",
        "default_printer_id",
        value.defaultPrinterId || null,
      ],
      [
        "defaultTemplateId",
        "default_template_id",
        value.defaultTemplateId || null,
      ],
      ["preferDirect", "prefer_direct", value.preferDirect],
      ["offerPdf", "offer_pdf", value.offerPdf],
      ["metafieldAllowlist", "metafield_allowlist", value.metafieldAllowlist],
      ["retentionDays", "retention_days", value.retentionDays],
      [
        "renderExecutionPolicy",
        "render_execution_policy",
        value.renderExecutionPolicy,
      ],
      [
        "printOrder",
        "print_order",
        value.printOrder === undefined
          ? undefined
          : JSON.stringify(value.printOrder),
      ],
    ];
    const columns = candidates.filter(([key]) => value[key] !== undefined);
    if (!columns.length) return;
    const normalizedShop = normalizeShopDomain(shop);
    const client = await this.pool.connect();
    try {
      await client.query("BEGIN");
      await client.query(
        "INSERT INTO shopify_merchant_settings(shop) VALUES($1) ON CONFLICT(shop) DO NOTHING",
        [normalizedShop],
      );
      const assignments = columns.map(
        ([, column], index) => `${column}=$${index + 2}`,
      );
      await client.query(
        `UPDATE shopify_merchant_settings SET ${assignments.join(",")},updated_at=now() WHERE shop=$1`,
        [normalizedShop, ...columns.map(([, , setting]) => setting)],
      );
      await client.query("COMMIT");
    } catch (error) {
      await client.query("ROLLBACK");
      throw error;
    } finally {
      client.release();
    }
  }
  async listTemplates(shop: string) {
    const result = await this.pool.query(
      `${TEMPLATE_SELECT} WHERE t.shop=$1 ORDER BY t.updated_at DESC LIMIT 100`,
      [normalizeShopDomain(shop)],
    );
    return result.rows.map(templateRow);
  }
  async getTemplate(shop: string, id: string) {
    const result = await this.pool.query(
      `${TEMPLATE_SELECT} WHERE t.shop=$1 AND t.id=$2`,
      [normalizeShopDomain(shop), id],
    );
    return result.rows[0] ? templateRow(result.rows[0]) : null;
  }
  async saveTemplate(shop: string, v: SaveMerchantTemplate) {
    const client = await this.pool.connect();
    const normalizedShop = normalizeShopDomain(shop);
    try {
      await client.query("BEGIN");
      const saved = await this.saveTemplateWithClient(
        client,
        normalizedShop,
        v,
      );
      await client.query("COMMIT");
      return saved;
    } catch (error) {
      await client.query("ROLLBACK");
      throw error;
    } finally {
      client.release();
    }
  }
  async saveTemplatesAtomically(shop: string, values: SaveMerchantTemplate[]) {
    const client = await this.pool.connect();
    const normalizedShop = normalizeShopDomain(shop);
    try {
      await client.query("BEGIN");
      const saved: MerchantTemplate[] = [];
      for (const value of [...values].sort((left, right) =>
        left.id.localeCompare(right.id),
      ))
        saved.push(
          await this.saveTemplateWithClient(client, normalizedShop, value),
        );
      await client.query("COMMIT");
      return saved;
    } catch (error) {
      await client.query("ROLLBACK");
      throw error;
    } finally {
      client.release();
    }
  }
  private async saveTemplateWithClient(
    client: PoolClient,
    normalizedShop: string,
    v: SaveMerchantTemplate,
  ): Promise<MerchantTemplate> {
    const current = await client.query(
      "SELECT draft_revision,published_revision FROM shopify_workflow_templates WHERE shop=$1 AND id=$2 FOR UPDATE",
      [normalizedShop, v.id],
    );
    const existing = current.rows[0];
    if (
      (existing &&
        v.expectedDraftRevision !== Number(existing.draft_revision)) ||
      (!existing && v.expectedDraftRevision != null)
    )
      throw new WorkflowConflictError();
    const draftRevision = existing ? Number(existing.draft_revision) + 1 : 1;
    const draftWrite = await client.query(
      "INSERT INTO shopify_workflow_templates(id,shop,name,kind,page_size,draft_source,draft_revision,design_target_id,design_specification_revision,state,source,revision) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12) ON CONFLICT(id,shop) DO UPDATE SET name=EXCLUDED.name,kind=EXCLUDED.kind,page_size=EXCLUDED.page_size,draft_source=EXCLUDED.draft_source,draft_revision=EXCLUDED.draft_revision,design_target_id=EXCLUDED.design_target_id,design_specification_revision=EXCLUDED.design_specification_revision,updated_at=now() WHERE $13::integer IS NOT NULL AND shopify_workflow_templates.draft_revision=$13 RETURNING draft_revision",
      [
        v.id,
        normalizedShop,
        v.name,
        v.kind,
        v.pageSize,
        v.source,
        draftRevision,
        v.designTargetId ?? null,
        v.designSpecificationRevision ?? null,
        v.state,
        v.source,
        Math.max(1, v.revision),
        v.expectedDraftRevision ?? null,
      ],
    );
    if (draftWrite.rowCount !== 1) throw new WorkflowConflictError();
    if (v.state === "published") {
      const highest = await client.query(
        "SELECT COALESCE(MAX(revision),0) AS revision FROM shopify_workflow_template_revisions WHERE template_id=$1 AND shop=$2",
        [v.id, normalizedShop],
      );
      const revision = Number(highest.rows[0]?.revision ?? 0) + 1;
      const media = parseTemplateEnvelope(v.source).document.media;
      await client.query(
        "INSERT INTO shopify_workflow_template_revisions(template_id,shop,revision,name,kind,page_size,source,design_target_id,design_specification_revision,media) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
        [
          v.id,
          normalizedShop,
          revision,
          v.name,
          v.kind,
          v.pageSize,
          v.source,
          v.designTargetId ?? null,
          v.designSpecificationRevision ?? null,
          JSON.stringify(media),
        ],
      );
      await client.query(
        "UPDATE shopify_workflow_templates SET published_revision=$3 WHERE shop=$1 AND id=$2",
        [normalizedShop, v.id, revision],
      );
    }
    const result = await client.query(
      `${TEMPLATE_SELECT} WHERE t.shop=$1 AND t.id=$2`,
      [normalizedShop, v.id],
    );
    return templateRow(result.rows[0]);
  }
  async deleteTemplate(shop: string, id: string) {
    const r = await this.pool.query(
      "DELETE FROM shopify_workflow_templates WHERE shop=$1 AND id=$2 AND published_revision IS NULL",
      [normalizeShopDomain(shop), id],
    );
    return r.rowCount === 1;
  }
  async listAutomations(shop: string) {
    const r = await this.pool.query(
      "SELECT id,name,trigger_event,delivery,template_id,destination,enabled,updated_at FROM shopify_automation_rules WHERE shop=$1 ORDER BY updated_at DESC LIMIT 100",
      [normalizeShopDomain(shop)],
    );
    return r.rows.map(automationRow);
  }
  async saveAutomation(shop: string, v: Omit<AutomationRule, "updatedAt">) {
    const r = await this.pool.query(
      "INSERT INTO shopify_automation_rules(id,shop,name,trigger_event,delivery,template_id,destination,enabled) VALUES($1,$2,$3,$4,$5,$6,$7,$8) ON CONFLICT(id,shop) DO UPDATE SET name=EXCLUDED.name,trigger_event=EXCLUDED.trigger_event,delivery=EXCLUDED.delivery,template_id=EXCLUDED.template_id,destination=EXCLUDED.destination,enabled=EXCLUDED.enabled,updated_at=now() RETURNING id,name,trigger_event,delivery,template_id,destination,enabled,updated_at",
      [
        v.id,
        normalizeShopDomain(shop),
        v.name,
        v.trigger,
        v.delivery,
        v.templateId,
        v.destination,
        v.enabled,
      ],
    );
    return automationRow(r.rows[0]);
  }
  async deleteAutomation(shop: string, id: string) {
    const r = await this.pool.query(
      "DELETE FROM shopify_automation_rules WHERE shop=$1 AND id=$2",
      [normalizeShopDomain(shop), id],
    );
    return r.rowCount === 1;
  }
  async listActivity(shop: string, query = "", state = "") {
    const term = `%${query.replaceAll("%", "\\%").replaceAll("_", "\\_")}%`;
    const r = await this.pool.query(
      "SELECT id,order_name,document_name,destination,state,created_at FROM shopify_print_activity WHERE shop=$1 AND ($2='' OR state=$2) AND ($3='%%' OR order_name ILIKE $3 ESCAPE E'\\\\' OR document_name ILIKE $3 ESCAPE E'\\\\') ORDER BY created_at DESC LIMIT 100",
      [normalizeShopDomain(shop), state, term],
    );
    return r.rows.map((x: any) => ({
      id: x.id,
      orderName: x.order_name,
      documentName: x.document_name,
      destination: x.destination,
      state: x.state,
      createdAt: new Date(x.created_at).toISOString(),
    }));
  }
  async recordActivity(shop: string, value: Omit<ActivityEntry, "createdAt">) {
    await this.pool.query(
      "INSERT INTO shopify_print_activity(id,shop,order_name,document_name,destination,state) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT(id,shop) DO UPDATE SET state=EXCLUDED.state,destination=EXCLUDED.destination",
      [
        value.id,
        normalizeShopDomain(shop),
        value.orderName,
        value.documentName,
        value.destination,
        value.state,
      ],
    );
  }
  async getBilling(shop: string) {
    const r = await this.pool.query(
      `SELECT COALESCE(b.mode,'existing_piqae') AS mode,
        COALESCE(b.plan_handle,'free') AS plan_handle,
        COALESCE(b.plan_limit,50) AS plan_limit,
        COALESCE(b.status,'active') AS status,
        COALESCE((SELECT SUM(u.document_count)::integer
          FROM shopify_document_usage_events u
          WHERE u.shop=$1
            AND u.created_at >= date_trunc('month', now() AT TIME ZONE 'UTC') AT TIME ZONE 'UTC'),0) AS used_count
       FROM shopify_installations i
       LEFT JOIN shopify_billing_state b ON b.shop=i.shop
       WHERE i.shop=$1`,
      [normalizeShopDomain(shop)],
    );
    const x = r.rows[0];
    return x
      ? {
          mode: x.mode,
          plan: x.plan_handle,
          used: x.used_count,
          limit: x.plan_limit,
          status: x.status,
        }
      : structuredClone(DEFAULT_BILLING);
  }
  async saveBilling(shop: string, v: BillingState) {
    await this.pool.query(
      "INSERT INTO shopify_billing_state(shop,mode,plan_handle,used_count,plan_limit,status) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT(shop) DO UPDATE SET mode=EXCLUDED.mode,plan_handle=EXCLUDED.plan_handle,used_count=EXCLUDED.used_count,plan_limit=EXCLUDED.plan_limit,status=EXCLUDED.status,updated_at=now()",
      [normalizeShopDomain(shop), v.mode, v.plan, v.used, v.limit, v.status],
    );
  }
  async recordUsage(shop: string, eventKey: string, documents: number) {
    if (
      !eventKey ||
      eventKey.length > 200 ||
      !Number.isInteger(documents) ||
      documents < 1 ||
      documents > 10_000
    )
      throw new Error("Document usage event is invalid");
    const result = await this.pool.query(
      "INSERT INTO shopify_document_usage_events(shop,event_key,document_count) VALUES($1,$2,$3) ON CONFLICT(shop,event_key) DO NOTHING",
      [normalizeShopDomain(shop), eventKey, documents],
    );
    return result.rowCount === 1;
  }
}
const TEMPLATE_SELECT = `SELECT
  t.id,t.name,t.kind,t.page_size,t.draft_source,t.draft_revision,
  t.design_target_id,t.design_specification_revision,t.updated_at,t.published_revision,
  r.name AS published_name,r.kind AS published_kind,r.page_size AS published_page_size,
  r.source AS published_source,r.design_target_id AS published_design_target_id,
  r.design_specification_revision AS published_design_specification_revision,
  r.media AS published_media
FROM shopify_workflow_templates t
LEFT JOIN shopify_workflow_template_revisions r
  ON r.template_id=t.id AND r.shop=t.shop AND r.revision=t.published_revision`;

function templateRow(x: any): MerchantTemplate {
  const draftRevision = Number(x.draft_revision);
  const published =
    x.published_revision === null
      ? null
      : {
          revision: Number(x.published_revision),
          name: x.published_name,
          kind: x.published_kind,
          pageSize: x.published_page_size,
          source: x.published_source,
          designTargetId: x.published_design_target_id ?? null,
          designSpecificationRevision:
            x.published_design_specification_revision ?? null,
          media: x.published_media,
        };
  return {
    id: String(x.id),
    name: x.name,
    kind: x.kind,
    pageSize: x.page_size,
    state: published ? "published" : "draft",
    source: x.draft_source,
    revision: published?.revision ?? draftRevision,
    draftRevision,
    designTargetId: x.design_target_id ?? null,
    designSpecificationRevision: x.design_specification_revision ?? null,
    published,
    updatedAt: new Date(x.updated_at).toISOString(),
  };
}
function automationRow(x: any): AutomationRule {
  return {
    id: String(x.id),
    name: x.name,
    trigger: x.trigger_event,
    delivery: x.delivery,
    templateId: String(x.template_id),
    destination: x.destination,
    enabled: x.enabled,
    updatedAt: new Date(x.updated_at).toISOString(),
  };
}

let injected: WorkflowRepository | undefined;
let production: WorkflowRepository | undefined;
const development = new MemoryWorkflowRepository();
export function setWorkflowRepositoryForTests(
  value: WorkflowRepository | undefined,
) {
  injected = value;
}
export function workflows(): WorkflowRepository {
  if (injected) return injected;
  if (resolveShopifyStorage() === "memory") return development;
  if (!production) {
    const connectionString = process.env.DATABASE_URL;
    if (!connectionString) throw new Error("DATABASE_URL is required");
    production = new PostgresWorkflowRepository(
      new pg.Pool({ connectionString, max: 10, statement_timeout: 10_000 }),
    );
  }
  return production;
}
export function newWorkflowId() {
  return randomUUID();
}

export function parseSettings(
  form: FormData,
  existing: MerchantSettings = DEFAULT_SETTINGS,
): MerchantSettings {
  const fields = String(form.get("metafields") ?? "")
    .split(/[\n,]/)
    .map((v) => v.trim())
    .filter(Boolean);
  if (
    fields.length > 50 ||
    fields.some(
      (v) =>
        !/^(?:(?:order|product|variant):)?[a-z0-9_-]{1,64}\.[a-z0-9_-]{1,64}(?:\.[a-z0-9_-]{1,64})?$/i.test(
          v,
        ),
    )
  )
    throw new Error(
      "Metafields must use [order:|product:|variant:]namespace.key[.metaobject_field] and contain at most 50 entries",
    );
  const retention = Number(form.get("retentionDays") ?? 30);
  if (!Number.isInteger(retention) || retention < 1 || retention > 365)
    throw new Error("Retention must be between 1 and 365 days");
  return {
    defaultPrinterId: bounded(form, "defaultPrinterId", 200),
    defaultTemplateId: bounded(form, "defaultTemplateId", 200),
    preferDirect: form.get("preferDirect") === "on",
    offerPdf: form.get("offerPdf") === "on",
    metafieldAllowlist: [...new Set(fields)],
    retentionDays: retention,
    renderExecutionPolicy: parseRenderExecutionPolicy(
      form.get("renderExecutionPolicy"),
    ),
    printOrder: structuredClone(existing.printOrder),
  };
}
export function parseRenderExecutionPolicy(
  value: FormDataEntryValue | string | null | undefined,
): RenderExecutionPolicy {
  if (value === null || value === undefined || value === "") return "automatic";
  if (
    value === "automatic" ||
    value === "cloud_only" ||
    value === "prefer_node" ||
    value === "require_node"
  )
    return value;
  throw new Error(
    "Render location must be Automatic, Cloud only, Prefer node, or Require node",
  );
}
export function bounded(
  form: FormData,
  key: string,
  max: number,
  required = false,
) {
  const value = String(form.get(key) ?? "").trim();
  if ((required && !value) || value.length > max)
    throw new Error(`${key} is invalid`);
  return value;
}

export function validateDocumentSource(source: string): string {
  parseTemplateEnvelope(source);
  return source;
}
