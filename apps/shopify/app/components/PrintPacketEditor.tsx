import {
  Fragment,
  useEffect,
  useId,
  useRef,
  useState,
  type CSSProperties,
  type ReactNode,
} from "react";
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
  PrintPacket,
  Expression,
  Inline,
  TextStyle,
} from "../core/template-model";
import {
  SHOPIFY_DOCUMENT_FIELDS,
  type ShopifyDocumentField,
} from "../core/shopify-document-fields";
import {
  authoringPathExpression,
  canonicalizeShopifyEditorBody,
  isLineItemsExpression,
  type ShopifyAuthoringScope,
} from "../core/shopify-editor-scopes";
import { documentHasPageBreak } from "../core/template-model";
import type { ShopifyPrintTarget } from "../core/shopify-print-targets";

type DesignStock = ShopifyPrintTarget["stock"];

const PIQAE_BLOCK_DRAG_TYPE = "application/x-piqae-printpacket-block";
const QUICK_INSERT_TYPES = [
  "paragraph",
  "heading",
  "image",
  "divider",
  "spacer",
] as const;
type QuickInsertType = (typeof QUICK_INSERT_TYPES)[number];

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
      attrs: { level: { default: 2 }, align: { default: "left" } },
      toDOM: (n) => [`h${n.attrs.level}`, { "data-align": n.attrs.align }, 0],
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

const AUTHORING_FIELDS: readonly ShopifyDocumentField[] =
  SHOPIFY_DOCUMENT_FIELDS;
export const SHOPIFY_VARIABLES = AUTHORING_FIELDS.map((field) => field.path);

