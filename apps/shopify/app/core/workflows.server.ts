import { randomUUID } from "node:crypto";
import pg, { type Pool } from "pg";
import { normalizeShopDomain } from "./model";
import { parseTemplateEnvelope } from "./template-model";
import { resolveShopifyStorage } from "./piqae-runtime.server";

export type MerchantSettings = {
  defaultPrinterId: string;
  defaultTemplateId: string;
  preferDirect: boolean;
  offerPdf: boolean;
  metafieldAllowlist: string[];
  retentionDays: number;
};
export type MerchantTemplate = {
  id: string;
  name: string;
  kind: string;
  pageSize: string;
  state: "draft" | "published";
  source: string;
  revision: number;
  updatedAt: string;
};
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
  listTemplates(shop: string): Promise<MerchantTemplate[]>;
  getTemplate(shop: string, id: string): Promise<MerchantTemplate | null>;
  saveTemplate(
    shop: string,
    value: Omit<MerchantTemplate, "updatedAt">,
  ): Promise<MerchantTemplate>;
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
}

export class MemoryWorkflowRepository implements WorkflowRepository {
  private settings = new Map<string, MerchantSettings>();
  private templates = new Map<string, MerchantTemplate[]>();
  private automations = new Map<string, AutomationRule[]>();
  private activity = new Map<string, ActivityEntry[]>();
  private billing = new Map<string, BillingState>();
  async getSettings(shop: string) {
    return structuredClone(
      this.settings.get(normalizeShopDomain(shop)) ?? DEFAULT_SETTINGS,
    );
  }
  async saveSettings(shop: string, value: MerchantSettings) {
    this.settings.set(normalizeShopDomain(shop), structuredClone(value));
  }
  async listTemplates(shop: string) {
    return structuredClone(this.templates.get(normalizeShopDomain(shop)) ?? []);
  }
  async getTemplate(shop: string, id: string) {
    return (
      (await this.listTemplates(shop)).find((value) => value.id === id) ?? null
    );
  }
  async saveTemplate(shop: string, value: Omit<MerchantTemplate, "updatedAt">) {
    const key = normalizeShopDomain(shop);
    const all = this.templates.get(key) ?? [];
    const previous = all.find((item) => item.id === value.id);
    const saved = {
      ...value,
      revision:
        previous && value.state === "published"
          ? previous.revision + 1
          : value.revision,
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
    if (all.find((value) => value.id === id)?.state !== "draft") return false;
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
    return structuredClone(
      this.billing.get(normalizeShopDomain(shop)) ?? DEFAULT_BILLING,
    );
  }
  async saveBilling(shop: string, value: BillingState) {
    this.billing.set(normalizeShopDomain(shop), structuredClone(value));
  }
}

export class PostgresWorkflowRepository implements WorkflowRepository {
  constructor(private pool: Pool) {}
  async getSettings(shop: string) {
    const result = await this.pool.query(
      "SELECT default_printer_id,default_template_id,prefer_direct,offer_pdf,metafield_allowlist,retention_days FROM shopify_merchant_settings WHERE shop=$1",
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
        }
      : structuredClone(DEFAULT_SETTINGS);
  }
  async saveSettings(shop: string, v: MerchantSettings) {
    await this.pool.query(
      "INSERT INTO shopify_merchant_settings(shop,default_printer_id,default_template_id,prefer_direct,offer_pdf,metafield_allowlist,retention_days) VALUES($1,$2,$3,$4,$5,$6,$7) ON CONFLICT(shop) DO UPDATE SET default_printer_id=EXCLUDED.default_printer_id,default_template_id=EXCLUDED.default_template_id,prefer_direct=EXCLUDED.prefer_direct,offer_pdf=EXCLUDED.offer_pdf,metafield_allowlist=EXCLUDED.metafield_allowlist,retention_days=EXCLUDED.retention_days,updated_at=now()",
      [
        normalizeShopDomain(shop),
        v.defaultPrinterId || null,
        v.defaultTemplateId || null,
        v.preferDirect,
        v.offerPdf,
        v.metafieldAllowlist,
        v.retentionDays,
      ],
    );
  }
  async listTemplates(shop: string) {
    const result = await this.pool.query(
      "SELECT id,name,kind,page_size,state,source,revision,updated_at FROM shopify_workflow_templates WHERE shop=$1 ORDER BY updated_at DESC LIMIT 100",
      [normalizeShopDomain(shop)],
    );
    return result.rows.map(templateRow);
  }
  async getTemplate(shop: string, id: string) {
    const result = await this.pool.query(
      "SELECT id,name,kind,page_size,state,source,revision,updated_at FROM shopify_workflow_templates WHERE shop=$1 AND id=$2",
      [normalizeShopDomain(shop), id],
    );
    return result.rows[0] ? templateRow(result.rows[0]) : null;
  }
  async saveTemplate(shop: string, v: Omit<MerchantTemplate, "updatedAt">) {
    const client = await this.pool.connect();
    try {
      await client.query("BEGIN");
      const result = await client.query(
        "INSERT INTO shopify_workflow_templates(id,shop,name,kind,page_size,state,source,revision) VALUES($1,$2,$3,$4,$5,$6,$7,$8) ON CONFLICT(id,shop) DO UPDATE SET name=EXCLUDED.name,kind=EXCLUDED.kind,page_size=EXCLUDED.page_size,state=EXCLUDED.state,source=EXCLUDED.source,revision=CASE WHEN EXCLUDED.state='published' THEN shopify_workflow_templates.revision+1 ELSE shopify_workflow_templates.revision END,updated_at=now() RETURNING id,name,kind,page_size,state,source,revision,updated_at",
        [
          v.id,
          normalizeShopDomain(shop),
          v.name,
          v.kind,
          v.pageSize,
          v.state,
          v.source,
          v.revision,
        ],
      );
      const saved = templateRow(result.rows[0]);
      if (saved.state === "published") {
        await client.query(
          "INSERT INTO shopify_workflow_template_revisions(template_id,shop,revision,name,kind,page_size,source) VALUES($1,$2,$3,$4,$5,$6,$7) ON CONFLICT DO NOTHING",
          [
            saved.id,
            normalizeShopDomain(shop),
            saved.revision,
            saved.name,
            saved.kind,
            saved.pageSize,
            saved.source,
          ],
        );
      }
      await client.query("COMMIT");
      return saved;
    } catch (error) {
      await client.query("ROLLBACK");
      throw error;
    } finally {
      client.release();
    }
  }
  async deleteTemplate(shop: string, id: string) {
    const r = await this.pool.query(
      "DELETE FROM shopify_workflow_templates WHERE shop=$1 AND id=$2 AND state='draft'",
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
      "SELECT mode,plan_handle,used_count,plan_limit,status FROM shopify_billing_state WHERE shop=$1",
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
}
function templateRow(x: any): MerchantTemplate {
  return {
    id: String(x.id),
    name: x.name,
    kind: x.kind,
    pageSize: x.page_size,
    state: x.state,
    source: x.source,
    revision: x.revision,
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

export function parseSettings(form: FormData): MerchantSettings {
  const fields = String(form.get("metafields") ?? "")
    .split(/[\n,]/)
    .map((v) => v.trim())
    .filter(Boolean);
  if (
    fields.length > 50 ||
    fields.some((v) => !/^[a-z0-9_-]{1,64}\.[a-z0-9_-]{1,64}$/i.test(v))
  )
    throw new Error(
      "Metafields must use namespace.key and contain at most 50 entries",
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
  };
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
