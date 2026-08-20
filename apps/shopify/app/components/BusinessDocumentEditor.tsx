import { useEffect, useRef, useState } from "react";
import {
  Schema,
  type DOMOutputSpec,
  type Node as ProseMirrorNode,
} from "prosemirror-model";
import { EditorState, NodeSelection } from "prosemirror-state";
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
import {
  SHOPIFY_DOCUMENT_FIELDS,
  type ShopifyDocumentField,
} from "../core/shopify-document-fields";

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
      toDOM: (n) => structuredBlockDOM(JSON.parse(n.attrs.json) as Block),
      parseDOM: [{ tag: "div.piqae-structured-block" }],
    },
    repeat_block: {
      group: "block",
      atom: true,
      attrs: { json: {} },
      toDOM: (n) => structuredBlockDOM(JSON.parse(n.attrs.json) as Block),
      parseDOM: [{ tag: "div.piqae-structured-block" }],
    },
    conditional_block: {
      group: "block",
      atom: true,
      attrs: { json: {} },
      toDOM: (n) => structuredBlockDOM(JSON.parse(n.attrs.json) as Block),
      parseDOM: [{ tag: "div.piqae-structured-block" }],
    },
    machine_code: {
      group: "block",
      atom: true,
      attrs: { json: {} },
      toDOM: (n) => structuredBlockDOM(JSON.parse(n.attrs.json) as Block),
      parseDOM: [{ tag: "div.piqae-structured-block" }],
    },
    structured_block: {
      group: "block",
      atom: true,
      attrs: { json: {}, label: { default: "Structured section" } },
      toDOM: (n) =>
        structuredBlockDOM(JSON.parse(n.attrs.json) as Block, n.attrs.label),
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

const AUTHORING_FIELDS: readonly ShopifyDocumentField[] = [
  ...SHOPIFY_DOCUMENT_FIELDS,
  { label: "Shop name", path: "shop.name", group: "Order" },
  {
    label: "Billing address",
    path: "order.billingAddress.formatted",
    group: "Order",
  },
  {
    label: "Order subtotal",
    path: "order.subtotal",
    group: "Order",
    conditionable: true,
  },
  {
    label: "Order tax",
    path: "order.taxTotal",
    group: "Order",
    conditionable: true,
  },
  {
    label: "Order total",
    path: "order.total",
    group: "Order",
    conditionable: true,
  },
  { label: "Order status link", path: "order.statusUrl", group: "Order" },
];
export const SHOPIFY_VARIABLES = AUTHORING_FIELDS.map((field) => field.path);

export function BusinessDocumentEditor({
  value,
  disabled,
  customFields = [],
  onChange,
}: {
  value: BusinessDocument;
  disabled?: boolean;
  customFields?: readonly ShopifyDocumentField[];
  onChange(document: BusinessDocument): void;
}) {
  const authoringFields = [...AUTHORING_FIELDS, ...customFields];
  const theme = value.theme ?? {};
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView | null>(null);
  const latest = useRef(value);
  const [selection, setSelection] = useState<{
    position: number;
    block: Block;
    path?: BlockPath;
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
      handleClickOn(_view, position, node) {
        if (!node.attrs.json) return false;
        setSelection({
          position,
          block: JSON.parse(node.attrs.json) as Block,
        });
        return false;
      },
      dispatchTransaction(transaction) {
        const next = view.current!.state.apply(transaction);
        view.current!.updateState(next);
        const selected =
          next.selection instanceof NodeSelection
            ? next.selection.node
            : next.selection.$from.nodeAfter;
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
    const inserted = docToBlocks(schema.nodes.doc!.create(null, [node]))[0];
    if (!inserted) return;
    const body = selection?.path
      ? insertBlockAfterPath(latest.current.body, selection.path, inserted)
      : [...latest.current.body, inserted];
    const nextDocument = { ...latest.current, body };
    latest.current = nextDocument;
    instance.updateState(
      EditorState.create({ schema, doc: blocksToDoc(body) }),
    );
    onChange(nextDocument);
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
    if (selection.path) {
      const body = replaceBlockAtPath(
        latest.current.body,
        selection.path,
        block,
      );
      const nextDocument = { ...latest.current, body };
      latest.current = nextDocument;
      instance.updateState(
        EditorState.create({
          schema,
          doc: blocksToDoc(body),
          plugins: [
            history(),
            keymap({ "Mod-z": undo, "Shift-Mod-z": redo }),
            keymap(baseKeymap),
          ],
        }),
      );
      setSelection({ position: -1, path: selection.path, block });
      onChange(nextDocument);
      return;
    }
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
  const insertBlock = (block: Block) =>
    insert(blockNode(nodeTypeForBlock(block), block));
  const removeSelected = () => {
    const instance = view.current;
    if (!instance || !selection) return;
    if (selection.path) {
      const body = removeBlockAtPath(latest.current.body, selection.path);
      const nextDocument = { ...latest.current, body };
      latest.current = nextDocument;
      instance.updateState(
        EditorState.create({ schema, doc: blocksToDoc(body) }),
      );
      onChange(nextDocument);
      setSelection(null);
      return;
    }
    const node = instance.state.doc.nodeAt(selection.position);
    if (!node) return;
    instance.dispatch(
      instance.state.tr
        .delete(selection.position, selection.position + node.nodeSize)
        .scrollIntoView(),
    );
    setSelection(null);
    instance.focus();
  };
  const moveSelected = (direction: -1 | 1) => {
    if (!selection?.path) return;
    const body = moveBlockAtPath(
      latest.current.body,
      selection.path,
      direction,
    );
    const nextPath = selection.path.map((part, index) =>
      index === selection.path!.length - 1
        ? { ...part, index: part.index + direction }
        : part,
    );
    const nextDocument = { ...latest.current, body };
    latest.current = nextDocument;
    view.current?.updateState(
      EditorState.create({ schema, doc: blocksToDoc(body) }),
    );
    setSelection({ position: -1, path: nextPath, block: selection.block });
    onChange(nextDocument);
  };
  const duplicateSelected = () => {
    if (!selection?.path) return;
    const body = insertBlockAfterPath(
      latest.current.body,
      selection.path,
      structuredClone(selection.block),
    );
    const nextDocument = { ...latest.current, body };
    latest.current = nextDocument;
    view.current?.updateState(
      EditorState.create({ schema, doc: blocksToDoc(body) }),
    );
    onChange(nextDocument);
  };
  return (
    <div className="piqae-word-editor">
      <div
        className="piqae-word-toolbar"
        role="toolbar"
        aria-label="Insert into document"
      >
        <span className="piqae-toolbar-label">Insert</span>
        <InsertButton
          icon="T"
          label="Text"
          onClick={() =>
            insert(
              schema.nodes.paragraph!.create({}, schema.text("Start typing…")),
            )
          }
        />
        <InsertButton
          icon="H"
          label="Heading"
          onClick={() =>
            insert(
              schema.nodes.heading!.create(
                { level: 2 },
                schema.text("Heading"),
              ),
            )
          }
        />
        <select
          className="piqae-variable-picker"
          aria-label="Insert Shopify variable"
          defaultValue=""
          onChange={(e) => {
            if (e.currentTarget.value) insertVariable(e.currentTarget.value);
            e.currentTarget.value = "";
          }}
        >
          <option value="">Insert variable…</option>
          {fieldGroups(authoringFields).map(([group, fields]) => (
            <optgroup label={group} key={group}>
              {fields.map((field) => (
                <option value={field.path} key={field.path}>
                  {field.label}
                </option>
              ))}
            </optgroup>
          ))}
        </select>
        <InsertButton
          icon="▤"
          label="Items"
          onClick={() => insertBlock(defaultTable())}
        />
        <InsertButton
          icon="◇"
          label="Condition"
          onClick={() => insertBlock(defaultConditional())}
        />
        <InsertButton
          icon="▦"
          label="QR"
          onClick={() =>
            insertBlock({
              type: "qr",
              value: pathExpression("order.statusUrl"),
              size_mm: 24,
            })
          }
        />
        <InsertButton
          icon="▥"
          label="Barcode"
          onClick={() =>
            insertBlock({
              type: "barcode",
              value: pathExpression("order.name"),
              symbology: "code128",
              width_mm: 48,
              height_mm: 16,
              human_readable: true,
            })
          }
        />
        <InsertButton
          icon="▧"
          label="Image"
          onClick={() =>
            insertBlock({
              type: "image",
              resource: "shop.logo",
              width_mm: 42,
              height_mm: 18,
              fit: "contain",
            })
          }
        />
        <InsertButton
          icon="─"
          label="Divider"
          onClick={() => insert(schema.nodes.divider!.create())}
        />
        <InsertButton
          icon="↕"
          label="Space"
          onClick={() => insertBlock({ type: "spacer", height_mm: 6 })}
        />
        <span className="piqae-toolbar-separator" />
        <InsertButton
          icon="▦"
          label="Columns"
          onClick={() => insertBlock(defaultGrid())}
        />
        <InsertButton
          icon="≡"
          label="Stack"
          onClick={() => insertBlock(defaultContainer("stack"))}
        />
        <InsertButton
          icon="↔"
          label="Row"
          onClick={() => insertBlock(defaultContainer("row"))}
        />
      </div>
      <div className="piqae-editor-workspace">
        <div className="piqae-canvas-wrap">
          <div className="piqae-canvas-context" aria-live="polite">
            {selection?.path ? (
              <>
                <strong>{blockTitle(selection.block)}</strong>
                <span>Selected on page</span>
                <button
                  type="button"
                  onClick={() => moveSelected(-1)}
                  disabled={disabled || selection.path.at(-1)?.index === 0}
                  aria-label="Move selected block up"
                  title="Move up"
                >
                  ↑
                </button>
                <button
                  type="button"
                  onClick={() => moveSelected(1)}
                  disabled={disabled}
                  aria-label="Move selected block down"
                  title="Move down"
                >
                  ↓
                </button>
                <button
                  type="button"
                  onClick={duplicateSelected}
                  disabled={disabled}
                >
                  Duplicate
                </button>
                <button
                  type="button"
                  className="piqae-danger"
                  onClick={removeSelected}
                  disabled={disabled}
                >
                  Delete
                </button>
              </>
            ) : (
              <span>Click content on the page to edit it</span>
            )}
          </div>
          <div className="piqae-page-sheet piqae-rendered-canvas">
            <DocumentCanvas
              blocks={value.body}
              selectedPath={selection?.path}
              editable={!disabled}
              authoringFields={authoringFields}
              onSelect={(block, path) =>
                setSelection({ position: -1, block, path })
              }
              onChange={(block, path) => {
                setSelection({ position: -1, block, path });
                const body = replaceBlockAtPath(value.body, path, block);
                latest.current = { ...latest.current, body };
                view.current?.updateState(
                  EditorState.create({ schema, doc: blocksToDoc(body) }),
                );
                onChange({ ...latest.current, body });
              }}
            />
          </div>
          <div
            className="piqae-prosemirror-source"
            ref={host}
            aria-hidden="true"
          />
        </div>
        <aside className="piqae-inspector" aria-label="Document block settings">
          <details open className="piqae-inspector-section">
            <summary>Brand &amp; styling</summary>
            <div className="piqae-block-properties">
              <NumberProperty
                label="Base text size (pt)"
                value={theme.font_size_pt ?? 10}
                min={7}
                max={24}
                disabled={disabled}
                onChange={(font_size_pt) =>
                  onChange({
                    ...latest.current,
                    theme: { ...latest.current.theme, font_size_pt },
                  })
                }
              />
              <label>
                Text colour
                <input
                  type="color"
                  value={rgbToHex(
                    theme.text_color ?? { red: 32, green: 34, blue: 35 },
                  )}
                  disabled={disabled}
                  onChange={(event) =>
                    onChange({
                      ...latest.current,
                      theme: {
                        ...latest.current.theme,
                        text_color: hexToRgb(event.currentTarget.value),
                      },
                    })
                  }
                />
              </label>
            </div>
          </details>
          <details className="piqae-inspector-section">
            <summary>Document options</summary>
            <div className="piqae-block-properties">
              <p className="piqae-muted">
                Page size and document type are set above. Content automatically
                reflows across pages.
              </p>
            </div>
          </details>
          <details className="piqae-inspector-section">
            <summary>Text &amp; language</summary>
            <div className="piqae-block-properties">
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
          </details>
          <div className="piqae-selected-inspector">
            {selection ? (
              <>
                <div className="piqae-inspector-heading">
                  <div>
                    <small>Selected</small>
                    <strong>{blockTitle(selection.block)}</strong>
                  </div>
                  <button
                    type="button"
                    className="piqae-icon-button piqae-danger"
                    onClick={removeSelected}
                    disabled={disabled}
                    aria-label="Delete selected block"
                  >
                    ✕
                  </button>
                </div>
                <BlockProperties
                  block={selection.block}
                  disabled={disabled}
                  authoringFields={authoringFields}
                  onChange={updateSelected}
                />
              </>
            ) : (
              <div className="piqae-inspector-empty">
                <span aria-hidden="true">↖</span>
                <strong>Select something on the page</strong>
                <p>Formatting and data options appear here.</p>
              </div>
            )}
          </div>
        </aside>
      </div>
    </div>
  );
}

export type BlockPathPart = {
  branch: "root" | "children" | "then" | "else";
  index: number;
};
export type BlockPath = BlockPathPart[];

export function BusinessDocumentPreview({
  value,
}: {
  value: BusinessDocument;
}) {
  return (
    <div className="piqae-preview-stage" aria-label="Rendered document preview">
      <div className="piqae-page-sheet piqae-rendered-canvas">
        <DocumentCanvas
          blocks={value.body}
          editable={false}
          onSelect={() => undefined}
          onChange={() => undefined}
        />
      </div>
    </div>
  );
}

function DocumentCanvas({
  blocks,
  path = [],
  branch = "root",
  editable = true,
  selectedPath,
  authoringFields = AUTHORING_FIELDS,
  onSelect,
  onChange,
}: {
  blocks: Block[];
  path?: BlockPath;
  branch?: BlockPathPart["branch"];
  editable?: boolean;
  selectedPath?: BlockPath;
  authoringFields?: readonly ShopifyDocumentField[];
  onSelect(block: Block, path: BlockPath): void;
  onChange(block: Block, path: BlockPath): void;
}) {
  return (
    <div className="piqae-document-flow">
      {blocks.map((block, index) => (
        <CanvasBlock
          key={`${path.map((part) => `${part.branch}-${part.index}`).join("/")}-${branch}-${index}`}
          block={block}
          path={[...path, { branch, index }]}
          editable={editable}
          selectedPath={selectedPath}
          selected={sameBlockPath(selectedPath, [...path, { branch, index }])}
          authoringFields={authoringFields}
          onSelect={onSelect}
          onChange={onChange}
        />
      ))}
    </div>
  );
}

function CanvasBlock({
  block,
  path,
  editable,
  selectedPath,
  selected,
  authoringFields,
  onSelect,
  onChange,
}: {
  block: Block;
  path: BlockPath;
  editable: boolean;
  selectedPath?: BlockPath;
  selected: boolean;
  authoringFields: readonly ShopifyDocumentField[];
  onSelect(block: Block, path: BlockPath): void;
  onChange(block: Block, path: BlockPath): void;
}) {
  const select = (event: React.MouseEvent) => {
    event.stopPropagation();
    onSelect(block, path);
  };
  if (block.type === "paragraph" || block.type === "heading") {
    const Tag =
      block.type === "heading" ? (`h${block.level ?? 2}` as "h1") : "p";
    return (
      <Tag
        className={`piqae-canvas-text${selected ? " piqae-canvas-selected" : ""}`}
        onClick={select}
        contentEditable={editable}
        suppressContentEditableWarning
        onBlur={(event) =>
          editable &&
          onChange(
            {
              ...block,
              content: parseEditableInline(
                event.currentTarget.textContent ?? "",
              ),
            },
            path,
          )
        }
      >
        {editableInline(block.content)}
      </Tag>
    );
  }
  if (block.type === "divider")
    return (
      <hr
        className={`piqae-canvas-selectable${selected ? " piqae-canvas-selected" : ""}`}
        onClick={select}
      />
    );
  if (block.type === "spacer")
    return (
      <div
        className={`piqae-canvas-spacer piqae-canvas-selectable${selected ? " piqae-canvas-selected" : ""}`}
        style={{ height: `${Math.max(8, block.height_mm * 2)}px` }}
        onClick={select}
      >
        <span>{block.height_mm} mm space</span>
      </div>
    );
  if (block.type === "page_break")
    return (
      <div
        className={
          selected
            ? "piqae-canvas-page-break piqae-canvas-selected"
            : "piqae-canvas-page-break"
        }
        onClick={select}
      >
        Page break
      </div>
    );
  if (block.type === "image")
    return (
      <div
        className={`piqae-canvas-image piqae-canvas-selectable${selected ? " piqae-canvas-selected" : ""}`}
        style={{
          width: `${Math.max(80, block.width_mm * 2)}px`,
          height: `${Math.max(40, block.height_mm * 2)}px`,
        }}
        onClick={select}
      >
        <span aria-hidden="true">▧</span>
        <small>{block.resource}</small>
      </div>
    );
  if (block.type === "qr" || block.type === "barcode")
    return (
      <div
        className={`piqae-canvas-code piqae-canvas-${block.type} piqae-canvas-selectable${selected ? " piqae-canvas-selected" : ""}`}
        onClick={select}
      >
        <span aria-hidden="true">{block.type === "qr" ? "▦" : "▌█▌▌█▌█"}</span>
        <small>{expressionLabel(block.value)}</small>
      </div>
    );
  if (block.type === "table")
    return (
      <div
        className={`piqae-canvas-table piqae-canvas-selectable${selected ? " piqae-canvas-selected" : ""}`}
        onClick={select}
      >
        <div className="piqae-canvas-table-row piqae-canvas-table-head">
          {block.columns.map((column, index) => (
            <span
              key={index}
              className="piqae-canvas-table-column"
              style={{ flex: column.width ?? 1, textAlign: column.align }}
              onClick={(event) => event.stopPropagation()}
            >
              <strong
                contentEditable={editable}
                suppressContentEditableWarning
                onBlur={(event) =>
                  editable &&
                  onChange(
                    {
                      ...block,
                      columns: block.columns.map((item, itemIndex) =>
                        itemIndex === index
                          ? {
                              ...item,
                              header: [
                                {
                                  type: "text" as const,
                                  value: event.currentTarget.textContent ?? "",
                                },
                              ],
                            }
                          : item,
                      ),
                    },
                    path,
                  )
                }
              >
                {inlineLabel(column.header)}
              </strong>
              {editable ? (
                <span className="piqae-canvas-column-actions">
                  <button
                    type="button"
                    disabled={index === 0}
                    aria-label={`Move ${inlineLabel(column.header)} left`}
                    onClick={() =>
                      onChange(
                        {
                          ...block,
                          columns: moveItem(block.columns, index, -1),
                        },
                        path,
                      )
                    }
                  >
                    ←
                  </button>
                  <button
                    type="button"
                    disabled={index === block.columns.length - 1}
                    aria-label={`Move ${inlineLabel(column.header)} right`}
                    onClick={() =>
                      onChange(
                        {
                          ...block,
                          columns: moveItem(block.columns, index, 1),
                        },
                        path,
                      )
                    }
                  >
                    →
                  </button>
                  <button
                    type="button"
                    disabled={block.columns.length === 1}
                    aria-label={`Remove ${inlineLabel(column.header)} column`}
                    onClick={() =>
                      onChange(
                        {
                          ...block,
                          columns: block.columns.filter((_, i) => i !== index),
                        },
                        path,
                      )
                    }
                  >
                    ×
                  </button>
                </span>
              ) : null}
            </span>
          ))}
        </div>
        <div className="piqae-canvas-table-row piqae-canvas-table-binding-row">
          {block.columns.map((column, index) => (
            <label
              key={index}
              style={{ flex: column.width ?? 1, textAlign: column.align }}
              onClick={(event) => event.stopPropagation()}
            >
              <select
                aria-label={`${inlineLabel(column.header)} value`}
                value={
                  columnCellPath(column.cell)
                    ? `item.${columnCellPath(column.cell)}`
                    : ""
                }
                disabled={!editable}
                onChange={(event) =>
                  onChange(
                    {
                      ...block,
                      columns: block.columns.map((item, itemIndex) =>
                        itemIndex === index
                          ? {
                              ...item,
                              cell: [
                                {
                                  type: "value" as const,
                                  value: currentPathExpression(
                                    event.currentTarget.value.replace(
                                      /^item\./,
                                      "",
                                    ),
                                  ),
                                },
                              ],
                            }
                          : item,
                      ),
                    },
                    path,
                  )
                }
              >
                {!columnCellPath(column.cell) ? (
                  <option value="">Computed value</option>
                ) : null}
                {authoringFields
                  .filter((field) => field.path.startsWith("item."))
                  .map((field) => (
                    <option value={field.path} key={field.path}>
                      {field.label}
                    </option>
                  ))}
              </select>
            </label>
          ))}
        </div>
        {editable ? (
          <button
            className="piqae-canvas-add-column"
            type="button"
            onClick={(event) => {
              event.stopPropagation();
              onChange(
                { ...block, columns: [...block.columns, defaultColumn()] },
                path,
              );
            }}
          >
            + Add column
          </button>
        ) : null}
      </div>
    );
  if (block.type === "conditional")
    return (
      <section
        className={`piqae-canvas-conditional piqae-canvas-selectable${selected ? " piqae-canvas-selected" : ""}`}
        onClick={select}
      >
        <small>Shown when {expressionLabel(block.condition)}</small>
        <DocumentCanvas
          blocks={block.then}
          path={path}
          branch="then"
          editable={editable}
          selectedPath={selectedPath}
          authoringFields={authoringFields}
          onSelect={onSelect}
          onChange={onChange}
        />
        {block.else?.length ? (
          <details>
            <summary>Otherwise</summary>
            <DocumentCanvas
              blocks={block.else}
              path={path}
              branch="else"
              editable={editable}
              selectedPath={selectedPath}
              authoringFields={authoringFields}
              onSelect={onSelect}
              onChange={onChange}
            />
          </details>
        ) : null}
      </section>
    );
  const children = "children" in block ? block.children : [];
  const className =
    block.type === "grid"
      ? "piqae-canvas-grid"
      : block.type === "row"
        ? "piqae-canvas-row"
        : "piqae-canvas-stack";
  const style =
    block.type === "grid"
      ? {
          gridTemplateColumns: block.columns
            .map((column) => `${column}fr`)
            .join(" "),
          gap: `${block.gap_mm ?? 0}mm`,
        }
      : { gap: `${"gap_mm" in block ? (block.gap_mm ?? 0) : 0}mm` };
  return (
    <section
      className={`${className} piqae-canvas-selectable${selected ? " piqae-canvas-selected" : ""}`}
      style={style}
      onClick={select}
    >
      <DocumentCanvas
        blocks={children}
        path={path}
        branch="children"
        editable={editable}
        selectedPath={selectedPath}
        authoringFields={authoringFields}
        onSelect={onSelect}
        onChange={onChange}
      />
    </section>
  );
}

function InsertButton({
  icon,
  label,
  onClick,
}: {
  icon: string;
  label: string;
  onClick(): void;
}) {
  return (
    <button
      className="piqae-insert-button"
      type="button"
      aria-label={`Insert ${label.toLowerCase()}`}
      title={`Insert ${label.toLowerCase()}`}
      onClick={onClick}
    >
      <span aria-hidden="true">{icon}</span>
      <small>{label}</small>
    </button>
  );
}

function BlockProperties({
  block,
  disabled,
  authoringFields,
  onChange,
}: {
  block: Block;
  disabled?: boolean;
  authoringFields: readonly ShopifyDocumentField[];
  onChange(block: Block): void;
}) {
  if (block.type === "table")
    return (
      <div className="piqae-block-properties">
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
        <p className="piqae-property-help">
          Edit headings and choose line-item fields directly in the table.
        </p>
      </div>
    );
  if (block.type === "conditional")
    return (
      <div className="piqae-block-properties">
        <label>
          Show this section when
          <select
            value={expressionLabel(block.condition)}
            disabled={disabled}
            onChange={(event) =>
              onChange({
                ...block,
                condition: pathExpression(event.currentTarget.value),
              })
            }
          >
            {authoringFields.some(
              (field) =>
                field.conditionable &&
                field.path === expressionLabel(block.condition),
            ) ? null : (
              <option value={expressionLabel(block.condition)}>
                {expressionLabel(block.condition)}
              </option>
            )}
            {fieldGroups(
              authoringFields.filter((field) => field.conditionable),
            ).map(([group, fields]) => (
              <optgroup label={group} key={group}>
                {fields.map((field) => (
                  <option value={field.path} key={field.path}>
                    {field.label}
                  </option>
                ))}
              </optgroup>
            ))}
          </select>
        </label>
        <label>
          Advanced data path
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
        <NestedContentEditor
          label="When matched"
          blocks={block.then}
          disabled={disabled}
          authoringFields={authoringFields}
          onChange={(then) => onChange({ ...block, then })}
        />
        <NestedContentEditor
          label="Otherwise (optional)"
          blocks={block.else ?? []}
          disabled={disabled}
          authoringFields={authoringFields}
          onChange={(otherwise) => onChange({ ...block, else: otherwise })}
        />
      </div>
    );
  if (block.type === "grid")
    return (
      <div className="piqae-block-properties">
        <label>
          Column widths
          <input
            value={block.columns.join(", ")}
            disabled={disabled}
            onChange={(event) => {
              const columns = event.currentTarget.value
                .split(",")
                .map(Number)
                .filter((v) => Number.isFinite(v) && v > 0);
              if (columns.length) onChange({ ...block, columns });
            }}
          />
        </label>
        <NumberProperty
          label="Gap between columns (mm)"
          value={block.gap_mm ?? 0}
          min={0}
          max={40}
          disabled={disabled}
          onChange={(gap_mm) => onChange({ ...block, gap_mm })}
        />
        <NestedContentEditor
          label="Column content"
          blocks={block.children}
          disabled={disabled}
          authoringFields={authoringFields}
          onChange={(children) => onChange({ ...block, children })}
        />
      </div>
    );
  if (
    block.type === "stack" ||
    block.type === "row" ||
    block.type === "section" ||
    block.type === "keep_together"
  )
    return (
      <div className="piqae-block-properties">
        {"gap_mm" in block ? (
          <NumberProperty
            label="Gap (mm)"
            value={block.gap_mm ?? 0}
            min={0}
            max={40}
            disabled={disabled}
            onChange={(gap_mm) => onChange({ ...block, gap_mm })}
          />
        ) : null}
        <NestedContentEditor
          label="Content"
          blocks={block.children}
          disabled={disabled}
          authoringFields={authoringFields}
          onChange={(children) => onChange({ ...block, children })}
        />
      </div>
    );
  if (block.type === "repeat")
    return (
      <div className="piqae-block-properties">
        <label>
          Repeat data from
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
        <NumberProperty
          label="Gap (mm)"
          value={block.gap_mm ?? 0}
          min={0}
          max={40}
          disabled={disabled}
          onChange={(gap_mm) => onChange({ ...block, gap_mm })}
        />
        <NestedContentEditor
          label="Repeated content"
          blocks={block.children}
          disabled={disabled}
          authoringFields={authoringFields}
          onChange={(children) => onChange({ ...block, children })}
        />
      </div>
    );
  if (block.type === "image")
    return (
      <div className="piqae-block-properties">
        <label>
          Image resource
          <input
            value={block.resource}
            disabled={disabled}
            onChange={(event) =>
              onChange({ ...block, resource: event.currentTarget.value })
            }
          />
        </label>
        <div className="piqae-property-grid">
          <NumberProperty
            label="Width (mm)"
            value={block.width_mm}
            min={1}
            max={210}
            disabled={disabled}
            onChange={(width_mm) => onChange({ ...block, width_mm })}
          />
          <NumberProperty
            label="Height (mm)"
            value={block.height_mm}
            min={1}
            max={297}
            disabled={disabled}
            onChange={(height_mm) => onChange({ ...block, height_mm })}
          />
        </div>
        <label>
          Fit
          <select
            value={block.fit ?? "contain"}
            disabled={disabled}
            onChange={(event) =>
              onChange({
                ...block,
                fit: event.currentTarget.value as
                  | "contain"
                  | "fill"
                  | "scale_down",
              })
            }
          >
            <option value="contain">Fit inside</option>
            <option value="fill">Fill frame</option>
            <option value="scale_down">Only scale down</option>
          </select>
        </label>
      </div>
    );
  if (block.type === "spacer")
    return (
      <div className="piqae-block-properties">
        <NumberProperty
          label="Height (mm)"
          value={block.height_mm}
          min={1}
          max={100}
          disabled={disabled}
          onChange={(height_mm) => onChange({ ...block, height_mm })}
        />
      </div>
    );
  if (block.type === "qr")
    return (
      <div className="piqae-block-properties">
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
        <label>
          Error correction
          <select
            value={block.error_correction ?? "M"}
            disabled={disabled}
            onChange={(event) =>
              onChange({
                ...block,
                error_correction: event.currentTarget.value as
                  | "L"
                  | "M"
                  | "Q"
                  | "H",
              })
            }
          >
            <option>L</option>
            <option>M</option>
            <option>Q</option>
            <option>H</option>
          </select>
        </label>
      </div>
    );
  if (block.type === "barcode")
    return (
      <div className="piqae-block-properties">
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
        <label className="piqae-checkbox">
          <input
            type="checkbox"
            checked={block.human_readable ?? false}
            disabled={disabled}
            onChange={(event) =>
              onChange({
                ...block,
                human_readable: event.currentTarget.checked,
              })
            }
          />{" "}
          Show value below barcode
        </label>
      </div>
    );
  return null;
}

function NumberProperty({
  label,
  value,
  min,
  max,
  disabled,
  onChange,
}: {
  label: string;
  value: number;
  min: number;
  max: number;
  disabled?: boolean;
  onChange(value: number): void;
}) {
  return (
    <label>
      {label}
      <input
        type="number"
        value={value}
        min={min}
        max={max}
        disabled={disabled}
        onChange={(event) => onChange(Number(event.currentTarget.value))}
      />
    </label>
  );
}

function NestedContentEditor({
  label,
  blocks,
  disabled,
  authoringFields,
  onChange,
}: {
  label: string;
  blocks: Block[];
  disabled?: boolean;
  authoringFields: readonly ShopifyDocumentField[];
  onChange(blocks: Block[]): void;
}) {
  return (
    <fieldset className="piqae-nested-editor">
      <legend>{label}</legend>
      {blocks.length === 0 ? (
        <p className="piqae-muted">No content. This area will be omitted.</p>
      ) : null}
      {blocks.map((child, index) => (
        <div className="piqae-nested-row" key={index}>
          <span className="piqae-block-kind">{blockTitle(child)}</span>
          {child.type === "paragraph" || child.type === "heading" ? (
            <input
              aria-label={`${label} block ${index + 1}`}
              value={editableInline(child.content)}
              disabled={disabled}
              onChange={(event) =>
                onChange(
                  blocks.map((item, i) =>
                    i === index
                      ? {
                          ...child,
                          content: parseEditableInline(
                            event.currentTarget.value,
                            child.content,
                          ),
                        }
                      : item,
                  ),
                )
              }
            />
          ) : (
            <details className="piqae-nested-settings">
              <summary>Edit {blockTitle(child).toLowerCase()}</summary>
              <BlockProperties
                block={child}
                disabled={disabled}
                authoringFields={authoringFields}
                onChange={(nextChild) =>
                  onChange(
                    blocks.map((item, itemIndex) =>
                      itemIndex === index ? nextChild : item,
                    ),
                  )
                }
              />
            </details>
          )}
          <div className="piqae-inline-actions">
            <button
              type="button"
              disabled={disabled || index === 0}
              onClick={() => onChange(moveItem(blocks, index, -1))}
              aria-label={`Move ${label} block ${index + 1} up`}
            >
              ↑
            </button>
            <button
              type="button"
              disabled={disabled || index === blocks.length - 1}
              onClick={() => onChange(moveItem(blocks, index, 1))}
              aria-label={`Move ${label} block ${index + 1} down`}
            >
              ↓
            </button>
            <button
              type="button"
              disabled={disabled}
              onClick={() => onChange(blocks.filter((_, i) => i !== index))}
            >
              Remove
            </button>
          </div>
        </div>
      ))}
      <div className="piqae-add-nested">
        <button
          type="button"
          disabled={disabled}
          onClick={() =>
            onChange([
              ...blocks,
              { type: "paragraph", content: [{ type: "text", value: "Text" }] },
            ])
          }
        >
          + Text
        </button>
        <button
          type="button"
          disabled={disabled}
          onClick={() =>
            onChange([
              ...blocks,
              {
                type: "paragraph",
                content: [
                  { type: "value", value: pathExpression("order.name") },
                ],
              },
            ])
          }
        >
          + Data
        </button>
        <button
          type="button"
          disabled={disabled}
          onClick={() => onChange([...blocks, { type: "divider" }])}
        >
          + Divider
        </button>
      </div>
    </fieldset>
  );
}

function editableInline(content: Inline[]) {
  return content
    .map((item) =>
      item.type === "text"
        ? item.value
        : item.type === "line_break"
          ? "\n"
          : `{{ ${expressionLabel(item.value)} }}`,
    )
    .join("");
}
function parseEditableInline(
  source: string,
  original: Inline[] = [],
): Inline[] {
  const content: Inline[] = [];
  const originalValues = original.filter(
    (item): item is Extract<Inline, { type: "value" }> => item.type === "value",
  );
  let valueIndex = 0;
  const pattern = /\{\{\s*([^}]+?)\s*\}\}|\n/g;
  let cursor = 0;
  for (const match of source.matchAll(pattern)) {
    if ((match.index ?? 0) > cursor)
      content.push({ type: "text", value: source.slice(cursor, match.index) });
    if (match[1]) {
      const previous = originalValues[valueIndex++];
      content.push({
        type: "value",
        value:
          previous && expressionLabel(previous.value) === match[1]
            ? previous.value
            : pathExpression(match[1]),
      });
    } else content.push({ type: "line_break" });
    cursor = (match.index ?? 0) + match[0].length;
  }
  if (cursor < source.length)
    content.push({ type: "text", value: source.slice(cursor) });
  return content.length ? content : [{ type: "text", value: "" }];
}
function moveItem<T>(items: T[], index: number, direction: -1 | 1) {
  const target = index + direction;
  if (target < 0 || target >= items.length) return items;
  const next = [...items];
  [next[index], next[target]] = [next[target]!, next[index]!];
  return next;
}
function sameBlockPath(left?: BlockPath, right?: BlockPath) {
  return Boolean(
    left &&
    right &&
    left.length === right.length &&
    left.every(
      (part, index) =>
        part.branch === right[index]?.branch &&
        part.index === right[index]?.index,
    ),
  );
}
export function moveBlockAtPath(
  blocks: Block[],
  path: BlockPath,
  direction: -1 | 1,
): Block[] {
  const [part, ...rest] = path;
  if (!part) return blocks;
  if (!rest.length) return moveItem(blocks, part.index, direction);
  return blocks.map((block, index) => {
    if (index !== part.index) return block;
    const nextPart = rest[0]!;
    if (nextPart.branch === "children" && "children" in block)
      return {
        ...block,
        children: moveBlockAtPath(block.children, rest, direction),
      };
    if (nextPart.branch === "then" && block.type === "conditional")
      return {
        ...block,
        then: moveBlockAtPath(block.then, rest, direction),
      };
    if (nextPart.branch === "else" && block.type === "conditional")
      return {
        ...block,
        else: moveBlockAtPath(block.else ?? [], rest, direction),
      };
    return block;
  });
}
export function replaceBlockAtPath(
  blocks: Block[],
  path: BlockPath,
  replacement: Block,
): Block[] {
  const [part, ...rest] = path;
  if (!part) return blocks;
  return blocks.map((block, index) => {
    if (index !== part.index) return block;
    if (!rest.length) return replacement;
    const nextPart = rest[0]!;
    if (nextPart.branch === "children" && "children" in block)
      return {
        ...block,
        children: replaceBlockAtPath(block.children, rest, replacement),
      };
    if (nextPart.branch === "then" && block.type === "conditional")
      return {
        ...block,
        then: replaceBlockAtPath(block.then, rest, replacement),
      };
    if (nextPart.branch === "else" && block.type === "conditional")
      return {
        ...block,
        else: replaceBlockAtPath(block.else ?? [], rest, replacement),
      };
    return block;
  });
}
export function insertBlockAfterPath(
  blocks: Block[],
  path: BlockPath,
  inserted: Block,
): Block[] {
  const [part, ...rest] = path;
  if (!part) return [...blocks, inserted];
  if (!rest.length) {
    const next = [...blocks];
    next.splice(Math.min(part.index + 1, next.length), 0, inserted);
    return next;
  }
  return blocks.map((block, index) => {
    if (index !== part.index) return block;
    const nextPart = rest[0]!;
    if (nextPart.branch === "children" && "children" in block)
      return {
        ...block,
        children: insertBlockAfterPath(block.children, rest, inserted),
      };
    if (nextPart.branch === "then" && block.type === "conditional")
      return {
        ...block,
        then: insertBlockAfterPath(block.then, rest, inserted),
      };
    if (nextPart.branch === "else" && block.type === "conditional")
      return {
        ...block,
        else: insertBlockAfterPath(block.else ?? [], rest, inserted),
      };
    return block;
  });
}
export function removeBlockAtPath(blocks: Block[], path: BlockPath): Block[] {
  const [part, ...rest] = path;
  if (!part) return blocks;
  if (!rest.length) return blocks.filter((_, index) => index !== part.index);
  return blocks.map((block, index) => {
    if (index !== part.index) return block;
    const nextPart = rest[0]!;
    if (nextPart.branch === "children" && "children" in block)
      return { ...block, children: removeBlockAtPath(block.children, rest) };
    if (nextPart.branch === "then" && block.type === "conditional")
      return { ...block, then: removeBlockAtPath(block.then, rest) };
    if (nextPart.branch === "else" && block.type === "conditional")
      return { ...block, else: removeBlockAtPath(block.else ?? [], rest) };
    return block;
  });
}
function rgbToHex(color: { red: number; green: number; blue: number }) {
  return `#${[color.red, color.green, color.blue].map((value) => Math.max(0, Math.min(255, value)).toString(16).padStart(2, "0")).join("")}`;
}
function hexToRgb(value: string) {
  return {
    red: Number.parseInt(value.slice(1, 3), 16),
    green: Number.parseInt(value.slice(3, 5), 16),
    blue: Number.parseInt(value.slice(5, 7), 16),
  };
}
function fieldGroups(fields: readonly ShopifyDocumentField[]) {
  const groups = new Map<
    ShopifyDocumentField["group"],
    ShopifyDocumentField[]
  >();
  for (const field of fields) {
    const items = groups.get(field.group) ?? [];
    items.push(field);
    groups.set(field.group, items);
  }
  return [...groups.entries()];
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
function structuredBlockDOM(block: Block, fallback?: string): DOMOutputSpec {
  const attrs = {
    class: `piqae-structured-block piqae-block-${block.type}`,
    "data-block-type": block.type,
  };
  if (block.type === "table")
    return [
      "div",
      attrs,
      [
        "div",
        { class: "piqae-block-label" },
        "Line items",
        ["span", {}, expressionLabel(block.items)],
      ],
      [
        "div",
        { class: "piqae-table-preview" },
        ...block.columns.map((column) => [
          "span",
          {
            style: `flex:${column.width ?? 1};text-align:${column.align ?? "left"}`,
          },
          inlineLabel(column.header),
        ]),
      ],
    ];
  if (block.type === "conditional")
    return [
      "div",
      attrs,
      [
        "div",
        { class: "piqae-block-label" },
        "Conditional section",
        ["span", {}, expressionLabel(block.condition)],
      ],
      [
        "div",
        { class: "piqae-branch-preview" },
        `When matched · ${block.then.length} block${block.then.length === 1 ? "" : "s"}`,
      ],
      [
        "div",
        { class: "piqae-branch-preview piqae-else-preview" },
        `Otherwise · ${block.else?.length ?? 0} blocks`,
      ],
    ];
  if (block.type === "grid")
    return [
      "div",
      attrs,
      [
        "div",
        { class: "piqae-block-label" },
        `${block.columns.length} column layout`,
      ],
      [
        "div",
        { class: "piqae-grid-preview" },
        ...block.columns.map((width, index) => [
          "span",
          { style: `flex:${width}` },
          `Column ${index + 1}`,
        ]),
      ],
    ];
  if (
    block.type === "stack" ||
    block.type === "row" ||
    block.type === "section" ||
    block.type === "keep_together"
  )
    return [
      "div",
      attrs,
      ["div", { class: "piqae-block-label" }, blockTitle(block)],
      [
        "div",
        { class: `piqae-container-preview piqae-container-${block.type}` },
        ...block.children
          .slice(0, 6)
          .map((child) => ["span", {}, blockTitle(child)]),
      ],
    ];
  if (block.type === "repeat")
    return [
      "div",
      attrs,
      [
        "div",
        { class: "piqae-block-label" },
        "Repeating content",
        ["span", {}, expressionLabel(block.items)],
      ],
      [
        "div",
        { class: "piqae-branch-preview" },
        `${block.children.length} nested blocks`,
      ],
    ];
  if (block.type === "qr" || block.type === "barcode")
    return [
      "div",
      attrs,
      [
        "span",
        { class: "piqae-machine-placeholder", "aria-hidden": "true" },
        block.type === "qr" ? "▦" : "▌█▌▌█",
      ],
      ["span", {}, blockTitle(block)],
      ["small", {}, expressionLabel(block.value)],
    ];
  if (block.type === "image")
    return [
      "div",
      attrs,
      [
        "span",
        { class: "piqae-image-placeholder", "aria-hidden": "true" },
        "▧",
      ],
      ["span", {}, "Image"],
      ["small", {}, block.resource],
    ];
  if (block.type === "spacer")
    return [
      "div",
      attrs,
      ["span", {}, `Vertical space · ${block.height_mm} mm`],
    ];
  return ["div", attrs, fallback ?? blockTitle(block)];
}
function blockTitle(block: Block) {
  const labels: Partial<Record<Block["type"], string>> = {
    table: "Line-item table",
    conditional: "Conditional section",
    grid: "Columns",
    stack: "Vertical stack",
    row: "Horizontal row",
    section: "Section",
    keep_together: "Keep together",
    repeat: "Repeating content",
    qr: "QR code",
    barcode: "Barcode",
    image: "Image",
    spacer: "Spacing",
    page_break: "Page break",
    divider: "Divider",
    paragraph: "Text",
    heading: "Heading",
  };
  return labels[block.type] ?? block.type.replaceAll("_", " ");
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
function currentPathExpression(value: string): Expression {
  return {
    type: "current_path",
    path: value
      .split(".")
      .map((part) => part.trim())
      .filter(Boolean),
  };
}
function columnCellPath(content: Inline[]) {
  const value = content.find((item) => item.type === "value");
  return value?.type === "value" && value.value.type === "current_path"
    ? value.value.path.join(".")
    : "";
}
function inlineLabel(content: Inline[]) {
  return content
    .map((item) => (item.type === "text" ? item.value : "Value"))
    .join("");
}
function blockNode(
  type:
    | "table_block"
    | "repeat_block"
    | "conditional_block"
    | "machine_code"
    | "structured_block",
  value: Block,
) {
  return schema.nodes[type]!.create({ json: JSON.stringify(value) });
}
function nodeTypeForBlock(
  block: Block,
):
  | "table_block"
  | "repeat_block"
  | "conditional_block"
  | "machine_code"
  | "structured_block" {
  if (block.type === "table") return "table_block";
  if (block.type === "repeat") return "repeat_block";
  if (block.type === "conditional") return "conditional_block";
  if (block.type === "qr" || block.type === "barcode") return "machine_code";
  return "structured_block";
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
function defaultColumn(): Extract<Block, { type: "table" }>["columns"][number] {
  return {
    header: [{ type: "text", value: "Column" }],
    cell: [{ type: "value", value: currentPathExpression("title") }],
    width: 2,
    align: "left",
  };
}
function defaultConditional(): Block {
  return {
    type: "conditional",
    condition: pathExpression("order.note"),
    then: [
      {
        type: "paragraph",
        content: [{ type: "value", value: pathExpression("order.note") }],
      },
    ],
    else: [],
  };
}
function defaultGrid(): Block {
  return {
    type: "grid",
    columns: [1, 1],
    gap_mm: 8,
    children: [
      { type: "paragraph", content: [{ type: "text", value: "Left column" }] },
      { type: "paragraph", content: [{ type: "text", value: "Right column" }] },
    ],
  };
}
function defaultContainer(type: "stack" | "row"): Block {
  return {
    type,
    gap_mm: 4,
    children: [
      {
        type: "paragraph",
        content: [
          {
            type: "text",
            value: type === "row" ? "First item" : "First block",
          },
        ],
      },
      {
        type: "paragraph",
        content: [
          {
            type: "text",
            value: type === "row" ? "Second item" : "Second block",
          },
        ],
      },
    ],
  };
}