export function PrintPacketEditor({
  value,
  disabled,
  customFields = [],
  stock = null,
  workspaceControls,
  onChange,
}: {
  value: PrintPacket;
  disabled?: boolean;
  customFields?: readonly ShopifyDocumentField[];
  stock?: DesignStock;
  workspaceControls?: ReactNode;
  onChange(document: PrintPacket): void;
}) {
  const allAuthoringFields = [...AUTHORING_FIELDS, ...customFields];
  const canonicalBody = canonicalizeShopifyEditorBody(value.body);
  const continuousPageBreaks =
    value.media.kind !== "paged" && documentHasPageBreak(value);
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView | null>(null);
  const latest = useRef(value);
  const [selection, setSelection] = useState<{
    position: number;
    block: Block;
    path?: BlockPath;
  } | null>(null);
  const insertionScope = selection?.path
    ? scopeForBlockPath(canonicalBody, selection.path)
    : "order";
  const insertionFields = contextualFieldSuggestions(
    allAuthoringFields,
    insertionScope,
  );
  latest.current = { ...value, body: canonicalBody };
  useEffect(() => {
    if (!host.current) return;
    const state = EditorState.create({
      schema,
      doc: blocksToDoc(canonicalBody),
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
        const body = canonicalizeShopifyEditorBody(docToBlocks(next.doc));
        latest.current = { ...latest.current, body };
        onChange(latest.current);
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
    const authoredBody = selection?.path
      ? insertBlockAfterPath(latest.current.body, selection.path, inserted)
      : [...latest.current.body, inserted];
    const body = canonicalizeShopifyEditorBody(authoredBody);
    const nextDocument = { ...latest.current, body };
    latest.current = nextDocument;
    instance.updateState(
      EditorState.create({ schema, doc: blocksToDoc(body) }),
    );
    onChange(nextDocument);
    instance.focus();
  };
  const insertAtPath = (block: Block, path: BlockPath) => {
    const body = canonicalizeShopifyEditorBody(
      insertBlockAtPath(latest.current.body, path, block),
    );
    const nextDocument = { ...latest.current, body };
    latest.current = nextDocument;
    view.current?.updateState(
      EditorState.create({ schema, doc: blocksToDoc(body) }),
    );
    setSelection({ position: -1, path, block });
    onChange(nextDocument);
  };
  const insertVariable = (path: string) =>
    insert(
      schema.nodes.variable!.create({
        expression: JSON.stringify(
          authoringPathExpression(path, insertionScope),
        ),
        label: path,
      }),
    );
  const updateSelected = (block: Block) => {
    const instance = view.current;
    if (!instance || !selection) return;
    if (selection.path) {
      const body = canonicalizeShopifyEditorBody(
        replaceBlockAtPath(latest.current.body, selection.path, block),
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
  const insertBlock = (block: Block) =>
    insert(blockNode(nodeTypeForBlock(block), block));
  const removeSelected = () => {
    const instance = view.current;
    if (!instance || !selection) return;
    if (selection.path) {
      const body = canonicalizeShopifyEditorBody(
        removeBlockAtPath(latest.current.body, selection.path),
      );
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
    const body = canonicalizeShopifyEditorBody(
      moveBlockAtPath(latest.current.body, selection.path, direction),
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
    const body = canonicalizeShopifyEditorBody(
      insertBlockAfterPath(
        latest.current.body,
        selection.path,
        structuredClone(selection.block),
      ),
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
      <div className="piqae-editor-toolbar">
        <div className="piqae-editor-toolbar-primary">
          {workspaceControls ? (
            <div className="piqae-editor-toolbar-workspaces">
              {workspaceControls}
            </div>
          ) : null}
          {workspaceControls ? <span className="piqae-tool-divider" /> : null}
          <div
            className="piqae-tool-rail"
            role="toolbar"
            aria-label="Insert into document"
          >
            <div className="piqae-tool-group">
              <ToolButton
                icon="text"
                label="Text"
                disabled={disabled}
                dragBlock={() => quickInsertBlock("paragraph")}
                onClick={() =>
                  insert(
                    schema.nodes.paragraph!.create(
                      {},
                      schema.text("Start typing…"),
                    ),
                  )
                }
              />
              <ToolButton
                icon="heading"
                label="Heading"
                disabled={disabled}
                dragBlock={() => quickInsertBlock("heading")}
                onClick={() =>
                  insert(
                    schema.nodes.heading!.create(
                      { level: 2 },
                      schema.text("Heading"),
                    ),
                  )
                }
              />
              <InsertDataButton
                fields={insertionFields}
                disabled={disabled}
                onInsert={insertVariable}
              />
            </div>
            <span className="piqae-tool-divider" />
            <div className="piqae-tool-group">
              <ToolButton
                icon="table"
                label="Line items"
                disabled={disabled}
                dragBlock={defaultTable}
                onClick={() => insertBlock(defaultTable())}
              />
              <ToolButton
                icon="repeat"
                label="Repeating content"
                disabled={disabled}
                dragBlock={defaultRepeat}
                onClick={() => insertBlock(defaultRepeat())}
              />
              <ToolButton
                icon="condition"
                label="Conditional section"
                disabled={disabled}
                dragBlock={defaultConditional}
                onClick={() => insertBlock(defaultConditional())}
              />
            </div>
            <span className="piqae-tool-divider" />
            <div className="piqae-tool-group">
              <ToolButton
                icon="image"
                label="Image"
                disabled={disabled}
                dragBlock={() => quickInsertBlock("image")}
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
              <ToolButton
                icon="qr"
                label="QR code"
                disabled={disabled}
                dragBlock={defaultQrCode}
                onClick={() => insertBlock(defaultQrCode())}
              />
              <ToolButton
                icon="barcode"
                label="Barcode"
                disabled={disabled}
                dragBlock={defaultBarcode}
                onClick={() => insertBlock(defaultBarcode())}
              />
            </div>
            <span className="piqae-tool-divider" />
            <div className="piqae-tool-group">
              <ToolButton
                icon="columns"
                label="Columns"
                disabled={disabled}
                dragBlock={defaultGrid}
                onClick={() => insertBlock(defaultGrid())}
              />
              <ToolButton
                icon="stack"
                label="Stack"
                disabled={disabled}
                dragBlock={() => defaultContainer("stack")}
                onClick={() => insertBlock(defaultContainer("stack"))}
              />
              <ToolButton
                icon="row"
                label="Row"
                disabled={disabled}
                dragBlock={() => defaultContainer("row")}
                onClick={() => insertBlock(defaultContainer("row"))}
              />
              <ToolButton
                icon="divider"
                label="Divider"
                disabled={disabled}
                dragBlock={() => quickInsertBlock("divider")}
                onClick={() => insert(schema.nodes.divider!.create())}
              />
              <ToolButton
                icon="spacer"
                label="Spacing"
                disabled={disabled}
                dragBlock={() => quickInsertBlock("spacer")}
                onClick={() => insertBlock({ type: "spacer", height_mm: 6 })}
              />
            </div>
          </div>
        </div>
        {selection ? (
          <div className="piqae-selection-rail" aria-live="polite">
            <div className="piqae-selection-bar">
              <span className="piqae-selection-title">
                <Icon name={blockIcon(selection.block)} />
                {blockTitle(selection.block)}
              </span>
              <SelectionSettings
                block={selection.block}
                disabled={disabled}
                authoringFields={insertionFields}
                scope={insertionScope}
                onChange={updateSelected}
              />
              <span className="piqae-selection-spacer" />
              <span className="piqae-selection-actions">
                <ToolButton
                  icon="up"
                  label="Move up"
                  disabled={
                    disabled ||
                    !selection.path ||
                    selection.path.at(-1)?.index === 0
                  }
                  onClick={() => moveSelected(-1)}
                />
                <ToolButton
                  icon="down"
                  label="Move down"
                  disabled={
                    disabled ||
                    !selection.path ||
                    selection.path.at(-1)?.index ===
                      siblingsAtPath(value.body, selection.path).length - 1
                  }
                  onClick={() => moveSelected(1)}
                />
                <ToolButton
                  icon="duplicate"
                  label="Duplicate"
                  disabled={disabled || !selection.path}
                  onClick={duplicateSelected}
                />
                <ToolButton
                  icon="trash"
                  label="Delete"
                  tone="critical"
                  disabled={disabled}
                  onClick={removeSelected}
                />
              </span>
            </div>
          </div>
        ) : null}
      </div>
      <div className="piqae-editor-workspace">
        {continuousPageBreaks ? (
          <div className="piqae-media-diagnostic" role="alert">
            Page breaks are not supported on {value.media.kind} media. Remove
            them before publishing.
          </div>
        ) : null}
        <MediaRuler value={value} stock={stock} />
        <div className="piqae-canvas-wrap">
          <div
            className={`piqae-page-sheet piqae-rendered-canvas piqae-media-${value.media.kind}`}
            style={canvasStyle(value)}
            onKeyDown={(event) => {
              if (
                disabled ||
                !selection ||
                (event.key !== "Delete" && event.key !== "Backspace") ||
                isEditingTarget(event.target)
              )
                return;
              event.preventDefault();
              removeSelected();
            }}
          >
            <SafeAreaGuide value={value} stock={stock} />
            <DocumentCanvas
              blocks={canonicalBody}
              insertionSlots={false}
              selectedPath={selection?.path}
              editable={!disabled}
              mediaKind={value.media.kind}
              authoringFields={allAuthoringFields}
              onSelect={(block, path) =>
                setSelection({ position: -1, block, path })
              }
              onInsert={insertAtPath}
              onChange={(block, path) => {
                setSelection({ position: -1, block, path });
                const body = canonicalizeShopifyEditorBody(
                  replaceBlockAtPath(canonicalBody, path, block),
                );
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
      </div>
    </div>
  );
}

export type BlockPathPart = {
  branch: "root" | "children" | "then" | "else";
  index: number;
};
export type BlockPath = BlockPathPart[];

export type OrderBatchPresentation =
  | "one_order_per_page"
  | "flowing_pages"
  | "continuous"
  | "fixed_media";

/**
 * Root `orders` repeats are document batching, not ordinary merchant content.
 * Keep the source open and editable in Code while presenting its actual page
 * behavior plainly in Design.
 */
export function orderBatchPresentation(
  block: Block,
  path: BlockPath,
  mediaKind: PrintPacket["media"]["kind"],
): OrderBatchPresentation | null {
  if (
    path.length !== 1 ||
    block.type !== "repeat" ||
    block.items.type !== "path" ||
    block.items.path.length !== 1 ||
    block.items.path[0] !== "orders"
  )
    return null;
  if (mediaKind === "continuous") return "continuous";
  if (mediaKind === "label") return "fixed_media";
  return blocksContainPageBreak(block.children)
    ? "one_order_per_page"
    : "flowing_pages";
}

function blocksContainPageBreak(blocks: readonly Block[]): boolean {
  return blocks.some((block) => {
    if (block.type === "page_break") return true;
    if ("children" in block && blocksContainPageBreak(block.children))
      return true;
    if (block.type === "table")
      return blocksContainPageBreak(block.empty ?? []);
    if (block.type === "data_list")
      return (
        blocksContainPageBreak(block.header ?? []) ||
        blocksContainPageBreak(block.item) ||
        blocksContainPageBreak(block.empty ?? [])
      );
    return block.type === "conditional"
      ? blocksContainPageBreak(block.then) ||
          blocksContainPageBreak(block.else ?? [])
      : false;
  });
}

export function PrintPacketPreview({
  value,
  stock = null,
  workspaceControls,
}: {
  value: PrintPacket;
  stock?: DesignStock;
  workspaceControls?: ReactNode;
}) {
  return (
    <div className="piqae-preview-stage" aria-label="Document layout preview">
      {workspaceControls ? (
        <div className="piqae-workspace-toolbar">{workspaceControls}</div>
      ) : null}
      <MediaRuler value={value} stock={stock} showSafeArea={false} />
      <div
        className={`piqae-page-sheet piqae-rendered-canvas piqae-presentation-canvas piqae-media-${value.media.kind}`}
        style={canvasStyle(value)}
      >
        <DocumentCanvas
          blocks={canonicalizeShopifyEditorBody(value.body)}
          editable={false}
          mediaKind={value.media.kind}
          preview
          onSelect={() => undefined}
          onChange={() => undefined}
        />
      </div>
    </div>
  );
}

export function canvasGeometry(value: PrintPacket): {
  widthMm: number;
  heightMm: number | null;
} {
  const media = value.media;
  let width = 210;
  let height: number | null = 297;
  if (media.kind === "paged") {
    const portrait =
      media.size === "a4"
        ? [210, 297]
        : media.size === "a5"
          ? [148, 210]
          : [215.9, 279.4];
    [width, height] =
      media.orientation === "landscape"
        ? [portrait[1]!, portrait[0]!]
        : [portrait[0]!, portrait[1]!];
  } else if (media.kind === "continuous") {
    width = media.width_mm;
    height = null;
  } else {
    width = media.width_mm;
    height = media.height_mm;
  }
  return { widthMm: width, heightMm: height };
}

export function canvasStyle(value: PrintPacket): CSSProperties {
  const media = value.media;
  const margins = media.margins ?? {
    top_mm: 0,
    right_mm: 0,
    bottom_mm: 0,
    left_mm: 0,
  };
  const { widthMm, heightMm } = canvasGeometry(value);
  const fixed = heightMm !== null;
  const padding = fixed
    ? `${(margins.top_mm / widthMm) * 100}% ${(margins.right_mm / widthMm) * 100}% ${(margins.bottom_mm / widthMm) * 100}% ${(margins.left_mm / widthMm) * 100}%`
    : `${margins.top_mm}mm ${margins.right_mm}mm ${margins.bottom_mm}mm ${margins.left_mm}mm`;
  return {
    "--piqae-media-width": `${widthMm}mm`,
    "--piqae-media-height": "auto",
    "--piqae-media-min-height": fixed ? "0" : "120mm",
    aspectRatio: fixed ? `${widthMm} / ${heightMm}` : undefined,
    padding,
  } as CSSProperties;
}

/**
 * Mirrors the fixed-media CSS rule at a concrete browser width. Keeping this
 * calculation explicit lets narrow-width tests prove that responsive scaling
 * preserves the physical aspect ratio without treating CSS pixels as mm.
 */
export function responsiveCanvasGeometry(
  value: PrintPacket,
  containerWidthPx: number,
  pixelsPerMm = 96 / 25.4,
): { widthPx: number; heightPx: number | null } {
  if (!Number.isFinite(containerWidthPx) || containerWidthPx < 0)
    throw new Error("Canvas container width is invalid");
  const { widthMm, heightMm } = canvasGeometry(value);
  const widthPx = Math.min(
    widthMm * pixelsPerMm,
    Math.max(0, containerWidthPx - 32),
  );
  return {
    widthPx,
    heightPx: heightMm === null ? null : widthPx * (heightMm / widthMm),
  };
}

export function safeAreaStyle(
  value: PrintPacket,
  safe: NonNullable<NonNullable<DesignStock>["safeAreaMm"]>,
): CSSProperties {
  const { widthMm, heightMm } = canvasGeometry(value);
  if (heightMm === null)
    return {
      inset: `${safe.top}mm ${safe.right}mm ${safe.bottom}mm ${safe.left}mm`,
    };
  return {
    top: `${(safe.top / heightMm) * 100}%`,
    right: `${(safe.right / widthMm) * 100}%`,
    bottom: `${(safe.bottom / heightMm) * 100}%`,
    left: `${(safe.left / widthMm) * 100}%`,
  };
}

function SafeAreaGuide({
  value,
  stock,
}: {
  value: PrintPacket;
  stock: DesignStock;
}) {
  if (!stock?.safeAreaMm) return null;
  const safe = stock.safeAreaMm;
  return (
    <div
      className="piqae-safe-area-guide"
      style={safeAreaStyle(value, safe)}
      aria-hidden="true"
    />
  );
}

function MediaRuler({
  value,
  stock,
  showSafeArea = true,
}: {
  value: PrintPacket;
  stock: DesignStock;
  showSafeArea?: boolean;
}) {
  const media = value.media;
  const size =
    media.kind === "paged"
      ? `${media.size.toUpperCase()} · ${media.orientation ?? "portrait"}`
      : media.kind === "continuous"
        ? `${media.width_mm} mm continuous roll`
        : `${media.width_mm} × ${media.height_mm} mm fixed label`;
  return (
    <div className="piqae-media-ruler" aria-label="Document media">
      <strong>{size}</strong>
      {showSafeArea && stock?.safeAreaMm ? <span>Safe area shown</span> : null}
      {stock?.gapMm !== null && stock?.gapMm !== undefined ? (
        <span>{stock.gapMm} mm gap</span>
      ) : null}
      {stock?.markIntervalMm !== null && stock?.markIntervalMm !== undefined ? (
        <span>{stock.markIntervalMm} mm mark interval</span>
      ) : null}
    </div>
  );
}

function DocumentCanvas({
  blocks,
  path = [],
  branch = "root",
  editable = true,
  mediaKind,
  preview = false,
  selectedPath,
  authoringFields = AUTHORING_FIELDS,
  scope = "order",
  insertionSlots = true,
  onSelect,
  onInsert,
  onChange,
}: {
  blocks: Block[];
  path?: BlockPath;
  branch?: BlockPathPart["branch"];
  editable?: boolean;
  mediaKind: PrintPacket["media"]["kind"];
  preview?: boolean;
  selectedPath?: BlockPath;
  authoringFields?: readonly ShopifyDocumentField[];
  scope?: ShopifyAuthoringScope;
  insertionSlots?: boolean;
  onSelect(block: Block, path: BlockPath): void;
  onInsert?(block: Block, path: BlockPath): void;
  onChange(block: Block, path: BlockPath): void;
}) {
  const insertionPath = (index: number): BlockPath => [
    ...path,
    { branch, index },
  ];
  return (
    <div className="piqae-document-flow">
      {blocks.map((block, index) => {
        const blockPath = insertionPath(index);
        const key = `${path.map((part) => `${part.branch}-${part.index}`).join("/")}-${branch}-${index}`;
        return (
          <Fragment key={key}>
            {editable && insertionSlots && onInsert ? (
              <CanvasInsertionSlot path={blockPath} onInsert={onInsert} />
            ) : null}
            <CanvasBlock
              block={block}
              path={blockPath}
              editable={editable}
              mediaKind={mediaKind}
              preview={preview}
              selectedPath={selectedPath}
              selected={sameBlockPath(selectedPath, blockPath)}
              authoringFields={authoringFields}
              scope={scope}
              onSelect={onSelect}
              onInsert={onInsert}
              onChange={onChange}
            />
          </Fragment>
        );
      })}
      {editable && insertionSlots && onInsert ? (
        <CanvasInsertionSlot
          path={insertionPath(blocks.length)}
          onInsert={onInsert}
        />
      ) : null}
    </div>
  );
}

function CanvasBlock({
  block,
  path,
  editable,
  mediaKind,
  preview,
  selectedPath,
  selected,
  authoringFields,
  scope,
  onSelect,
  onInsert,
  onChange,
}: {
  block: Block;
  path: BlockPath;
  editable: boolean;
  mediaKind: PrintPacket["media"]["kind"];
  preview: boolean;
  selectedPath?: BlockPath;
  selected: boolean;
  authoringFields: readonly ShopifyDocumentField[];
  scope: ShopifyAuthoringScope;
  onSelect(block: Block, path: BlockPath): void;
  onInsert?(block: Block, path: BlockPath): void;
  onChange(block: Block, path: BlockPath): void;
}) {
  const [editingText, setEditingText] = useState(false);
  const textBlockElement = useRef<HTMLElement | null>(null);
  const resizeDrag = useRef<{
    index: number;
    startX: number;
    tableWidth: number;
    columns: Extract<Block, { type: "table" }>["columns"];
  } | null>(null);
  const topLevelText =
    isTopLevelAuthoringPath(path) &&
    (block.type === "paragraph" || block.type === "heading");
  useEffect(() => {
    if (!selected) setEditingText(false);
  }, [selected]);
  const batchPresentation = orderBatchPresentation(block, path, mediaKind);
  const selectableClass = preview ? "" : " piqae-canvas-selectable";
  const select = (event: React.MouseEvent) => {
    if (preview || batchPresentation) return;
    event.stopPropagation();
    onSelect(block, path);
  };
  if (block.type === "paragraph" || block.type === "heading") {
    const Tag =
      block.type === "heading" ? (`h${block.level ?? 2}` as "h1") : "p";
    return (
      <Tag
        ref={(element) => {
          textBlockElement.current = element;
        }}
        className={`piqae-canvas-text${selected ? " piqae-canvas-selected" : ""}`}
        style={{ textAlign: block.style?.align ?? "left" }}
        tabIndex={topLevelText && editable ? 0 : undefined}
        aria-label={
          topLevelText && editable
            ? `${block.type === "heading" ? "Heading" : "Text"} block: ${inlineLabel(block.content)}. Press Enter or F2 to edit.`
            : undefined
        }
        onClick={select}
        onFocus={(event) => {
          if (topLevelText && event.target === event.currentTarget)
            onSelect(block, path);
        }}
        onDoubleClick={(event) => {
          if (!topLevelText || !editable) return;
          event.stopPropagation();
          onSelect(block, path);
          setEditingText(true);
        }}
        onKeyDown={(event) => {
          if (
            topLevelText &&
            editable &&
            !editingText &&
            (event.key === "Enter" || event.key === "F2")
          ) {
            event.preventDefault();
            event.stopPropagation();
            onSelect(block, path);
            setEditingText(true);
          }
        }}
      >
        <ExpressionEditor
          value={editableInlineWithScope(block.content, scope)}
          fields={contextualFieldSuggestions(authoringFields, scope)}
          disabled={!editable || (topLevelText && !editingText)}
          multiline
          autoFocus={topLevelText && editingText}
          onEscape={() => {
            setEditingText(false);
            requestAnimationFrame(() => textBlockElement.current?.focus());
          }}
          onBlur={() => {
            if (topLevelText) setEditingText(false);
          }}
          onChange={(source) =>
            onChange(
              {
                ...block,
                content: parseContextualInline(source, block.content, scope),
              },
              path,
            )
          }
        />
      </Tag>
    );
  }
  if (block.type === "divider")
    return (
      <hr
        className={`${selectableClass.trim()}${selected ? " piqae-canvas-selected" : ""}`}
        onClick={preview ? undefined : select}
      />
    );
  if (block.type === "spacer")
    return (
      <div
        className={`piqae-canvas-spacer${selectableClass}${selected ? " piqae-canvas-selected" : ""}`}
        style={{ height: `${Math.max(8, block.height_mm * 2)}px` }}
        onClick={select}
      >
        <span>{block.height_mm} mm space</span>
      </div>
    );
  if (block.type === "page_break")
    return preview ? null : (
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
        className={`piqae-canvas-image${selectableClass}${selected ? " piqae-canvas-selected" : ""}`}
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
        className={`piqae-canvas-code piqae-canvas-${block.type}${selectableClass}${selected ? " piqae-canvas-selected" : ""}`}
        onClick={select}
      >
        <span aria-hidden="true">{block.type === "qr" ? "▦" : "▌█▌▌█▌█"}</span>
        <small>{expressionLabel(block.value)}</small>
      </div>
    );
  if (block.type === "table")
    return (
      <div
        className={`piqae-canvas-table${selectableClass}${selected ? " piqae-canvas-selected" : ""}`}
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
                <>
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
                            columns: block.columns.filter(
                              (_, i) => i !== index,
                            ),
                          },
                          path,
                        )
                      }
                    >
                      ×
                    </button>
                  </span>
                  {index < block.columns.length - 1 ? (
                    <span
                      className="piqae-canvas-column-resize"
                      role="separator"
                      aria-orientation="vertical"
                      aria-label={`Resize ${inlineLabel(column.header)} column`}
                      aria-valuemin={10}
                      aria-valuemax={90}
                      aria-valuenow={columnBoundaryPercent(
                        block.columns,
                        index,
                      )}
                      tabIndex={0}
                      onClick={(event) => event.stopPropagation()}
                      onKeyDown={(event) => {
                        if (
                          event.key !== "ArrowLeft" &&
                          event.key !== "ArrowRight"
                        )
                          return;
                        event.preventDefault();
                        event.stopPropagation();
                        onChange(
                          {
                            ...block,
                            columns: resizeColumns(
                              block.columns,
                              index,
                              (event.key === "ArrowRight" ? 1 : -1) *
                                adjacentColumnWidth(block.columns, index) *
                                0.05,
                            ),
                          },
                          path,
                        );
                      }}
                      onPointerDown={(event) => {
                        event.preventDefault();
                        event.stopPropagation();
                        const table = event.currentTarget.closest(
                          ".piqae-canvas-table",
                        );
                        resizeDrag.current = {
                          index,
                          startX: event.clientX,
                          tableWidth: Math.max(
                            1,
                            table?.getBoundingClientRect().width ?? 1,
                          ),
                          columns: block.columns,
                        };
                        event.currentTarget.setPointerCapture(event.pointerId);
                      }}
                      onPointerMove={(event) => {
                        const drag = resizeDrag.current;
                        if (!drag || drag.index !== index) return;
                        const total = drag.columns.reduce(
                          (sum, item) => sum + (item.width ?? 1),
                          0,
                        );
                        onChange(
                          {
                            ...block,
                            columns: resizeColumns(
                              drag.columns,
                              index,
                              ((event.clientX - drag.startX) /
                                drag.tableWidth) *
                                total,
                            ),
                          },
                          path,
                        );
                      }}
                      onPointerUp={(event) => {
                        if (resizeDrag.current?.index === index)
                          resizeDrag.current = null;
                        if (
                          event.currentTarget.hasPointerCapture(event.pointerId)
                        )
                          event.currentTarget.releasePointerCapture(
                            event.pointerId,
                          );
                      }}
                      onPointerCancel={() => {
                        if (resizeDrag.current?.index === index)
                          resizeDrag.current = null;
                      }}
                    />
                  ) : null}
                </>
              ) : null}
            </span>
          ))}
          {editable ? (
            <button
              className="piqae-canvas-add-column"
              type="button"
              aria-label="Add table column"
              data-tooltip="Add column"
              onClick={(event) => {
                event.stopPropagation();
                onChange(
                  { ...block, columns: [...block.columns, defaultColumn()] },
                  path,
                );
              }}
            >
              <Icon name="plus" />
            </button>
          ) : null}
        </div>
        <div className="piqae-canvas-table-row piqae-canvas-table-binding-row">
          {block.columns.map((column, index) => (
            <div
              key={index}
              style={{ flex: column.width ?? 1, textAlign: column.align }}
              onClick={(event) => event.stopPropagation()}
            >
              <ExpressionEditor
                aria-label={`${inlineLabel(column.header)} value`}
                value={editableInlineWithScope(column.cell, "item")}
                fields={contextualFieldSuggestions(authoringFields, "item")}
                disabled={!editable}
                placeholder="{{ item.title }}"
                onChange={(source) =>
                  onChange(
                    {
                      ...block,
                      columns: block.columns.map((item, itemIndex) =>
                        itemIndex === index
                          ? {
                              ...item,
                              cell: parseContextualInline(
                                source,
                                item.cell,
                                "item",
                              ),
                            }
                          : item,
                      ),
                    },
                    path,
                  )
                }
              />
            </div>
          ))}
        </div>
      </div>
    );
  if (block.type === "conditional")
    return (
      <section
        className={`piqae-canvas-conditional${selectableClass}${selected ? " piqae-canvas-selected" : ""}`}
        onClick={select}
      >
        {preview ? null : (
          <span className="piqae-canvas-badge">
            Shown when {expressionLabel(block.condition)}
          </span>
        )}
        {block.then.length ? (
          <DocumentCanvas
            blocks={block.then}
            path={path}
            branch="then"
            editable={editable}
            mediaKind={mediaKind}
            preview={preview}
            selectedPath={selectedPath}
            authoringFields={authoringFields}
            scope={scope}
            onSelect={onSelect}
            onInsert={onInsert}
            onChange={onChange}
          />
        ) : (
          <AddContentSlot
            label="Add content"
            editable={editable}
            preview={preview}
            onAdd={(child) => onChange({ ...block, then: [child] }, path)}
          />
        )}
        {!preview && block.else?.length ? (
          <>
            <span className="piqae-canvas-badge">Otherwise</span>
            <DocumentCanvas
              blocks={block.else}
              path={path}
              branch="else"
              editable={editable}
              mediaKind={mediaKind}
              preview={preview}
              selectedPath={selectedPath}
              authoringFields={authoringFields}
              scope={scope}
              onSelect={onSelect}
              onInsert={onInsert}
              onChange={onChange}
            />
          </>
        ) : null}
      </section>
    );
  const children = "children" in block ? block.children : [];
  const childScope =
    block.type === "repeat" && isLineItemsExpression(block.items, scope)
      ? "item"
      : scope;
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
      : block.type === "repeat"
        ? undefined
        : { gap: `${"gap_mm" in block ? (block.gap_mm ?? 0) : 0}mm` };
  const batchLabel =
    batchPresentation === "one_order_per_page"
      ? "One page per order"
      : batchPresentation === "flowing_pages"
        ? "Orders flow across pages"
        : batchPresentation === "continuous"
          ? "Continuous order batch"
          : batchPresentation === "fixed_media"
            ? "Fixed-media order batch"
            : null;
  return (
    <section
      className={`${className}${batchPresentation ? " piqae-canvas-order-batch" : selectableClass}${selected ? " piqae-canvas-selected" : ""}`}
      style={style}
      onClick={preview || batchPresentation ? undefined : select}
    >
      {!preview && batchLabel ? (
        <span
          className="piqae-canvas-structure-note"
          role="note"
          aria-label="Order batching behavior"
        >
          {batchLabel}
        </span>
      ) : !preview && block.type === "repeat" ? (
        <span className="piqae-canvas-badge">
          Repeats for each {expressionLabel(block.items)}
        </span>
      ) : null}
      {editable && block.type === "grid" ? (
        <GridResizeHandles block={block} path={path} onChange={onChange} />
      ) : null}
      {children.length ? (
        <DocumentCanvas
          blocks={children}
          path={path}
          branch="children"
          editable={editable}
          mediaKind={mediaKind}
          preview={preview}
          selectedPath={selectedPath}
          authoringFields={authoringFields}
          scope={childScope}
          insertionSlots={block.type !== "grid" && block.type !== "row"}
          onSelect={onSelect}
          onInsert={onInsert}
          onChange={onChange}
        />
      ) : (
        <AddContentSlot
          label="Add content"
          editable={editable}
          preview={preview}
          onAdd={(child) =>
            "children" in block
              ? onChange({ ...block, children: [child] }, path)
              : undefined
          }
        />
      )}
    </section>
  );
}

function GridResizeHandles({
  block,
  path,
  onChange,
}: {
  block: Extract<Block, { type: "grid" }>;
  path: BlockPath;
  onChange(block: Block, path: BlockPath): void;
}) {
  const drag = useRef<{
    index: number;
    startX: number;
    gridWidth: number;
    columns: number[];
  } | null>(null);
  const total = block.columns.reduce((sum, width) => sum + width, 0);
  let cumulative = 0;
  return (
    <>
      {block.columns.slice(0, -1).map((width, index) => {
        cumulative += width;
        const position = (cumulative / Math.max(total, 0.01)) * 100;
        return (
          <span
            key={index}
            className="piqae-canvas-grid-resize"
            style={{ left: `${position}%` }}
            role="separator"
            aria-orientation="vertical"
            aria-label={`Resize column ${index + 1}`}
            aria-valuemin={10}
            aria-valuemax={90}
            aria-valuenow={gridBoundaryPercent(block.columns, index)}
            tabIndex={0}
            onClick={(event) => event.stopPropagation()}
            onKeyDown={(event) => {
              if (event.key !== "ArrowLeft" && event.key !== "ArrowRight")
                return;
              event.preventDefault();
              event.stopPropagation();
              onChange(
                {
                  ...block,
                  columns: resizeGridColumns(
                    block.columns,
                    index,
                    (event.key === "ArrowRight" ? 1 : -1) *
                      adjacentGridWidth(block.columns, index) *
                      0.05,
                  ),
                },
                path,
              );
            }}
            onPointerDown={(event) => {
              event.preventDefault();
              event.stopPropagation();
              const grid = event.currentTarget.closest(".piqae-canvas-grid");
              drag.current = {
                index,
                startX: event.clientX,
                gridWidth: Math.max(
                  1,
                  grid?.getBoundingClientRect().width ?? 1,
                ),
                columns: block.columns,
              };
              event.currentTarget.setPointerCapture(event.pointerId);
            }}
            onPointerMove={(event) => {
              const active = drag.current;
              if (!active || active.index !== index) return;
              const activeTotal = active.columns.reduce(
                (sum, item) => sum + item,
                0,
              );
              onChange(
                {
                  ...block,
                  columns: resizeGridColumns(
                    active.columns,
                    index,
                    ((event.clientX - active.startX) / active.gridWidth) *
                      activeTotal,
                  ),
                },
                path,
              );
            }}
            onPointerUp={(event) => {
              if (drag.current?.index === index) drag.current = null;
              if (event.currentTarget.hasPointerCapture(event.pointerId))
                event.currentTarget.releasePointerCapture(event.pointerId);
            }}
            onPointerCancel={() => {
              if (drag.current?.index === index) drag.current = null;
            }}
          />
        );
      })}
    </>
  );
}

function CanvasInsertionSlot({
  path,
  onInsert,
}: {
  path: BlockPath;
  onInsert(block: Block, path: BlockPath): void;
}) {
  const [open, setOpen] = useState(false);
  const [dragOver, setDragOver] = useState(false);
  const insert = (block: Block) => {
    onInsert(block, path);
    setOpen(false);
    setDragOver(false);
  };
  return (
    <span
      className={`piqae-canvas-insertion-slot${dragOver ? " is-drag-over" : ""}`}
      data-insertion-index={path.at(-1)?.index}
      onBlur={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget as Node | null))
          setOpen(false);
      }}
      onDragEnter={(event) => {
        if (!hasPiqaeBlockDrag(event.dataTransfer)) return;
        event.preventDefault();
        setDragOver(true);
      }}
      onDragOver={(event) => {
        if (!hasPiqaeBlockDrag(event.dataTransfer)) return;
        event.preventDefault();
        event.dataTransfer.dropEffect = "copy";
        setDragOver(true);
      }}
      onDragLeave={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget as Node | null))
          setDragOver(false);
      }}
      onDrop={(event) => {
        event.preventDefault();
        const block = draggedBlock(event.dataTransfer);
        if (block) insert(block);
        else setDragOver(false);
      }}
    >
      <span className="piqae-canvas-insertion-line" aria-hidden="true" />
      <button
        className="piqae-canvas-insertion-button"
        type="button"
        aria-label="Add content here"
        aria-expanded={open}
        aria-haspopup="menu"
        onClick={(event) => {
          event.stopPropagation();
          setOpen((value) => !value);
        }}
      >
        <Icon name="plus" />
      </button>
      {open ? (
        <span
          className="piqae-canvas-insertion-menu"
          role="menu"
          aria-label="Add content"
          onKeyDown={(event) => {
            if (event.key === "Escape") {
              event.preventDefault();
              setOpen(false);
            }
          }}
        >
          {QUICK_INSERT_TYPES.map((type) => (
            <button
              key={type}
              type="button"
              role="menuitem"
              onClick={(event) => {
                event.stopPropagation();
                insert(quickInsertBlock(type));
              }}
            >
              <Icon name={quickInsertIcon(type)} />
              {quickInsertLabel(type)}
            </button>
          ))}
        </span>
      ) : null}
    </span>
  );
}

