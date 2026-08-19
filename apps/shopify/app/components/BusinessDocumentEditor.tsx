import { useEffect, useRef, useState } from "react";
import { Schema, type Node as ProseMirrorNode } from "prosemirror-model";
import { EditorState } from "prosemirror-state";
import { EditorView } from "prosemirror-view";
import { history, undo, redo } from "prosemirror-history";
import { keymap } from "prosemirror-keymap";
import { baseKeymap } from "prosemirror-commands";
import type {
  Block,
  BusinessDocument,
  Expression,
  Inline,
  TextStyle,
} from "../core/template-model";

const schema = new Schema({
  nodes: {
    doc: { content: "block+" },
    text: { group: "inline" },
    paragraph: {
      group: "block",
      content: "inline*",
      attrs: { align: { default: "start" } },
      toDOM: (n) => ["p", { "data-align": n.attrs.align }, 0],
      parseDOM: [{ tag: "p" }],
    },
    heading: {
      group: "block",
      content: "inline*",
      attrs: { level: { default: 2 } },
      toDOM: (n) => [`h${n.attrs.level}`, 0],
      parseDOM: [
        { tag: "h1", attrs: { level: 1 } },
        { tag: "h2", attrs: { level: 2 } },
        { tag: "h3", attrs: { level: 3 } },
      ],
    },
    variable: {
      group: "inline",
      inline: true,
      atom: true,
      attrs: { expression: {}, label: {} },
      toDOM: (n) => [
        "span",
        {
          class: "piqae-variable",
          "data-expression": n.attrs.expression,
        },
        n.attrs.label,
      ],
      parseDOM: [
        {
          tag: "span[data-expression]",
          getAttrs: (el) => ({
            expression: (el as HTMLElement).dataset.expression,
            label: (el as HTMLElement).textContent,
          }),
        },
      ],
    },
    hard_break: {
      group: "inline",
      inline: true,
      selectable: false,
      toDOM: () => ["br"],
      parseDOM: [{ tag: "br" }],
    },
    table_block: {
      group: "block",
      atom: true,
      attrs: { json: {} },
      toDOM: () => [
        "div",
        { class: "piqae-structured-block" },
        "↕ Repeating line-item table",
      ],
      parseDOM: [{ tag: "div.piqae-structured-block" }],
    },
    repeat_block: {
      group: "block",
      atom: true,
      attrs: { json: {} },
      toDOM: (n) => [
        "div",
        { class: "piqae-structured-block" },
        `↻ Repeat ${JSON.parse(n.attrs.json).as}`,
      ],
      parseDOM: [{ tag: "div.piqae-structured-block" }],
    },
    conditional_block: {
      group: "block",
      atom: true,
      attrs: { json: {} },
      toDOM: () => [
        "div",
        { class: "piqae-structured-block" },
        "◇ Conditional section",
      ],
      parseDOM: [{ tag: "div.piqae-structured-block" }],
    },
    machine_code: {
      group: "block",
      atom: true,
      attrs: { json: {} },
      toDOM: (n) => [
        "div",
        { class: "piqae-structured-block" },
        JSON.parse(n.attrs.json).type === "qr" ? "▦ QR code" : "▥ Barcode",
      ],
      parseDOM: [{ tag: "div.piqae-structured-block" }],
    },
    structured_block: {
      group: "block",
      atom: true,
      attrs: { json: {}, label: {} },
      toDOM: (n) => ["div", { class: "piqae-structured-block" }, n.attrs.label],
    },
    divider: {
      group: "block",
      atom: true,
      toDOM: () => ["hr"],
      parseDOM: [{ tag: "hr" }],
    },
  },
  marks: {
    strong: {
      toDOM: () => ["strong", 0],
      parseDOM: [{ tag: "strong" }, { tag: "b" }],
    },
    em: { toDOM: () => ["em", 0], parseDOM: [{ tag: "em" }, { tag: "i" }] },
    underline: { toDOM: () => ["u", 0], parseDOM: [{ tag: "u" }] },
  },
});

export const SHOPIFY_VARIABLES = [
  "shop.name",
  "order.name",
  "order.createdAt",
  "order.customer.displayName",
  "order.billingAddress.formatted",
  "order.shippingAddress.formatted",
  "order.subtotal",
  "order.taxTotal",
  "order.total",
  "order.statusUrl",
  "line.title",
  "line.quantity",
  "line.price",
  "line.total",
  "refundLine.title",
  "refundLine.total",
] as const;

