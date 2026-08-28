import type { Block, Expression, Inline } from "./template-model";

export type ShopifyAuthoringScope = "order" | "item";

/** Compile Shopify's friendly aliases into the current renderer scope. */
export function authoringPathExpression(
  value: string,
  scope: ShopifyAuthoringScope,
): Expression {
  const parts = value
    .split(".")
    .map((part) => part.trim())
    .filter(Boolean);
  if (parts[0] === scope) return { type: "current_path", path: parts.slice(1) };
  return { type: "path", path: parts };
}

function scopeExpression(
  expression: Expression,
  scope: ShopifyAuthoringScope,
): Expression {
  if (expression.type === "path")
    return authoringPathExpression(expression.path.join("."), scope);
  if (
    expression.type === "literal" ||
    expression.type === "current_path" ||
    expression.type === "page_number" ||
    expression.type === "page_count"
  )
    return expression;
  if (expression.type === "coalesce" || expression.type === "concat")
    return {
      ...expression,
      values: expression.values.map((value) => scopeExpression(value, scope)),
    };
  if (expression.type === "compare" || expression.type === "arithmetic")
    return {
      ...expression,
      left: scopeExpression(expression.left, scope),
      right: scopeExpression(expression.right, scope),
    };
  if (expression.type === "boolean")
    return {
      ...expression,
      values: expression.values.map((value) => scopeExpression(value, scope)),
    };
  if (expression.type === "contains")
    return {
      ...expression,
      collection: scopeExpression(expression.collection, scope),
      value: scopeExpression(expression.value, scope),
    };
  if (expression.type === "format_money")
    return {
      ...expression,
      amount: scopeExpression(expression.amount, scope),
      currency: scopeExpression(expression.currency, scope),
    };
  if ("value" in expression)
    return {
      ...expression,
      value: scopeExpression(expression.value as Expression, scope),
    } as Expression;
  return expression;
}

function scopeInline(
  content: Inline[],
  scope: ShopifyAuthoringScope,
): Inline[] {
  return content.map((item) =>
    item.type === "value"
      ? { ...item, value: scopeExpression(item.value, scope) }
      : item,
  );
}

export function isLineItemsExpression(
  expression: Expression,
  scope: ShopifyAuthoringScope,
) {
  const scoped = scopeExpression(expression, scope);
  return (
    (scoped.type === "current_path" && scoped.path[0] === "lineItems") ||
    (scoped.type === "path" &&
      scoped.path.at(-1) === "lineItems" &&
      ["order", "item"].includes(scoped.path[0] ?? ""))
  );
}

function scopeBlocks(blocks: Block[], scope: ShopifyAuthoringScope): Block[] {
  return blocks.map((block): Block => {
    if (block.type === "paragraph" || block.type === "heading")
      return { ...block, content: scopeInline(block.content, scope) };
    if (block.type === "table")
      return {
        ...block,
        items: scopeExpression(block.items, scope),
        columns: block.columns.map((column) => ({
          ...column,
          header: scopeInline(column.header, scope),
          cell: scopeInline(column.cell, "item"),
        })),
        empty: scopeBlocks(block.empty ?? [], scope),
      };
    if (block.type === "repeat") {
      const childScope = isLineItemsExpression(block.items, scope)
        ? "item"
        : scope;
      return {
        ...block,
        items: scopeExpression(block.items, scope),
        children: scopeBlocks(block.children, childScope),
      };
    }
    if (block.type === "data_list")
      return {
        ...block,
        items: scopeExpression(block.items, scope),
        header: scopeBlocks(block.header ?? [], scope),
        item: scopeBlocks(block.item, "item"),
        empty: scopeBlocks(block.empty ?? [], scope),
      };
    if (block.type === "conditional")
      return {
        ...block,
        condition: scopeExpression(block.condition, scope),
        then: scopeBlocks(block.then, scope),
        else: scopeBlocks(block.else ?? [], scope),
      };
    if (
      block.type === "section" ||
      block.type === "stack" ||
      block.type === "row" ||
      block.type === "box" ||
      block.type === "keep_together" ||
      block.type === "grid"
    )
      return { ...block, children: scopeBlocks(block.children, scope) };
    if (block.type === "image_value")
      return { ...block, resource: scopeExpression(block.resource, scope) };
    if (block.type === "qr" || block.type === "barcode")
      return { ...block, value: scopeExpression(block.value, scope) };
    return block;
  });
}

function isOrdersRepeat(
  block: Block,
): block is Extract<Block, { type: "repeat" }> {
  return (
    block.type === "repeat" &&
    block.items.type === "path" &&
    block.items.path.length === 1 &&
    block.items.path[0] === "orders"
  );
}

/** Produces the canonical batch-safe body emitted by the visual editor. */
export function canonicalizeShopifyEditorBody(blocks: Block[]): Block[] {
  let gapMm: number | undefined;
  const orderChildren: Block[] = [];
  for (const block of blocks) {
    if (isOrdersRepeat(block)) {
      gapMm ??= block.gap_mm;
      orderChildren.push(...block.children);
    } else orderChildren.push(block);
  }
  return [
    {
      type: "repeat",
      items: { type: "path", path: ["orders"] },
      ...(gapMm === undefined ? {} : { gap_mm: gapMm }),
      children: scopeBlocks(orderChildren, "order"),
    },
  ];
}