/** Keeps empty containers and branches editable without a side panel. */
function AddContentSlot({
  label,
  editable,
  preview = false,
  onAdd,
}: {
  label: string;
  editable: boolean;
  preview?: boolean;
  onAdd(block: Block): void;
}) {
  if (preview) return null;
  if (!editable) return <span className="piqae-canvas-empty">No content</span>;
  return (
    <button
      className="piqae-canvas-empty piqae-canvas-add"
      type="button"
      onClick={(event) => {
        event.stopPropagation();
        onAdd({
          type: "paragraph",
          content: [{ type: "text", value: "Start typing…" }],
        });
      }}
    >
      <Icon name="plus" />
      {label}
    </button>
  );
}

/** One uniform 16px stroke icon set so every action reads the same weight. */
const ICON_PATHS = {
  text: "M3 4.2h10M8 4.2v8.6M6 12.8h4",
  heading: "M4.2 3v10M11.8 3v10M4.2 8h7.6",
  data: "M6.6 2.8c-1.4 0-1.4 1.9-1.4 3.3S3.8 8 3.8 8s1.4 0 1.4 1.9.0 3.3 1.4 3.3M9.4 2.8c1.4 0 1.4 1.9 1.4 3.3S12.2 8 12.2 8s-1.4 0-1.4 1.9 0 3.3-1.4 3.3",
  table: "M2.5 3.5h11v9h-11zM2.5 6.7h11M6.4 6.7v5.8M10.1 6.7v5.8",
  repeat:
    "M3.5 6.2A3 3 0 0 1 6.5 3.4h6M11 1.9l1.6 1.5L11 4.9M12.5 9.8a3 3 0 0 1-3 2.8h-6M5 11.1 3.4 12.6 5 14.1",
  condition: "M8 2.4 13.6 8 8 13.6 2.4 8Z",
  image:
    "M2.5 3.5h11v9h-11zM2.5 10.2l3.1-3 2.7 2.6 2.3-2.2 2.9 2.8M11.1 6.3a.85.85 0 1 1-1.7 0 .85.85 0 0 1 1.7 0Z",
  qr: "M2.6 2.6h4v4h-4zM9.4 2.6h4v4h-4zM2.6 9.4h4v4h-4zM9.4 9.4h1.6v1.6h-1.6zM12.1 12.1h1.3v1.3h-1.3z",
  barcode: "M3 3v10M5.4 3v10M7.8 3v7M10.2 3v10M13 3v7",
  divider: "M2.5 8h11",
  spacer: "M8 3.2v9.6M5.6 5.6 8 3.2l2.4 2.4M5.6 10.4 8 12.8l2.4-2.4",
  columns: "M2.5 3.5h11v9h-11zM8 3.5v9",
  stack: "M2.5 3.6h11M2.5 8h11M2.5 12.4h11",
  row: "M2.5 8h11M5 5.5 2.5 8 5 10.5M11 5.5 13.5 8 11 10.5",
  up: "M8 12.8V3.6M4.6 7 8 3.6 11.4 7",
  down: "M8 3.2v9.2M4.6 9 8 12.4 11.4 9",
  duplicate: "M5.6 5.6h7.8v7.8H5.6zM10.6 5.6V2.6H2.8v7.8h2.8",
  trash: "M2.8 4.4h10.4M6.3 4.4V2.9h3.4v1.5M4.4 4.4l.6 8.7h6l.6-8.7",
  settings: "M2.6 5.2h10.8M2.6 10.8h10.8M6 3.6v3.2M10.4 9.2v3.2",
  design: "M2.5 3.5h11v9h-11zM2.5 6.6h11M6.2 6.6v5.9",
  code: "M5.9 4.4 2.6 8l3.3 3.6M10.1 4.4 13.4 8l-3.3 3.6",
  preview:
    "M1.6 8s2.4-4.2 6.4-4.2S14.4 8 14.4 8s-2.4 4.2-6.4 4.2S1.6 8 1.6 8ZM9.8 8a1.8 1.8 0 1 1-3.6 0 1.8 1.8 0 0 1 3.6 0Z",
  more: "M4 8h.01M8 8h.01M12 8h.01",
  close: "M4.2 4.2 11.8 11.8M11.8 4.2 4.2 11.8",
  plus: "M8 3.6v8.8M3.6 8h8.8",
} as const;
export type IconName = keyof typeof ICON_PATHS;