export function BusinessDocumentEditor({
  value,
  disabled,
  onChange,
}: {
  value: BusinessDocument;
  disabled?: boolean;
  onChange(document: BusinessDocument): void;
}) {
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView | null>(null);
  const latest = useRef(value);
  const [selection, setSelection] = useState<{
    position: number;
    block: Block;
  } | null>(null);
  latest.current = value;
  useEffect(() => {
    if (!host.current) return;
    const state = EditorState.create({
      schema,
      doc: blocksToDoc(value.body),
      plugins: [
        history(),
        keymap({ "Mod-z": undo, "Shift-Mod-z": redo }),
        keymap(baseKeymap),
      ],
    });
    view.current = new EditorView(host.current, {
      state,
      editable: () => !disabled,
      dispatchTransaction(transaction) {
        const next = view.current!.state.apply(transaction);
        view.current!.updateState(next);
        const selected = next.selection.$from.nodeAfter;
        setSelection(
          selected?.attrs.json
            ? {
                position: next.selection.from,
                block: JSON.parse(selected.attrs.json) as Block,
              }
            : null,
        );
        onChange({ ...latest.current, body: docToBlocks(next.doc) });
      },
    });
    return () => {
      view.current?.destroy();
      view.current = null;
    };
  }, [disabled]);
  const insert = (node: ProseMirrorNode) => {
    const instance = view.current;
    if (!instance) return;
    instance.dispatch(
      instance.state.tr.replaceSelectionWith(node).scrollIntoView(),
    );
    instance.focus();
  };
  const insertVariable = (path: string) =>
    insert(
      schema.nodes.variable!.create({
        expression: JSON.stringify({ type: "path", path: path.split(".") }),
        label: path,
      }),
    );
  const updateSelected = (block: Block) => {
    const instance = view.current;
    if (!instance || !selection) return;
    instance.dispatch(
      instance.state.tr.setNodeMarkup(selection.position, undefined, {
        json: JSON.stringify(block),
      }),
    );
    setSelection({ ...selection, block });
  };
  const updateRegion = (region: "header" | "footer", content: string) => {
    const block: Block = {
      type: "paragraph",
      content: [{ type: "text", value: content }],
    };
    onChange({
      ...latest.current,
      [region]: {
        first: latest.current[region]?.first ?? [],
        default: content ? [block] : [],
        last: latest.current[region]?.last ?? [],
      },
    });
  };
  return (
    <div className="piqae-word-editor">
      <div
        className="piqae-word-toolbar"
        role="toolbar"
        aria-label="Document formatting"
      >
        <button
          type="button"
          onClick={() =>
            insert(
              schema.nodes.heading!.create(
                { level: 2 },
                schema.text("Heading"),
              ),
            )
          }
        >
          Heading
        </button>
        <select
          aria-label="Insert Shopify variable"
          defaultValue=""
          onChange={(e) => {
            if (e.currentTarget.value) insertVariable(e.currentTarget.value);
            e.currentTarget.value = "";
          }}
        >
          <option value="">Insert variable…</option>
          {SHOPIFY_VARIABLES.map((item) => (
            <option key={item}>{item}</option>
          ))}
        </select>
        <button
          type="button"
          onClick={() => insert(blockNode("table_block", defaultTable()))}
        >
          Line items
        </button>
        <button
          type="button"
          onClick={() =>
            insert(
              blockNode("conditional_block", {
                type: "conditional",
                condition: { type: "path", path: ["order", "taxTotal"] },
                then: [
                  {
                    type: "paragraph",
                    content: [{ type: "text", value: "Tax" }],
                  },
                ],
              }),
            )
          }
        >
          Condition
        </button>
        <button
          type="button"
          onClick={() =>
            insert(
              blockNode("machine_code", {
                type: "qr",
                value: { type: "path", path: ["order", "statusUrl"] },
                size_mm: 24,
              }),
            )
          }
        >
          QR code
        </button>
        <button
          type="button"
          onClick={() =>
            insert(
              blockNode("machine_code", {
                type: "barcode",
                value: { type: "path", path: ["order", "name"] },
                symbology: "code128",
                width_mm: 48,
                height_mm: 16,
                human_readable: true,
              }),
            )
          }
        >
          Barcode
        </button>
      </div>
      <div className="piqae-region-editors">
        <label>
          Repeating page header
          <input
            type="text"
            value={regionText(value.header?.default)}
            onChange={(event) =>
              updateRegion("header", event.currentTarget.value)
            }
            disabled={disabled}
            placeholder="Optional header text"
          />
        </label>
        <label>
          Repeating page footer
          <input
            type="text"
            value={regionText(value.footer?.default)}
            onChange={(event) =>
              updateRegion("footer", event.currentTarget.value)
            }
            disabled={disabled}
            placeholder="Optional footer text"
          />
        </label>
      </div>
      <div className="piqae-page-sheet" ref={host} />
      {selection ? (
        <BlockProperties
          block={selection.block}
          disabled={disabled}
          onChange={updateSelected}
        />
      ) : null}
    </div>
  );
}

