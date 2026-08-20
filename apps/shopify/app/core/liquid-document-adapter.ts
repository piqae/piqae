import type {
  Block,
  BusinessDocument,
  Expression,
  Inline,
} from "./template-model";
export type LiquidDiagnostic = {
  code: string;
  line: number;
  column: number;
  message: string;
};
export type LiquidConversion =
  | {
      ok: true;
      document: BusinessDocument;
      normalizedSource: string;
      diagnostics: [];
    }
  | { ok: false; diagnostics: LiquidDiagnostic[] };
const TOKEN = /({{[-]?[\s\S]*?[-]?}}|{%[-]?[\s\S]*?[-]?%})/g;
const IDENT = /^[a-z_]\w*(?:\.[a-z_]\w*)*$/i;
type Frame = {
  kind: "root" | "repeat" | "conditional";
  blocks: Block[];
  variable?: string;
  node?: Extract<Block, { type: "conditional" }>;
  otherwise?: boolean;
};
export function liquidToCanonical(
  source: string,
  base?: BusinessDocument | BusinessDocument["media"],
): LiquidConversion {
  if (new TextEncoder().encode(source).byteLength > 65_536)
    return fail(source, 0, "source_too_large", "Liquid source exceeds 64 KiB");
  if (/<[a-z!/][^>]*>/i.test(source))
    return fail(
      source,
      source.search(/<[a-z!/]/i),
      "html_unsupported",
      "HTML and CSS are not part of the business-document profile",
    );
  const root: Block[] = [];
  const stack: Frame[] = [{ kind: "root", blocks: root }];
  const tokens = [...source.matchAll(TOKEN)];
  if (tokens.length > 4_000)
    return fail(
      source,
      0,
      "too_many_tokens",
      "Liquid source exceeds 4,000 tokens",
    );
  let cursor = 0;
  const appendText = (raw: string) => {
    for (const value of raw
      .split(/\n\s*\n/)
      .map((x) => x.trim())
      .filter(Boolean))
      stack
        .at(-1)!
        .blocks.push({ type: "paragraph", content: [{ type: "text", value }] });
  };
  for (const match of tokens) {
    appendText(source.slice(cursor, match.index));
    cursor = match.index! + match[0].length;
    const raw = match[0];
    const inner = raw.slice(2, -2).replace(/^-|-$/g, "").trim();
    if (raw.startsWith("{{")) {
      const expression = parseOutput(inner, stack);
      if (!expression)
        return fail(
          source,
          match.index!,
          "unsupported_output",
          `Unsupported output expression '${inner}'`,
        );
      const blocks = stack.at(-1)!.blocks;
      const previous = blocks.at(-1);
      if (previous?.type === "paragraph")
        previous.content.push({ type: "value", value: expression });
      else
        blocks.push({
          type: "paragraph",
          content: [{ type: "value", value: expression }],
        });
      continue;
    }
    const tag = inner.split(/\s+/, 1)[0];
    if (tag === "for") {
      const m =
        /^for\s+([a-z_]\w*)\s+in\s+([\w.]+)(?:\s+limit:\s*(\d+))?$/i.exec(
          inner,
        );
      if (!m)
        return fail(
          source,
          match.index!,
          "invalid_for",
          "Use: {% for item in order.lineItems limit: 1000 %}",
        );
      const limit = Number(m[3] ?? 1000);
      if (limit < 1 || limit > 1000)
        return fail(
          source,
          match.index!,
          "repeat_limit",
          "Loop limit must be between 1 and 1,000",
        );
      const node: Extract<Block, { type: "repeat" }> = {
        type: "repeat",
        items: scopedPath(m[2]!, stack),
        children: [],
      };
      stack.at(-1)!.blocks.push(node);
      stack.push({ kind: "repeat", blocks: node.children, variable: m[1] });
    } else if (tag === "endfor") {
      if (stack.at(-1)?.kind !== "repeat")
        return fail(
          source,
          match.index!,
          "unexpected_endfor",
          "endfor has no matching for",
        );
      stack.pop();
    } else if (tag === "if" || tag === "unless") {
      const condition = parseCondition(
        inner.slice(tag.length).trim(),
        tag === "unless",
        stack,
      );
      if (!condition)
        return fail(
          source,
          match.index!,
          "invalid_condition",
          "Conditions support paths and bounded comparisons",
        );
      const node: Extract<Block, { type: "conditional" }> = {
        type: "conditional",
        condition,
        then: [],
      };
      stack.at(-1)!.blocks.push(node);
      stack.push({ kind: "conditional", blocks: node.then, node });
    } else if (tag === "else") {
      const frame = stack.at(-1);
      if (frame?.kind !== "conditional" || !frame.node || frame.otherwise)
        return fail(
          source,
          match.index!,
          "unexpected_else",
          "else has no matching condition",
        );
      frame.node.else = [];
      frame.blocks = frame.node.else;
      frame.otherwise = true;
    } else if (tag === "endif" || tag === "endunless") {
      if (stack.at(-1)?.kind !== "conditional")
        return fail(
          source,
          match.index!,
          "unexpected_end_condition",
          `${tag} has no matching condition`,
        );
      stack.pop();
    } else if (tag === "piqae_table") {
      const m = /^piqae_table\s+([\w.]+)\s+as:\s*([a-z_]\w*)$/i.exec(inner);
      if (!m)
        return fail(
          source,
          match.index!,
          "invalid_table",
          "Use: {% piqae_table order.lineItems as: line %}",
        );
      stack.at(-1)!.blocks.push(table(scopedPath(m[1]!, stack), m[2]!));
    } else if (tag === "piqae_qr") {
      const m = /^piqae_qr\s+([\w.]+)$/i.exec(inner);
      if (!m)
        return fail(
          source,
          match.index!,
          "invalid_qr",
          "QR requires a value path",
        );
      stack.at(-1)!.blocks.push({
        type: "qr",
        value: scopedPath(m[1]!, stack),
        size_mm: 24,
      });
    } else if (tag === "piqae_barcode") {
      const m = /^piqae_barcode\s+([\w.]+)(?:\s+symbology:\s*code128)?$/i.exec(
        inner,
      );
      if (!m)
        return fail(
          source,
          match.index!,
          "invalid_barcode",
          "Barcode requires a Code 128 value path",
        );
      stack.at(-1)!.blocks.push({
        type: "barcode",
        value: scopedPath(m[1]!, stack),
        symbology: "code128",
        width_mm: 48,
        height_mm: 16,
        human_readable: true,
      });
    } else if (tag === "piqae_divider")
      stack.at(-1)!.blocks.push({ type: "divider" });
    else if (tag === "piqae_page_break")
      stack.at(-1)!.blocks.push({ type: "page_break" });
    else
      return fail(
        source,
        match.index!,
        "unsupported_tag",
        `Tag '${tag}' is not supported; includes, render, assign, capture and plugins are disabled`,
      );
    if (stack.length > 12)
      return fail(
        source,
        match.index!,
        "nesting_limit",
        "Liquid nesting exceeds 12 levels",
      );
  }
  appendText(source.slice(cursor));
  if (stack.length !== 1)
    return fail(
      source,
      source.length,
      "unclosed_block",
      `Unclosed ${stack.at(-1)!.kind} block`,
    );
  const media = base && "format" in base ? base.media : base;
  const document: BusinessDocument = {
    ...(base && "format" in base
      ? base
      : {
          format: "piqae.business-document/v1" as const,
          theme: {
            font_size_pt: 10,
            line_height: 1.35,
            text_color: { red: 32, green: 34, blue: 35 },
          },
          resources: {},
        }),
    format: "piqae.business-document/v1",
    media: media ?? {
      kind: "paged",
      size: "a4",
      orientation: "portrait",
      margins: { top_mm: 14, right_mm: 14, bottom_mm: 16, left_mm: 14 },
    },
    body: root,
  };
  return {
    ok: true,
    document,
    normalizedSource: canonicalToLiquid(document).source,
    diagnostics: [],
  };
}
export function canonicalToLiquid(document: BusinessDocument) {
  const lines: string[] = [];
  const inline = (items: Inline[]) =>
    items
      .map((item) =>
        item.type === "text"
          ? item.value
          : item.type === "line_break"
            ? "\n"
            : `{{ ${toLiquid(item.value)} }}`,
      )
      .join("");
  const emit = (blocks: Block[], depth = 0) => {
    const pad = "  ".repeat(depth);
    for (const block of blocks) {
      if (block.type === "paragraph" || block.type === "heading")
        lines.push(pad + inline(block.content));
      else if (block.type === "repeat") {
        lines.push(
          `${pad}{% for item in ${toLiquid(block.items)} limit: 1000 %}`,
        );
        emit(block.children, depth + 1);
        lines.push(`${pad}{% endfor %}`);
      } else if (block.type === "conditional") {
        const negated = block.condition.type === "not";
        const condition =
          negated && "value" in block.condition
            ? (block.condition.value as Expression)
            : block.condition;
        lines.push(
          `${pad}{% ${negated ? "unless" : "if"} ${toLiquid(condition)} %}`,
        );
        emit(block.then, depth + 1);
        if (block.else?.length) {
          lines.push(`${pad}{% else %}`);
          emit(block.else, depth + 1);
        }
        lines.push(`${pad}{% ${negated ? "endunless" : "endif"} %}`);
      } else if (block.type === "table")
        lines.push(`${pad}{% piqae_table ${toLiquid(block.items)} as: line %}`);
      else if (block.type === "qr")
        lines.push(`${pad}{% piqae_qr ${toLiquid(block.value)} %}`);
      else if (block.type === "barcode")
        lines.push(
          `${pad}{% piqae_barcode ${toLiquid(block.value)} symbology: code128 %}`,
        );
      else if (block.type === "divider")
        lines.push(`${pad}{% piqae_divider %}`);
      else if (block.type === "page_break")
        lines.push(`${pad}{% piqae_page_break %}`);
      else if ("children" in block) emit(block.children, depth);
    }
  };
  emit(document.body);
  return { source: lines.join("\n"), diagnostics: [] as LiquidDiagnostic[] };
}
function parseOutput(input: string, stack: Frame[]): Expression | null {
  const [name, ...filters] = input.split("|").map((x) => x.trim());
  if (!IDENT.test(name!)) return null;
  let value = scopedPath(name!, stack);
  for (const raw of filters) {
    const filter = raw.split(":")[0]!.trim();
    if (filter === "number") value = { type: "format_number", value };
    else if (filter === "money")
      value = {
        type: "format_money",
        amount: value,
        currency: scopedPath(
          `${name!.split(".")[0] === "line_item" || name!.split(".")[0] === "item" ? name!.split(".")[0] : "order"}.currency`,
          stack,
        ),
      };
    else if (filter === "date")
      value = { type: "format_date", value, format: "day_month_year" };
    else return null;
  }
  return value;
}
function parseCondition(
  input: string,
  negate: boolean,
  stack: Frame[],
): Expression | null {
  const m =
    /^([\w.]+)(?:\s*(==|!=|>=|<=|>|<)\s*("[^"]*"|'[^']*'|true|false|null|-?\d+(?:\.\d+)?|[\w.]+))?$/.exec(
      input,
    );
  if (!m) return null;
  let value: Expression = m[2]
    ? {
        type: "compare",
        operator: (
          {
            "==": "equal",
            "!=": "not_equal",
            ">": "greater",
            ">=": "greater_or_equal",
            "<": "less",
            "<=": "less_or_equal",
          } as const
        )[m[2] as "=="],
        left: scopedPath(m[1]!, stack),
        right: literalOrPath(m[3]!, stack),
      }
    : scopedPath(m[1]!, stack);
  return negate ? { type: "not", value } : value;
}
function scopedPath(input: string, stack: Frame[]): Expression {
  const parts = input.split(".");
  const variable = [...stack]
    .reverse()
    .find((frame) => frame.variable === parts[0])?.variable;
  return variable
    ? ({ type: "current_path", path: parts.slice(1) } as Expression)
    : { type: "path", path: parts };
}
function path(input: string): Expression {
  return { type: "path", path: input.split(".") };
}
function literalOrPath(value: string, stack: Frame[]): Expression {
  if (/^['"]/.test(value))
    return { type: "literal", value: value.slice(1, -1) };
  if (["true", "false"].includes(value))
    return { type: "literal", value: value === "true" };
  if (value === "null") return { type: "literal", value: null };
  if (/^-?\d/.test(value)) return { type: "literal", value: Number(value) };
  return scopedPath(value, stack);
}
function toLiquid(value: Expression): string {
  if (value.type === "path") return value.path.join(".");
  if (value.type === "current_path") return `item.${value.path.join(".")}`;
  if (value.type === "literal")
    return typeof value.value === "string"
      ? JSON.stringify(value.value)
      : String(value.value);
  if (value.type === "format_number")
    return `${toLiquid(value.value)} | number`;
  if (value.type === "format_money") return `${toLiquid(value.amount)} | money`;
  if (value.type === "format_date") return `${toLiquid(value.value)} | date`;
  if (value.type === "not") return `not ${toLiquid(value.value)}`;
  if (value.type === "compare")
    return `${toLiquid(value.left)} ${{ equal: "==", not_equal: "!=", greater: ">", greater_or_equal: ">=", less: "<", less_or_equal: "<=" }[value.operator]} ${toLiquid(value.right)}`;
  return "unsupported";
}
function table(
  items: Expression,
  variable: string,
): Extract<Block, { type: "table" }> {
  const current = (name: string): Expression =>
    ({ type: "current_path", path: [name] }) as Expression;
  void variable;
  return {
    type: "table",
    items,
    repeat_header: true,
    empty: [],
    columns: [
      {
        header: [{ type: "text", value: "Item" }],
        cell: [{ type: "value", value: current("title") }],
        width: 5,
      },
      {
        header: [{ type: "text", value: "Qty" }],
        cell: [{ type: "value", value: current("quantity") }],
        width: 1,
        align: "right",
      },
      {
        header: [{ type: "text", value: "Total" }],
        cell: [
          {
            type: "value",
            value: {
              type: "format_money",
              amount: current("total"),
              currency: current("currency"),
            },
          },
        ],
        width: 2,
        align: "right",
      },
    ],
  };
}
function fail(
  source: string,
  index: number,
  code: string,
  message: string,
): LiquidConversion {
  const before = source.slice(0, index).split("\n");
  return {
    ok: false,
    diagnostics: [
      { code, line: before.length, column: before.at(-1)!.length + 1, message },
    ],
  };
}