export function Icon({ name }: { name: IconName }) {
  return (
    <svg
      className="piqae-icon"
      viewBox="0 0 16 16"
      width="16"
      height="16"
      fill="none"
      stroke="currentColor"
      strokeWidth={name === "more" ? 2.2 : 1.35}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      <path d={ICON_PATHS[name]} />
    </svg>
  );
}

function ToolButton({
  icon,
  label,
  tone,
  disabled,
  dragBlock,
  onClick,
}: {
  icon: IconName;
  label: string;
  tone?: "critical";
  disabled?: boolean;
  dragBlock?: () => Block;
  onClick(): void;
}) {
  return (
    <button
      className={`piqae-tool-button${tone === "critical" ? " piqae-tool-critical" : ""}`}
      type="button"
      aria-label={label}
      data-tooltip={label}
      disabled={disabled}
      draggable={Boolean(dragBlock) && !disabled}
      onDragStart={(event) => {
        if (!dragBlock || disabled) {
          event.preventDefault();
          return;
        }
        event.dataTransfer.effectAllowed = "copy";
        event.dataTransfer.setData(
          PIQAE_BLOCK_DRAG_TYPE,
          JSON.stringify(dragBlock()),
        );
      }}
      onClick={onClick}
    >
      <Icon name={icon} />
    </button>
  );
}