function BlockProperties({
  block,
  disabled,
  onChange,
}: {
  block: Block;
  disabled?: boolean;
  onChange(block: Block): void;
}) {
  if (block.type === "table")
    return (
      <aside className="piqae-block-properties">
        <strong>Line-item table</strong>
        <label>
          Collection path
          <input
            value={expressionLabel(block.items)}
            disabled={disabled}
            onChange={(event) =>
              onChange({
                ...block,
                items: pathExpression(event.currentTarget.value),
              })
            }
          />
        </label>
        <label>
          <input
            type="checkbox"
            checked={block.repeat_header ?? false}
            disabled={disabled}
            onChange={(event) =>
              onChange({ ...block, repeat_header: event.currentTarget.checked })
            }
          />{" "}
          Repeat header on each page
        </label>
        <label>
          Columns
          <input
            value={block.columns
              .map((column) => inlineLabel(column.header))
              .join(", ")}
            disabled={disabled}
            onChange={(event) => {
              const labels = event.currentTarget.value
                .split(",")
                .map((value) => value.trim());
              onChange({
                ...block,
                columns: block.columns.map((column, index) => ({
                  ...column,
                  header: [
                    {
                      type: "text" as const,
                      value: labels[index] ?? inlineLabel(column.header),
                    },
                  ],
                })),
              });
            }}
          />
        </label>
      </aside>
    );
  if (block.type === "conditional")
    return (
      <aside className="piqae-block-properties">
        <strong>Conditional section</strong>
        <label>
          Show when path is truthy
          <input
            value={expressionLabel(block.condition)}
            disabled={disabled}
            onChange={(event) =>
              onChange({
                ...block,
                condition: pathExpression(event.currentTarget.value),
              })
            }
          />
        </label>
      </aside>
    );
  if (block.type === "qr")
    return (
      <aside className="piqae-block-properties">
        <strong>QR code</strong>
        <label>
          Value path
          <input
            value={expressionLabel(block.value)}
            disabled={disabled}
            onChange={(event) =>
              onChange({
                ...block,
                value: pathExpression(event.currentTarget.value),
              })
            }
          />
        </label>
        <label>
          Size (mm)
          <input
            type="number"
            min="10"
            max="80"
            value={block.size_mm}
            disabled={disabled}
            onChange={(event) =>
              onChange({ ...block, size_mm: Number(event.currentTarget.value) })
            }
          />
        </label>
      </aside>
    );
  if (block.type === "barcode")
    return (
      <aside className="piqae-block-properties">
        <strong>Code 128 barcode</strong>
        <label>
          Value path
          <input
            value={expressionLabel(block.value)}
            disabled={disabled}
            onChange={(event) =>
              onChange({
                ...block,
                value: pathExpression(event.currentTarget.value),
              })
            }
          />
        </label>
        <label>
          Width (mm)
          <input
            type="number"
            min="20"
            max="180"
            value={block.width_mm}
            disabled={disabled}
            onChange={(event) =>
              onChange({
                ...block,
                width_mm: Number(event.currentTarget.value),
              })
            }
          />
        </label>
        <label>
          Height (mm)
          <input
            type="number"
            min="8"
            max="80"
            value={block.height_mm}
            disabled={disabled}
            onChange={(event) =>
              onChange({
                ...block,
                height_mm: Number(event.currentTarget.value),
              })
            }
          />
        </label>
      </aside>
    );
  return null;
}