function InsertDataButton({
  fields,
  disabled,
  onInsert,
}: {
  fields: readonly ShopifyDocumentField[];
  disabled?: boolean;
  onInsert(path: string): void;
}) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const matches = searchDocumentFields(fields, query, 40);
  return (
    <span
      className="piqae-tool-menu"
      onBlur={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget as Node | null))
          setOpen(false);
      }}
    >
      <button
        className="piqae-tool-button"
        type="button"
        aria-label="Insert Shopify data"
        aria-expanded={open}
        aria-haspopup="dialog"
        data-tooltip="Shopify data"
        disabled={disabled}
        onClick={() => setOpen((value) => !value)}
      >
        <Icon name="data" />
      </button>
      {open ? (
        <span className="piqae-popover" role="dialog" aria-label="Shopify data">
          <input
            className="piqae-popover-search"
            type="search"
            autoFocus
            placeholder="Search order, customer or item data…"
            aria-label="Search Shopify data"
            value={query}
            onChange={(event) => setQuery(event.currentTarget.value)}
            onKeyDown={(event) => {
              if (event.key === "Escape") setOpen(false);
              if (event.key === "Enter" && matches[0]) {
                event.preventDefault();
                onInsert(matches[0].path);
                setOpen(false);
              }
            }}
          />
          <span className="piqae-popover-list" role="listbox">
            {matches.length ? (
              matches.map((field) => (
                <button
                  type="button"
                  role="option"
                  aria-selected="false"
                  key={field.path}
                  onClick={() => {
                    onInsert(field.path);
                    setOpen(false);
                  }}
                >
                  <span>{field.label}</span>
                  <small>{`{{ ${field.path} }}`}</small>
                  <em>{field.group}</em>
                </button>
              ))
            ) : (
              <span className="piqae-popover-empty">
                No matching Shopify data.
              </span>
            )}
          </span>
          <span className="piqae-popover-footnote">
            Tip: type <code>{"{{"}</code> anywhere on the page to insert data
            inline.
          </span>
        </span>
      ) : null}
    </span>
  );
}

/**
 * Document-wide settings. These belong to the document rather than to any
 * selected block, so the editor surfaces them from the top action bar instead
 * of a side panel.
 */
export function DocumentSettingsFields({
  value,
  disabled,
  onChange,
}: {
  value: PrintPacket;
  disabled?: boolean;
  onChange(document: PrintPacket): void;
}) {
  const theme = value.theme ?? {};
  const updateRegion = (region: "header" | "footer", content: string) =>
    onChange({
      ...value,
      [region]: {
        first: value[region]?.first ?? [],
        default: content
          ? [{ type: "paragraph", content: [{ type: "text", value: content }] }]
          : [],
        last: value[region]?.last ?? [],
      },
    });
  return (
    <>
      <label className="piqae-field">
        <span>Base text size</span>
        <input
          type="number"
          min={7}
          max={24}
          value={theme.font_size_pt ?? 10}
          disabled={disabled}
          onChange={(event) =>
            onChange({
              ...value,
              theme: {
                ...value.theme,
                font_size_pt: Number(event.currentTarget.value),
              },
            })
          }
        />
      </label>
      <label className="piqae-field">
        <span>Text colour</span>
        <input
          type="color"
          value={rgbToHex(theme.text_color ?? { red: 32, green: 34, blue: 35 })}
          disabled={disabled}
          onChange={(event) =>
            onChange({
              ...value,
              theme: {
                ...value.theme,
                text_color: hexToRgb(event.currentTarget.value),
              },
            })
          }
        />
      </label>
      <label className="piqae-field piqae-field-wide">
        <span>Repeating page header</span>
        <input
          type="text"
          value={regionText(value.header?.default)}
          placeholder="Shown at the top of every page"
          disabled={disabled}
          onChange={(event) =>
            updateRegion("header", event.currentTarget.value)
          }
        />
      </label>
      <label className="piqae-field piqae-field-wide">
        <span>Repeating page footer</span>
        <input
          type="text"
          value={regionText(value.footer?.default)}
          placeholder="Shown at the bottom of every page"
          disabled={disabled}
          onChange={(event) =>
            updateRegion("footer", event.currentTarget.value)
          }
        />
      </label>
    </>
  );
}

function ExpressionEditor({
  value,
  fields,
  disabled,
  multiline = false,
  autoFocus = false,
  placeholder,
  onBlur,
  onEscape,
  onChange,
  ...attributes
}: {
  value: string;
  fields: readonly ShopifyDocumentField[];
  disabled?: boolean;
  multiline?: boolean;
  autoFocus?: boolean;
  placeholder?: string;
  onBlur?(): void;
  onEscape?(): void;
  onChange(value: string): void;
  "aria-label"?: string;
}) {
  const editor = useRef<HTMLSpanElement>(null);
  const [query, setQuery] = useState<string | null>(null);
  const [active, setActive] = useState(0);
  const matches = query === null ? [] : searchDocumentFields(fields, query);
  useEffect(() => {
    if (!editor.current || editor.current.textContent === value) return;
    const focused = document.activeElement === editor.current;
    editor.current.textContent = value;
    if (focused) placeCaretAtEnd(editor.current);
  }, [value]);
  useEffect(() => {
    if (!autoFocus || disabled || !editor.current) return;
    editor.current.focus();
    placeCaretAtEnd(editor.current);
  }, [autoFocus, disabled]);
  const update = (source: string) => {
    onChange(source);
    const nextQuery = incompleteExpressionQuery(source);
    setQuery(nextQuery);
    setActive(0);
  };
  const choose = (field: ShopifyDocumentField) => {
    const source = completeExpression(valueFromEditor(editor), field.path);
    if (editor.current) editor.current.textContent = source;
    onChange(source);
    setQuery(null);
    editor.current?.focus();
    placeCaretAtEnd(editor.current);
  };
  return (
    <span
      className={`piqae-expression-editor${multiline ? " piqae-expression-editor-multiline" : ""}`}
    >
      <span
        {...attributes}
        ref={editor}
        role="textbox"
        aria-multiline={multiline}
        aria-autocomplete="list"
        aria-expanded={query !== null}
        aria-readonly={disabled || undefined}
        data-placeholder={placeholder}
        contentEditable={!disabled}
        suppressContentEditableWarning
        onInput={(event) => update(event.currentTarget.textContent ?? "")}
        onKeyDown={(event) => {
          if (event.key === "Escape" && query === null && onEscape) {
            event.preventDefault();
            event.stopPropagation();
            onEscape();
            return;
          }
          if (query === null) {
            if (!multiline && event.key === "Enter") event.preventDefault();
            return;
          }
          if (event.key === "Escape") {
            event.preventDefault();
            setQuery(null);
          } else if (event.key === "ArrowDown") {
            event.preventDefault();
            setActive((index) => Math.min(index + 1, matches.length - 1));
          } else if (event.key === "ArrowUp") {
            event.preventDefault();
            setActive((index) => Math.max(index - 1, 0));
          } else if (
            (event.key === "Enter" || event.key === "Tab") &&
            matches[active]
          ) {
            event.preventDefault();
            choose(matches[active]!);
          } else if (!multiline && event.key === "Enter")
            event.preventDefault();
        }}
        onBlur={() => {
          setTimeout(() => setQuery(null), 100);
          onBlur?.();
        }}
      />
      {query !== null ? (
        <span className="piqae-expression-menu" role="listbox">
          <span className="piqae-expression-menu-title">
            {query ? `Results for “${query}”` : "Insert Shopify data"}
          </span>
          {matches.length ? (
            matches.map((field, index) => (
              <button
                type="button"
                role="option"
                aria-selected={index === active}
                className={index === active ? "is-active" : ""}
                key={field.path}
                onMouseDown={(event) => event.preventDefault()}
                onClick={() => choose(field)}
              >
                <span>{field.label}</span>
                <small>{`{{ ${field.path} }}`}</small>
                <em>{field.group}</em>
              </button>
            ))
          ) : (
            <span className="piqae-expression-empty">
              No matching Shopify data. Continue typing a valid path.
            </span>
          )}
        </span>
      ) : null}
    </span>
  );
}

/**
 * Compact settings for the selected block, rendered inline in the canvas
 * selection bar. Nested content is edited directly on the page, so there is no
 * parallel block tree in a side panel to keep in sync.
 */
function SelectionSettings({
  block,
  disabled,
  authoringFields,
  scope,
  onChange,
}: {
  block: Block;
  disabled?: boolean;
  authoringFields: readonly ShopifyDocumentField[];
  scope: ShopifyAuthoringScope;
  onChange(block: Block): void;
}) {
  const listId = useId();
  const fields = selectionFields({ block, disabled, listId, scope, onChange });
  if (!fields) return null;
  return (
    <span className="piqae-bar-fields">
      <datalist id={`${listId}-all`}>
        {authoringFields.map((field) => (
          <option value={field.path} key={field.path} label={field.label} />
        ))}
      </datalist>
      <datalist id={`${listId}-conditions`}>
        {authoringFields
          .filter((field) => field.conditionable)
          .map((field) => (
            <option value={field.path} key={field.path} label={field.label} />
          ))}
      </datalist>
      {fields}
    </span>
  );
}

function selectionFields({
  block,
  disabled,
  listId,
  scope,
  onChange,
}: {
  block: Block;
  disabled?: boolean;
  listId: string;
  scope: ShopifyAuthoringScope;
  onChange(block: Block): void;
}): ReactNode {
  const allPaths = `${listId}-all`;
  if (block.type === "paragraph" || block.type === "heading") {
    const restyle = (style: BlockTextStyle): Block =>
      block.type === "heading"
        ? {
            type: "heading",
            level: block.level ?? 2,
            content: block.content,
            style,
          }
        : { type: "paragraph", content: block.content, style };
    return (
      <>
        <BarSelect
          label="Style"
          value={block.type === "heading" ? `h${block.level ?? 2}` : "body"}
          disabled={disabled}
          options={[
            ["body", "Body text"],
            ["h1", "Heading 1"],
            ["h2", "Heading 2"],
            ["h3", "Heading 3"],
          ]}
          onChange={(next) =>
            onChange(
              next === "body"
                ? {
                    type: "paragraph",
                    content: block.content,
                    ...(block.style ? { style: block.style } : {}),
                  }
                : {
                    type: "heading",
                    level: Number(next.slice(1)),
                    content: block.content,
                    ...(block.style ? { style: block.style } : {}),
                  },
            )
          }
        />
        <BarSegmented
          label="Alignment"
          value={block.style?.align ?? "left"}
          disabled={disabled}
          options={[
            ["left", "Left"],
            ["center", "Centre"],
            ["right", "Right"],
          ]}
          onChange={(align) =>
            restyleAlign(block.style, align, restyle, onChange)
          }
        />
      </>
    );
  }
  if (block.type === "table")
    return (
      <>
        <BarPath
          label="Line items"
          list={allPaths}
          value={authoringExpressionLabel(block.items, scope)}
          disabled={disabled}
          onChange={(items) =>
            onChange({
              ...block,
              items: authoringPathExpression(items, scope),
            })
          }
        />
        <BarToggle
          label="Repeat header on every page"
          checked={block.repeat_header ?? false}
          disabled={disabled}
          onChange={(repeat_header) => onChange({ ...block, repeat_header })}
        />
      </>
    );
  if (block.type === "repeat")
    return (
      <>
        <BarPath
          label="Repeat for each"
          list={allPaths}
          value={authoringExpressionLabel(block.items, scope)}
          disabled={disabled}
          onChange={(items) =>
            onChange({
              ...block,
              items: authoringPathExpression(items, scope),
            })
          }
        />
        <BarNumber
          label="Gap"
          unit="mm"
          value={block.gap_mm ?? 0}
          min={0}
          max={40}
          disabled={disabled}
          onChange={(gap_mm) => onChange({ ...block, gap_mm })}
        />
      </>
    );
  if (block.type === "conditional")
    return (
      <>
        <BarPath
          label="Show when"
          list={`${listId}-conditions`}
          value={authoringExpressionLabel(block.condition, scope)}
          disabled={disabled}
          onChange={(condition) =>
            onChange({
              ...block,
              condition: authoringPathExpression(condition, scope),
            })
          }
        />
        {block.else?.length ? null : (
          <button
            className="piqae-bar-button"
            type="button"
            disabled={disabled}
            onClick={() =>
              onChange({
                ...block,
                else: [
                  {
                    type: "paragraph",
                    content: [{ type: "text", value: "Shown otherwise" }],
                  },
                ],
              })
            }
          >
            <Icon name="plus" />
            Otherwise branch
          </button>
        )}
      </>
    );
  if (block.type === "grid")
    return (
      <>
        <BarText
          label="Column widths"
          value={block.columns.join(", ")}
          disabled={disabled}
          onChange={(next) => {
            const columns = next
              .split(",")
              .map(Number)
              .filter((width) => Number.isFinite(width) && width > 0);
            if (columns.length) onChange({ ...block, columns });
          }}
        />
        <BarNumber
          label="Gap"
          unit="mm"
          value={block.gap_mm ?? 0}
          min={0}
          max={40}
          disabled={disabled}
          onChange={(gap_mm) => onChange({ ...block, gap_mm })}
        />
      </>
    );
  if (
    block.type === "stack" ||
    block.type === "row" ||
    block.type === "section"
  )
    return (
      <BarNumber
        label="Gap"
        unit="mm"
        value={block.gap_mm ?? 0}
        min={0}
        max={40}
        disabled={disabled}
        onChange={(gap_mm) => onChange({ ...block, gap_mm })}
      />
    );
  if (block.type === "image")
    return (
      <>
        <BarText
          label="Source"
          value={block.resource}
          disabled={disabled}
          onChange={(resource) => onChange({ ...block, resource })}
        />
        <BarNumber
          label="Width"
          unit="mm"
          value={block.width_mm}
          min={1}
          max={210}
          disabled={disabled}
          onChange={(width_mm) => onChange({ ...block, width_mm })}
        />
        <BarNumber
          label="Height"
          unit="mm"
          value={block.height_mm}
          min={1}
          max={297}
          disabled={disabled}
          onChange={(height_mm) => onChange({ ...block, height_mm })}
        />
        <BarSelect
          label="Fit"
          value={block.fit ?? "contain"}
          disabled={disabled}
          options={[
            ["contain", "Fit inside"],
            ["fill", "Fill frame"],
            ["scale_down", "Only scale down"],
          ]}
          onChange={(fit) =>
            onChange({
              ...block,
              fit: fit as "contain" | "fill" | "scale_down",
            })
          }
        />
      </>
    );
  if (block.type === "spacer")
    return (
      <BarNumber
        label="Height"
        unit="mm"
        value={block.height_mm}
        min={1}
        max={100}
        disabled={disabled}
        onChange={(height_mm) => onChange({ ...block, height_mm })}
      />
    );
  if (block.type === "qr")
    return (
      <>
        <BarPath
          label="Value"
          list={allPaths}
          value={authoringExpressionLabel(block.value, scope)}
          disabled={disabled}
          onChange={(value) =>
            onChange({
              ...block,
              value: authoringPathExpression(value, scope),
            })
          }
        />
        <BarNumber
          label="Size"
          unit="mm"
          value={block.size_mm}
          min={10}
          max={80}
          disabled={disabled}
          onChange={(size_mm) => onChange({ ...block, size_mm })}
        />
        <BarSelect
          label="Error correction"
          value={block.error_correction ?? "M"}
          disabled={disabled}
          options={[
            ["L", "L"],
            ["M", "M"],
            ["Q", "Q"],
            ["H", "H"],
          ]}
          onChange={(level) =>
            onChange({
              ...block,
              error_correction: level as "L" | "M" | "Q" | "H",
            })
          }
        />
      </>
    );
  if (block.type === "barcode")
    return (
      <>
        <BarPath
          label="Value"
          list={allPaths}
          value={authoringExpressionLabel(block.value, scope)}
          disabled={disabled}
          onChange={(value) =>
            onChange({
              ...block,
              value: authoringPathExpression(value, scope),
            })
          }
        />
        <BarNumber
          label="Width"
          unit="mm"
          value={block.width_mm}
          min={20}
          max={180}
          disabled={disabled}
          onChange={(width_mm) => onChange({ ...block, width_mm })}
        />
        <BarNumber
          label="Height"
          unit="mm"
          value={block.height_mm}
          min={8}
          max={80}
          disabled={disabled}
          onChange={(height_mm) => onChange({ ...block, height_mm })}
        />
        <BarToggle
          label="Show value below"
          checked={block.human_readable ?? false}
          disabled={disabled}
          onChange={(human_readable) => onChange({ ...block, human_readable })}
        />
      </>
    );
  return null;
}