function regionText(blocks: Block[] | undefined) {
  const first = blocks?.[0];
  return first && (first.type === "paragraph" || first.type === "heading")
    ? first.content
        .map((item) =>
          item.type === "text"
            ? item.value
            : item.type === "value"
              ? `{{ ${expressionLabel(item.value)} }}`
              : "\n",
        )
        .join("")
    : "";
}
function pathExpression(value: string): Expression {
  return {
    type: "path",
    path: value
      .split(".")
      .map((part) => part.trim())
      .filter(Boolean),
  };
}
function inlineLabel(content: Inline[]) {
  return content
    .map((item) => (item.type === "text" ? item.value : "Value"))
    .join("");
}
function blockNode(
  type: "table_block" | "repeat_block" | "conditional_block" | "machine_code",
  value: Block,
) {
  return schema.nodes[type]!.create({ json: JSON.stringify(value) });
}
export function blocksToDoc(blocks: Block[]) {
  const nodes = blocks.map((block) => {
    if (block.type === "paragraph" || block.type === "heading")
      return schema.nodes[block.type]!.create(
        block.type === "heading"
          ? { level: block.level ?? 2 }
          : { align: block.style?.align ?? "left" },
        block.content.map(inlineToNode),
      );
    if (block.type === "divider") return schema.nodes.divider!.create();
    if (block.type === "table") return blockNode("table_block", block);
    if (block.type === "repeat") return blockNode("repeat_block", block);
    if (block.type === "conditional")
      return blockNode("conditional_block", block);
    if (block.type === "qr" || block.type === "barcode")
      return blockNode("machine_code", block);
    return schema.nodes.structured_block!.create({
      json: JSON.stringify(block),
      label: `▦ ${block.type.replaceAll("_", " ")}`,
    });
  });
  return schema.nodes.doc!.create(
    {},
    nodes.length ? nodes : [schema.nodes.paragraph!.create()],
  );
}
function inlineToNode(item: Inline) {
  if (item.type === "line_break") return schema.nodes.hard_break!.create();
  const style = item.style ?? {};
  const marks = [
    style.bold ? schema.marks.strong!.create() : null,
    style.italic ? schema.marks.em!.create() : null,
    style.underline ? schema.marks.underline!.create() : null,
  ].filter(Boolean) as import("prosemirror-model").Mark[];
  return item.type === "text"
    ? schema.text(item.value || " ", marks)
    : schema.nodes.variable!.create(
        {
          expression: JSON.stringify(item.value),
          label: expressionLabel(item.value),
        },
        undefined,
        marks,
      );
}
export function docToBlocks(doc: ProseMirrorNode): Block[] {
  const result: Block[] = [];
  doc.forEach((node) => {
    if (node.type.name === "paragraph" || node.type.name === "heading") {
      const content: Inline[] = [];
      node.forEach((child) => {
        const style: TextStyle = {};
        if (child.marks.some((mark) => mark.type.name === "strong"))
          style.bold = true;
        if (child.marks.some((mark) => mark.type.name === "em"))
          style.italic = true;
        if (child.marks.some((mark) => mark.type.name === "underline"))
          style.underline = true;
        const withStyle = Object.keys(style).length ? { style } : {};
        if (child.isText)
          content.push({ type: "text", value: child.text ?? "", ...withStyle });
        else if (child.type.name === "hard_break")
          content.push({ type: "line_break" });
        else
          content.push({
            type: "value",
            value: JSON.parse(String(child.attrs.expression)) as Expression,
            ...withStyle,
          });
      });
      result.push(
        node.type.name === "heading"
          ? { type: "heading", level: node.attrs.level, content }
          : {
              type: "paragraph",
              ...(node.attrs.align &&
              !["start", "left"].includes(node.attrs.align)
                ? {
                    style: {
                      align:
                        node.attrs.align === "end" ? "right" : node.attrs.align,
                    },
                  }
                : {}),
              content,
            },
      );
    } else if (node.type.name === "divider") result.push({ type: "divider" });
    else if (node.attrs.json) result.push(JSON.parse(node.attrs.json) as Block);
  });
  return result;
}
function expressionLabel(value: Expression) {
  if (value.type === "path") return value.path.join(".");
  if (value.type === "current_path") return value.path.join(".");
  return value.type.replaceAll("_", " ");
}
function defaultTable(): Block {
  const current = (key: string): Expression =>
    ({ type: "current_path", path: [key] }) as Expression;
  return {
    type: "table",
    items: { type: "path", path: ["order", "lineItems"] },
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
              currency: { type: "path", path: ["order", "currencyCode"] },
            },
          },
        ],
        width: 2,
        align: "right",
      },
    ],
  };
}