type BlockTextStyle = NonNullable<
  Extract<Block, { type: "paragraph" }>["style"]
>;

function restyleAlign(
  style: BlockTextStyle | undefined,
  align: string,
  restyle: (style: BlockTextStyle) => Block,
  onChange: (block: Block) => void,
) {
  onChange(restyle({ ...style, align: align as BlockTextStyle["align"] }));
}

function BarField({ label, children }: { label: string; children: ReactNode }) {
  return (
    <label className="piqae-bar-field">
      <span>{label}</span>
      {children}
    </label>
  );
}

function BarText({
  label,
  value,
  disabled,
  onChange,
}: {
  label: string;
  value: string;
  disabled?: boolean;
  onChange(value: string): void;
}) {
  return (
    <BarField label={label}>
      <input
        className="piqae-bar-input"
        type="text"
        value={value}
        disabled={disabled}
        onChange={(event) => onChange(event.currentTarget.value)}
      />
    </BarField>
  );
}

function BarPath({
  label,
  value,
  list,
  disabled,
  onChange,
}: {
  label: string;
  value: string;
  list: string;
  disabled?: boolean;
  onChange(value: string): void;
}) {
  return (
    <BarField label={label}>
      <input
        className="piqae-bar-input piqae-bar-path"
        type="text"
        list={list}
        spellCheck={false}
        value={value}
        disabled={disabled}
        onChange={(event) => onChange(event.currentTarget.value)}
      />
    </BarField>
  );
}

function BarNumber({
  label,
  value,
  min,
  max,
  unit,
  disabled,
  onChange,
}: {
  label: string;
  value: number;
  min: number;
  max: number;
  unit?: string;
  disabled?: boolean;
  onChange(value: number): void;
}) {
  return (
    <BarField label={unit ? `${label} (${unit})` : label}>
      <input
        className="piqae-bar-input piqae-bar-number"
        type="number"
        min={min}
        max={max}
        value={value}
        disabled={disabled}
        onChange={(event) => onChange(Number(event.currentTarget.value))}
      />
    </BarField>
  );
}

function BarSelect({
  label,
  value,
  options,
  disabled,
  onChange,
}: {
  label: string;
  value: string;
  options: readonly (readonly [string, string])[];
  disabled?: boolean;
  onChange(value: string): void;
}) {
  return (
    <BarField label={label}>
      <select
        className="piqae-bar-input piqae-bar-select"
        value={value}
        disabled={disabled}
        onChange={(event) => onChange(event.currentTarget.value)}
      >
        {options.map(([optionValue, optionLabel]) => (
          <option value={optionValue} key={optionValue}>
            {optionLabel}
          </option>
        ))}
      </select>
    </BarField>
  );
}

function BarSegmented({
  label,
  value,
  options,
  disabled,
  onChange,
}: {
  label: string;
  value: string;
  options: readonly (readonly [string, string])[];
  disabled?: boolean;
  onChange(value: string): void;
}) {
  return (
    <span className="piqae-bar-field">
      <span>{label}</span>
      <span className="piqae-bar-segmented" role="group" aria-label={label}>
        {options.map(([optionValue, optionLabel]) => (
          <button
            key={optionValue}
            type="button"
            aria-pressed={value === optionValue}
            disabled={disabled}
            onClick={() => onChange(optionValue)}
          >
            {optionLabel}
          </button>
        ))}
      </span>
    </span>
  );
}

function BarToggle({
  label,
  checked,
  disabled,
  onChange,
}: {
  label: string;
  checked: boolean;
  disabled?: boolean;
  onChange(checked: boolean): void;
}) {
  return (
    <label className="piqae-bar-toggle">
      <input
        type="checkbox"
        checked={checked}
        disabled={disabled}
        onChange={(event) => onChange(event.currentTarget.checked)}
      />
      <span>{label}</span>
    </label>
  );
}

function editableInlineWithScope(content: Inline[], currentAlias: string) {
  return content
    .map((item) =>
      item.type === "text"
        ? item.value
        : item.type === "line_break"
          ? "\n"
          : `{{ ${item.value.type === "current_path" ? `${currentAlias}.${item.value.path.join(".")}` : expressionLabel(item.value)} }}`,
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
export function parseContextualInline(
  source: string,
  original: Inline[] = [],
  currentAlias: ShopifyAuthoringScope = "order",
): Inline[] {
  const parsed = parseEditableInline(source, original);
  return parsed.map((item) => {
    if (item.type !== "value" || item.value.type !== "path") return item;
    return {
      ...item,
      value: authoringPathExpression(item.value.path.join("."), currentAlias),
    };
  });
}

export function contextualFieldSuggestions(
  fields: readonly ShopifyDocumentField[],
  currentAlias?: ShopifyAuthoringScope,
) {
  if (!currentAlias) return [...fields];
  return fields
    .filter(
      (field) =>
        field.path.startsWith(`${currentAlias}.`) ||
        field.path.startsWith("shop."),
    )
    .sort((left, right) => {
      const leftCurrent = left.path.startsWith(`${currentAlias}.`) ? 0 : 1;
      const rightCurrent = right.path.startsWith(`${currentAlias}.`) ? 0 : 1;
      return leftCurrent - rightCurrent;
    });
}

export function incompleteExpressionQuery(source: string) {
  const open = source.lastIndexOf("{{");
  if (open < 0 || source.slice(open + 2).includes("}}")) return null;
  return source.slice(open + 2).trimStart();
}

export function searchDocumentFields(
  fields: readonly ShopifyDocumentField[],
  query: string,
  limit = 12,
) {
  const terms = query
    .toLowerCase()
    .split(/[\s.]+/)
    .filter(Boolean);
  return fields
    .filter((field) => {
      const haystack =
        `${field.label} ${field.path} ${field.group}`.toLowerCase();
      return terms.every((term) => haystack.includes(term));
    })
    .slice(0, limit);
}

export function completeExpression(source: string, path: string) {
  const open = source.lastIndexOf("{{");
  if (open < 0 || source.slice(open + 2).includes("}}"))
    return `${source}{{ ${path} }}`;
  return `${source.slice(0, open)}{{ ${path} }}`;
}

function valueFromEditor(editor: React.RefObject<HTMLSpanElement | null>) {
  return editor.current?.textContent ?? "";
}

function placeCaretAtEnd(element: HTMLElement | null) {
  if (!element) return;
  const selection = window.getSelection();
  const range = document.createRange();
  range.selectNodeContents(element);
  range.collapse(false);
  selection?.removeAllRanges();
  selection?.addRange(range);
}

function isEditingTarget(target: EventTarget | null) {
  return (
    target instanceof HTMLElement &&
    Boolean(
      target.closest(
        '[contenteditable="true"], input, textarea, select, button, a[href]',
      ),
    )
  );
}

function moveItem<T>(items: T[], index: number, direction: -1 | 1) {
  const target = index + direction;
  if (target < 0 || target >= items.length) return items;
  const next = [...items];
  [next[index], next[target]] = [next[target]!, next[index]!];
  return next;
}

function resizeColumns(
  columns: Extract<Block, { type: "table" }>["columns"],
  index: number,
  delta: number,
) {
  const left = columns[index];
  const right = columns[index + 1];
  if (!left || !right) return columns;
  const leftWidth = left.width ?? 1;
  const rightWidth = right.width ?? 1;
  const minimum = adjacentColumnWidth(columns, index) * 0.1;
  const boundedDelta = Math.max(
    minimum - leftWidth,
    Math.min(rightWidth - minimum, delta),
  );
  return columns.map((column, columnIndex) =>
    columnIndex === index
      ? { ...column, width: leftWidth + boundedDelta }
      : columnIndex === index + 1
        ? { ...column, width: rightWidth - boundedDelta }
        : column,
  );
}

function adjacentColumnWidth(
  columns: Extract<Block, { type: "table" }>["columns"],
  index: number,
) {
  return (columns[index]?.width ?? 1) + (columns[index + 1]?.width ?? 1);
}

function columnBoundaryPercent(
  columns: Extract<Block, { type: "table" }>["columns"],
  index: number,
) {
  const left = columns[index]?.width ?? 1;
  const right = columns[index + 1]?.width ?? 1;
  return Math.round((left / Math.max(left + right, 0.01)) * 100);
}
function resizeGridColumns(columns: number[], index: number, delta: number) {
  const left = columns[index];
  const right = columns[index + 1];
  if (left === undefined || right === undefined) return columns;
  const minimum = adjacentGridWidth(columns, index) * 0.1;
  const boundedDelta = Math.max(
    minimum - left,
    Math.min(right - minimum, delta),
  );
  return columns.map((width, columnIndex) =>
    columnIndex === index
      ? left + boundedDelta
      : columnIndex === index + 1
        ? right - boundedDelta
        : width,
  );
}
function adjacentGridWidth(columns: number[], index: number) {
  return (columns[index] ?? 1) + (columns[index + 1] ?? 1);
}
function gridBoundaryPercent(columns: number[], index: number) {
  const left = columns[index] ?? 1;
  const right = columns[index + 1] ?? 1;
  return Math.round((left / Math.max(left + right, 0.01)) * 100);
}
/** The list a path's final segment indexes into, so move limits are accurate. */
export function siblingsAtPath(blocks: Block[], path: BlockPath): Block[] {
  const [part, ...rest] = path;
  if (!part || !rest.length) return blocks;
  const block = blocks[part.index];
  if (!block) return blocks;
  const next = rest[0]!;
  if (next.branch === "children" && "children" in block)
    return siblingsAtPath(block.children, rest);
  if (next.branch === "then" && block.type === "conditional")
    return siblingsAtPath(block.then, rest);
  if (next.branch === "else" && block.type === "conditional")
    return siblingsAtPath(block.else ?? [], rest);
  return blocks;
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

/** Treat the mandatory orders repeat as document framing, not author content. */
function isTopLevelAuthoringPath(path: BlockPath) {
  return (
    (path.length === 1 && path[0]?.branch === "root") ||
    (path.length === 2 &&
      path[0]?.branch === "root" &&
      path[0].index === 0 &&
      path[1]?.branch === "children")
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
export function insertBlockAtPath(
  blocks: Block[],
  path: BlockPath,
  inserted: Block,
): Block[] {
  const [part, ...rest] = path;
  if (!part) return [...blocks, inserted];
  if (!rest.length) {
    const next = [...blocks];
    next.splice(Math.min(Math.max(part.index, 0), next.length), 0, inserted);
    return next;
  }
  return blocks.map((block, index) => {
    if (index !== part.index) return block;
    const nextPart = rest[0]!;
    if (nextPart.branch === "children" && "children" in block)
      return {
        ...block,
        children: insertBlockAtPath(block.children, rest, inserted),
      };
    if (nextPart.branch === "then" && block.type === "conditional")
      return {
        ...block,
        then: insertBlockAtPath(block.then, rest, inserted),
      };
    if (nextPart.branch === "else" && block.type === "conditional")
      return {
        ...block,
        else: insertBlockAtPath(block.else ?? [], rest, inserted),
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
function blockIcon(block: Block): IconName {
  const icons: Partial<Record<Block["type"], IconName>> = {
    table: "table",
    repeat: "repeat",
    conditional: "condition",
    grid: "columns",
    stack: "stack",
    section: "stack",
    keep_together: "stack",
    row: "row",
    qr: "qr",
    barcode: "barcode",
    image: "image",
    spacer: "spacer",
    divider: "divider",
    page_break: "divider",
    heading: "heading",
    paragraph: "text",
  };
  return icons[block.type] ?? "text";
}
function scopeForBlockPath(
  blocks: Block[],
  path: BlockPath,
): ShopifyAuthoringScope {
  let scope: ShopifyAuthoringScope = "order";
  let siblings = blocks;
  for (const [depth, part] of path.entries()) {
    const block = siblings[part.index];
    if (!block) break;
    const next = path[depth + 1];
    if (!next) return scope;
    if (
      block.type === "repeat" &&
      next.branch === "children" &&
      isLineItemsExpression(block.items, scope)
    )
      scope = "item";
    siblings =
      next.branch === "then" && block.type === "conditional"
        ? block.then
        : next.branch === "else" && block.type === "conditional"
          ? (block.else ?? [])
          : "children" in block
            ? block.children
            : [];
  }
  return scope;
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
          ? { level: block.level ?? 2, align: block.style?.align ?? "left" }
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
      const alignStyle =
        node.attrs.align && !["start", "left"].includes(node.attrs.align)
          ? {
              style: {
                align:
                  node.attrs.align === "end"
                    ? ("right" as const)
                    : (node.attrs.align as TextStyle["align"]),
              },
            }
          : {};
      result.push(
        node.type.name === "heading"
          ? {
              type: "heading",
              level: node.attrs.level,
              ...alignStyle,
              content,
            }
          : { type: "paragraph", ...alignStyle, content },
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
function authoringExpressionLabel(
  value: Expression,
  scope: ShopifyAuthoringScope,
) {
  return value.type === "current_path"
    ? [scope, ...value.path].join(".")
    : expressionLabel(value);
}
function hasPiqaeBlockDrag(dataTransfer: DataTransfer) {
  return Array.from(dataTransfer.types).includes(PIQAE_BLOCK_DRAG_TYPE);
}
function draggedBlock(dataTransfer: DataTransfer): Block | null {
  const source = dataTransfer.getData(PIQAE_BLOCK_DRAG_TYPE);
  if (!source || source.length > 65_536) return null;
  try {
    const value = JSON.parse(source) as unknown;
    if (!value || typeof value !== "object") return null;
    const type = (value as { type?: unknown }).type;
    if (typeof type !== "string" || !DRAGGABLE_BLOCK_TYPES.has(type))
      return null;
    return value as Block;
  } catch {
    return null;
  }
}
const DRAGGABLE_BLOCK_TYPES = new Set([
  "section",
  "stack",
  "row",
  "box",
  "paragraph",
  "heading",
  "grid",
  "table",
  "repeat",
  "data_list",
  "conditional",
  "spacer",
  "divider",
  "page_break",
  "keep_together",
  "image",
  "image_value",
  "qr",
  "barcode",
]);
function quickInsertBlock(type: QuickInsertType): Block {
  if (type === "paragraph")
    return {
      type: "paragraph",
      content: [{ type: "text", value: "Start typing…" }],
    };
  if (type === "heading")
    return {
      type: "heading",
      level: 2,
      content: [{ type: "text", value: "Heading" }],
    };
  if (type === "image")
    return {
      type: "image",
      resource: "shop.logo",
      width_mm: 42,
      height_mm: 18,
      fit: "contain",
    };
  if (type === "divider") return { type: "divider" };
  return { type: "spacer", height_mm: 6 };
}
function quickInsertIcon(type: QuickInsertType): IconName {
  if (type === "paragraph") return "text";
  if (type === "heading") return "heading";
  return type;
}
function quickInsertLabel(type: QuickInsertType) {
  if (type === "paragraph") return "Text";
  if (type === "heading") return "Heading";
  if (type === "spacer") return "Spacing";
  return type[0]!.toUpperCase() + type.slice(1);
}
function defaultQrCode(): Block {
  return {
    type: "qr",
    value: currentPathExpression("statusUrl"),
    size_mm: 24,
  };
}
function defaultBarcode(): Block {
  return {
    type: "barcode",
    value: currentPathExpression("name"),
    symbology: "code128",
    width_mm: 48,
    height_mm: 16,
    human_readable: true,
  };
}
function defaultTable(): Block {
  const current = (key: string): Expression =>
    ({ type: "current_path", path: [key] }) as Expression;
  return {
    type: "table",
    items: currentPathExpression("lineItems"),
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
    condition: currentPathExpression("note"),
    then: [
      {
        type: "paragraph",
        content: [{ type: "value", value: currentPathExpression("note") }],
      },
    ],
    else: [],
  };
}
function defaultRepeat(): Block {
  return {
    type: "repeat",
    items: currentPathExpression("lineItems"),
    gap_mm: 4,
    children: [
      {
        type: "paragraph",
        content: [
          { type: "value", value: currentPathExpression("title") },
          { type: "text", value: " × " },
          { type: "value", value: currentPathExpression("quantity") },
        ],
      },
    ],
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
