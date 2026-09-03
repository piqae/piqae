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
export type PickedShopifyImage = {
  resourceKey: string;
  resource: NonNullable<PrintPacket["resources"]>[string];
};
type TableEditorBlock = Extract<Block, { type: "table" }>;
type DocumentRegion = "body" | "header" | "footer";
type CanvasSelectionTarget =
  | { kind: "table_cell"; columnIndex: number }
  | { kind: "table_row" };
type TableColumnResizeDrag = {
  index: number;
  grabOffsetPx: number;
  delta: number;
  columns: TableEditorBlock["columns"];
};

const PIQAE_BLOCK_DRAG_TYPE = "application/x-piqae-printpacket-block";
const QUICK_INSERT_TYPES = [
  "paragraph",
  "heading",
  "image",
  "divider",
  "spacer",
] as const;
type QuickInsertType = (typeof QUICK_INSERT_TYPES)[number];
const DRAG_INSERT_TYPES = [
  ...QUICK_INSERT_TYPES,
  "table",
  "repeat",
  "conditional",
  "qr",
  "barcode",
  "grid",
  "stack",
  "row",
] as const;
type DragInsertType = (typeof DRAG_INSERT_TYPES)[number];

function documentRegionBlocks(
  document: PrintPacket,
  region: DocumentRegion,
): Block[] {
  return region === "body" ? document.body : (document[region]?.default ?? []);
}

function withDocumentRegionBlocks(
  document: PrintPacket,
  region: DocumentRegion,
  blocks: Block[],
): PrintPacket {
  if (region === "body")
    return { ...document, body: canonicalizeShopifyEditorBody(blocks) };
  return {
    ...document,
    [region]: {
      first: document[region]?.first ?? [],
      default: blocks,
      last: document[region]?.last ?? [],
    },
  };
}

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

const EDITOR_HISTORY_LIMIT = 100;
type EditorHistoryEntry = { document: PrintPacket; key: string };
type EditorDocumentHistory = {
  past: EditorHistoryEntry[];
  present: EditorHistoryEntry;
  future: EditorHistoryEntry[];
};

/**
 * A document-scoped history owner. Keep this object above workspace routing so
 * temporarily unmounting Design (for example, while showing Preview) does not
 * discard the merchant's undo stack.
 */
export type PrintPacketEditorHistory = EditorDocumentHistory;

function editorHistoryEntry(document: PrintPacket): EditorHistoryEntry {
  const snapshot = structuredClone(document);
  return { document: snapshot, key: JSON.stringify(snapshot) };
}

function createEditorHistory(document: PrintPacket): EditorDocumentHistory {
  return { past: [], present: editorHistoryEntry(document), future: [] };
}

export function createPrintPacketEditorHistory(
  document: PrintPacket,
): PrintPacketEditorHistory {
  return createEditorHistory(document);
}

function recordEditorHistory(
  historyState: EditorDocumentHistory,
  document: PrintPacket,
) {
  const next = editorHistoryEntry(document);
  if (next.key === historyState.present.key) return false;
  historyState.past.push(historyState.present);
  if (historyState.past.length > EDITOR_HISTORY_LIMIT)
    historyState.past.splice(
      0,
      historyState.past.length - EDITOR_HISTORY_LIMIT,
    );
  historyState.present = next;
  historyState.future = [];
  return true;
}

function stepEditorHistory(
  historyState: EditorDocumentHistory,
  direction: "undo" | "redo",
) {
  const source = direction === "undo" ? historyState.past : historyState.future;
  const next = source.pop();
  if (!next) return null;
  const destination =
    direction === "undo" ? historyState.future : historyState.past;
  destination.push(historyState.present);
  if (destination.length > EDITOR_HISTORY_LIMIT)
    destination.splice(0, destination.length - EDITOR_HISTORY_LIMIT);
  historyState.present = next;
  return structuredClone(next.document);
}

export function PrintPacketEditor({
  value,
  disabled,
  customFields = [],
  stock = null,
  resourcePreviewUrls = {},
  workspaceControls,
  history: sharedHistory,
  onPickShopifyImage,
  onChange,
}: {
  value: PrintPacket;
  disabled?: boolean;
  customFields?: readonly ShopifyDocumentField[];
  stock?: DesignStock;
  resourcePreviewUrls?: Readonly<Record<string, string>>;
  workspaceControls?: ReactNode;
  history?: PrintPacketEditorHistory;
  onPickShopifyImage?: () => Promise<PickedShopifyImage | null>;
  onChange(document: PrintPacket): void;
}) {
  const allAuthoringFields = [...AUTHORING_FIELDS, ...customFields];
  const canonicalBody = canonicalizeShopifyEditorBody(value.body);
  const currentDocument = { ...value, body: canonicalBody };
  const currentDocumentKey = JSON.stringify(currentDocument);
  const continuousPageBreaks =
    value.media.kind === "continuous" && documentHasPageBreak(value);
  const showsOrderPageBoundary =
    value.media.kind === "paged" &&
    canonicalBody.some(
      (block, index) =>
        orderBatchPresentation(
          block,
          [{ branch: "root", index }],
          value.media.kind,
        ) === "one_order_per_page",
    );
  const editorRoot = useRef<HTMLDivElement>(null);
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView | null>(null);
  const latest = useRef(currentDocument);
  const observedValueKey = useRef(currentDocumentKey);
  const localHistory = useRef<EditorDocumentHistory | null>(null);
  if (!localHistory.current)
    localHistory.current = createEditorHistory(currentDocument);
  const documentHistory = sharedHistory ?? localHistory.current;
  if (observedValueKey.current !== currentDocumentKey) {
    observedValueKey.current = currentDocumentKey;
    latest.current = currentDocument;
  }
  const [historyAvailability, setHistoryAvailability] = useState(() => ({
    canUndo: documentHistory.past.length > 0,
    canRedo: documentHistory.future.length > 0,
  }));
  const [compactAnnotations, setCompactAnnotations] = useState(() =>
    compactCanvasAnnotations(value),
  );
  const [activeRegion, setActiveRegion] = useState<DocumentRegion>("body");
  const [selection, setSelection] = useState<{
    position: number;
    block: Block;
    path?: BlockPath;
    region: DocumentRegion;
    target?: CanvasSelectionTarget;
  } | null>(null);
  const selectedRegionBlocks = documentRegionBlocks(
    currentDocument,
    selection?.region ?? activeRegion,
  );
  const insertionScope =
    selection?.target?.kind === "table_cell"
      ? "item"
      : selection?.path
        ? scopeForBlockPath(selectedRegionBlocks, selection.path)
        : "order";
  const insertionFields = contextualFieldSuggestions(
    allAuthoringFields,
    insertionScope,
  );
  const syncHistoryAvailability = () => {
    setHistoryAvailability({
      canUndo: documentHistory.past.length > 0,
      canRedo: documentHistory.future.length > 0,
    });
  };
  const publishEditorDocument = (document: PrintPacket) => {
    latest.current = document;
    if (!recordEditorHistory(documentHistory, document)) return;
    syncHistoryAvailability();
    onChange(document);
  };
  const applyHistoryStep = (direction: "undo" | "redo") => {
    const document = stepEditorHistory(documentHistory, direction);
    if (!document) return;
    latest.current = document;
    view.current?.updateState(
      EditorState.create({
        schema,
        doc: blocksToDoc(document.body),
        plugins: [
          history(),
          keymap({ "Mod-z": undo, "Shift-Mod-z": redo }),
          keymap(baseKeymap),
        ],
      }),
    );
    setSelection(null);
    setActiveRegion("body");
    syncHistoryAvailability();
    onChange(document);
  };
  const undoDocument = () => applyHistoryStep("undo");
  const redoDocument = () => applyHistoryStep("redo");
  const handleHistoryShortcut = (
    event: React.KeyboardEvent | KeyboardEvent,
  ) => {
    if (
      disabled ||
      event.defaultPrevented ||
      event.altKey ||
      nativeHistoryTarget(event.target)
    )
      return;
    const key = event.key.toLowerCase();
    const modified = event.metaKey || event.ctrlKey;
    const wantsUndo = modified && key === "z" && !event.shiftKey;
    const wantsRedo =
      (modified && key === "z" && event.shiftKey) ||
      (event.ctrlKey && !event.metaKey && !event.shiftKey && key === "y");
    if (
      (!wantsUndo && !wantsRedo) ||
      (wantsUndo && !historyAvailability.canUndo) ||
      (wantsRedo && !historyAvailability.canRedo)
    )
      return;
    event.preventDefault();
    event.stopPropagation();
    if (wantsUndo) undoDocument();
    else redoDocument();
  };
  useEffect(() => {
    const handleDocumentHistoryShortcut = (event: KeyboardEvent) => {
      const target = event.target;
      if (
        !(target instanceof Node) ||
        (target !== document.body && !editorRoot.current?.contains(target))
      )
        return;
      handleHistoryShortcut(event);
    };
    document.addEventListener("keydown", handleDocumentHistoryShortcut);
    return () =>
      document.removeEventListener("keydown", handleDocumentHistoryShortcut);
  });
  useEffect(() => {
    if (documentHistory.present.key === currentDocumentKey) return;
    const reset = createEditorHistory(currentDocument);
    documentHistory.past = reset.past;
    documentHistory.present = reset.present;
    documentHistory.future = reset.future;
    setHistoryAvailability({ canUndo: false, canRedo: false });
    setSelection(null);
    setActiveRegion("body");
  }, [currentDocumentKey, documentHistory]);
  useEffect(() => {
    const instance = view.current;
    if (!instance) return;
    const nextDocument = blocksToDoc(canonicalBody);
    if (instance.state.doc.eq(nextDocument)) return;
    instance.updateState(
      EditorState.create({
        schema,
        doc: nextDocument,
        plugins: [
          history(),
          keymap({ "Mod-z": undo, "Shift-Mod-z": redo }),
          keymap(baseKeymap),
        ],
      }),
    );
    setSelection(null);
    setActiveRegion("body");
  }, [currentDocumentKey]);
  useEffect(() => {
    if (!selection && activeRegion === "body") return;
    const clearSelectionOnEscape = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || event.defaultPrevented) return;
      const target = event.target;
      const root = editorRoot.current;
      if (!(target instanceof Node) || !root?.contains(target)) {
        setSelection(null);
        setActiveRegion("body");
        return;
      }
      if (editorInputTarget(target)) return;
      event.preventDefault();
      setSelection(null);
      setActiveRegion("body");
    };
    const clearSelectionAwayFromEditorActions = (event: PointerEvent) => {
      const target = event.target;
      if (!(target instanceof Element)) return;
      const root = editorRoot.current;
      if (root?.contains(target) && target.closest(".piqae-editor-toolbar"))
        return;
      const region = target.closest<HTMLElement>("[data-document-region]")
        ?.dataset.documentRegion as DocumentRegion | undefined;
      if (region === activeRegion) {
        if (
          root?.contains(target) &&
          target.closest(
            ".piqae-canvas-selected, .piqae-canvas-insertion-slot, .piqae-canvas-column-resize",
          )
        )
          return;
        setSelection(null);
        return;
      }
      setSelection(null);
      setActiveRegion("body");
    };
    document.addEventListener("keydown", clearSelectionOnEscape);
    document.addEventListener(
      "pointerdown",
      clearSelectionAwayFromEditorActions,
      true,
    );
    return () => {
      document.removeEventListener("keydown", clearSelectionOnEscape);
      document.removeEventListener(
        "pointerdown",
        clearSelectionAwayFromEditorActions,
        true,
      );
    };
  }, [activeRegion, selection]);
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
          region: "body",
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
                region: "body",
              }
            : null,
        );
        const body = canonicalizeShopifyEditorBody(docToBlocks(next.doc));
        latest.current = { ...latest.current, body };
        publishEditorDocument(latest.current);
      },
    });
    return () => {
      view.current?.destroy();
      view.current = null;
    };
  }, [disabled]);
  const syncBodyEditor = (document: PrintPacket) =>
    view.current?.updateState(
      EditorState.create({
        schema,
        doc: blocksToDoc(document.body),
        plugins: [
          history(),
          keymap({ "Mod-z": undo, "Shift-Mod-z": redo }),
          keymap(baseKeymap),
        ],
      }),
    );
  const insert = (node: ProseMirrorNode) => {
    const instance = view.current;
    if (!instance) return;
    const inserted = docToBlocks(schema.nodes.doc!.create(null, [node]))[0];
    if (!inserted) return;
    const region = selection?.region ?? activeRegion;
    if (selection?.path) {
      const regionBlocks = documentRegionBlocks(latest.current, region);
      const insertedInCell = insertBlockIntoGridCell(
        regionBlocks,
        selection.path,
        inserted,
      );
      if (insertedInCell) {
        const nextDocument = withDocumentRegionBlocks(
          latest.current,
          region,
          insertedInCell,
        );
        latest.current = nextDocument;
        if (region === "body") syncBodyEditor(nextDocument);
        setSelection(null);
        publishEditorDocument(nextDocument);
        return;
      }
    }
    if (
      selection?.path &&
      selection.target?.kind === "table_cell" &&
      selection.block.type === "table"
    ) {
      const converted = tableToRichDataList(
        selection.block,
        selection.target.columnIndex,
        inserted,
      );
      const blocks = replaceBlockAtPath(
        documentRegionBlocks(latest.current, region),
        selection.path,
        converted,
      );
      const nextDocument = withDocumentRegionBlocks(
        latest.current,
        region,
        blocks,
      );
      latest.current = nextDocument;
      if (region === "body") syncBodyEditor(nextDocument);
      setSelection(null);
      publishEditorDocument(nextDocument);
      return;
    }
    if (
      selection?.path &&
      (selection.block.type === "stack" ||
        selection.block.type === "row" ||
        selection.block.type === "section" ||
        selection.block.type === "box" ||
        selection.block.type === "keep_together")
    ) {
      const container = {
        ...selection.block,
        children: [...selection.block.children, inserted],
      } as Block;
      const blocks = replaceBlockAtPath(
        documentRegionBlocks(latest.current, region),
        selection.path,
        container,
      );
      const nextDocument = withDocumentRegionBlocks(
        latest.current,
        region,
        blocks,
      );
      latest.current = nextDocument;
      if (region === "body") syncBodyEditor(nextDocument);
      setSelection({ ...selection, block: container });
      publishEditorDocument(nextDocument);
      return;
    }
    if (region !== "body") {
      const blocks = documentRegionBlocks(latest.current, region);
      const selectedPath = selection?.region === region ? selection.path : null;
      const nextBlocks = selectedPath
        ? insertBlockAfterPath(blocks, selectedPath, inserted)
        : [...blocks, inserted];
      const nextDocument = withDocumentRegionBlocks(
        latest.current,
        region,
        nextBlocks,
      );
      latest.current = nextDocument;
      publishEditorDocument(nextDocument);
      return;
    }
    const selectedPath = selection?.path;
    const authoredBody = selectedPath
      ? isProtectedOrderPageBreakPath(
          latest.current.body,
          selectedPath,
          latest.current.media.kind,
        )
        ? insertBeforeTerminalOrderPageBreak(
            latest.current.body,
            inserted,
            latest.current.media.kind,
          )
        : insertBlockAfterPath(latest.current.body, selectedPath, inserted)
      : insertBeforeTerminalOrderPageBreak(
          latest.current.body,
          inserted,
          latest.current.media.kind,
        );
    const body = canonicalizeShopifyEditorBody(authoredBody);
    const nextDocument = { ...latest.current, body };
    latest.current = nextDocument;
    instance.updateState(
      EditorState.create({ schema, doc: blocksToDoc(body) }),
    );
    publishEditorDocument(nextDocument);
    instance.focus();
  };
  const insertAtPath = (
    block: Block,
    path: BlockPath,
    region: DocumentRegion = activeRegion,
  ) => {
    const blocks = insertBlockAtPath(
      documentRegionBlocks(latest.current, region),
      path,
      block,
    );
    const nextDocument = withDocumentRegionBlocks(
      latest.current,
      region,
      blocks,
    );
    latest.current = nextDocument;
    if (region === "body") syncBodyEditor(nextDocument);
    setSelection({ position: -1, path, block, region });
    publishEditorDocument(nextDocument);
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
      if (
        selection.region === "body" &&
        isProtectedOrderPageBreakPath(
          latest.current.body,
          selection.path,
          latest.current.media.kind,
        )
      ) {
        setSelection(null);
        return;
      }
      const blocks = replaceBlockAtPath(
        documentRegionBlocks(latest.current, selection.region),
        selection.path,
        block,
      );
      const nextDocument = withDocumentRegionBlocks(
        latest.current,
        selection.region,
        blocks,
      );
      latest.current = nextDocument;
      if (selection.region === "body") syncBodyEditor(nextDocument);
      setSelection({
        position: -1,
        path: selection.path,
        block,
        region: selection.region,
      });
      publishEditorDocument(nextDocument);
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
  const insertShopifyImage = async () => {
    const picked = await onPickShopifyImage?.();
    if (!picked || disabled) return;
    latest.current = {
      ...latest.current,
      resources: {
        ...(latest.current.resources ?? {}),
        [picked.resourceKey]: picked.resource,
      },
    };
    insertBlock({
      type: "image",
      resource: picked.resourceKey,
      width_mm: insertionScope === "item" ? 16 : 42,
      height_mm: insertionScope === "item" ? 16 : 18,
      fit: "contain",
    });
  };
  const removeSelected = () => {
    const instance = view.current;
    if (!instance || !selection) return;
    if (selection.path) {
      if (
        selection.region === "body" &&
        isProtectedOrderPageBreakPath(
          latest.current.body,
          selection.path,
          latest.current.media.kind,
        )
      ) {
        setSelection(null);
        return;
      }
      const blocks = removeBlockAtPath(
        documentRegionBlocks(latest.current, selection.region),
        selection.path,
      );
      const nextDocument = withDocumentRegionBlocks(
        latest.current,
        selection.region,
        blocks,
      );
      latest.current = nextDocument;
      if (selection.region === "body") syncBodyEditor(nextDocument);
      publishEditorDocument(nextDocument);
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
  useEffect(() => {
    if (!selection || disabled) return;
    const removeWithKeyboard = (event: KeyboardEvent) => {
      if (
        event.defaultPrevented ||
        (event.key !== "Delete" && event.key !== "Backspace") ||
        isEditingTarget(event.target)
      )
        return;
      const target = event.target;
      if (
        target instanceof Node &&
        target !== document.body &&
        !editorRoot.current?.contains(target)
      )
        return;
      event.preventDefault();
      removeSelected();
    };
    document.addEventListener("keydown", removeWithKeyboard);
    return () => document.removeEventListener("keydown", removeWithKeyboard);
  }, [disabled, selection]);
  const moveSelected = (direction: -1 | 1) => {
    if (!selection?.path) return;
    const regionBlocks = documentRegionBlocks(latest.current, selection.region);
    if (
      !canMoveBlockAtPath(
        regionBlocks,
        selection.path,
        direction,
        selection.region === "body" ? latest.current.media.kind : "label",
      )
    )
      return;
    const blocks = moveBlockAtPath(regionBlocks, selection.path, direction);
    const nextPath = selection.path.map((part, index) =>
      index === selection.path!.length - 1
        ? { ...part, index: part.index + direction }
        : part,
    );
    const nextDocument = withDocumentRegionBlocks(
      latest.current,
      selection.region,
      blocks,
    );
    latest.current = nextDocument;
    if (selection.region === "body") syncBodyEditor(nextDocument);
    setSelection(
      selection.region === "body" &&
        isProtectedOrderPageBreakPath(
          blocks,
          nextPath,
          latest.current.media.kind,
        )
        ? null
        : {
            position: -1,
            path: nextPath,
            block: selection.block,
            region: selection.region,
          },
    );
    publishEditorDocument(nextDocument);
  };
  const duplicateSelected = () => {
    if (!selection?.path) return;
    if (
      selection.region === "body" &&
      isProtectedOrderPageBreakPath(
        latest.current.body,
        selection.path,
        latest.current.media.kind,
      )
    ) {
      setSelection(null);
      return;
    }
    const blocks = insertBlockAfterPath(
      documentRegionBlocks(latest.current, selection.region),
      selection.path,
      structuredClone(selection.block),
    );
    const nextDocument = withDocumentRegionBlocks(
      latest.current,
      selection.region,
      blocks,
    );
    latest.current = nextDocument;
    if (selection.region === "body") syncBodyEditor(nextDocument);
    publishEditorDocument(nextDocument);
  };
  const focusDocumentRegion = (region: DocumentRegion) => {
    setSelection(null);
    setActiveRegion(region);
  };
  const changeDocumentRegionBlock = (
    region: DocumentRegion,
    block: Block,
    path: BlockPath,
  ) => {
    const blocks = replaceBlockAtPath(
      documentRegionBlocks(latest.current, region),
      path,
      block,
    );
    const nextDocument = withDocumentRegionBlocks(
      latest.current,
      region,
      blocks,
    );
    latest.current = nextDocument;
    setSelection({ position: -1, block, path, region });
    publishEditorDocument(nextDocument);
  };
  const renderRepeatingRegion = (region: "header" | "footer") => {
    const blocks = documentRegionBlocks(currentDocument, region);
    const active = activeRegion === region;
    const name = region === "header" ? "header" : "footer";
    return (
      <section
        className={`piqae-document-region piqae-document-${region}${active ? " is-active" : ""}${activeRegion !== "body" && !active ? " is-dimmed" : ""}`}
        data-document-region={region}
        aria-label={`Repeating page ${name}${active ? ", editing" : ", double-click, Enter, or F2 to edit"}`}
        tabIndex={active ? -1 : 0}
        onDoubleClick={(event) => {
          if (disabled) return;
          event.stopPropagation();
          focusDocumentRegion(region);
        }}
        onKeyDown={(event) => {
          if (
            disabled ||
            active ||
            (event.key !== "Enter" && event.key !== "F2")
          )
            return;
          event.preventDefault();
          event.stopPropagation();
          focusDocumentRegion(region);
        }}
      >
        <span className="piqae-document-region-hit-area" aria-hidden="true" />
        <span className="piqae-document-region-label">
          {active ? `Editing repeating ${name}` : `Repeating ${name}`}
        </span>
        {blocks.length || active ? (
          <DocumentCanvas
            blocks={blocks}
            resourcePreviewUrls={resourcePreviewUrls}
            selectedPath={
              selection?.region === region ? selection.path : undefined
            }
            editable={active && !disabled}
            preview={!active}
            mediaKind={value.media.kind}
            authoringFields={allAuthoringFields}
            onSelect={(block, path, target) =>
              setSelection({ position: -1, block, path, region, target })
            }
            onInsert={(block, path) => insertAtPath(block, path, region)}
            onChange={(block, path) =>
              changeDocumentRegionBlock(region, block, path)
            }
          />
        ) : (
          <span className="piqae-document-region-empty">
            Double-click to add content
          </span>
        )}
      </section>
    );
  };
  return (
    <div
      className="piqae-word-editor"
      ref={editorRoot}
      onKeyDown={handleHistoryShortcut}
    >
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
            aria-label="Edit document"
          >
            <div
              className="piqae-tool-group"
              role="group"
              aria-label="Document history"
            >
              <ToolButton
                icon="undo"
                label="Undo"
                ariaKeyShortcuts="Control+Z Meta+Z"
                disabled={disabled || !historyAvailability.canUndo}
                onClick={undoDocument}
              />
              <ToolButton
                icon="redo"
                label="Redo"
                ariaKeyShortcuts="Control+Y Control+Shift+Z Meta+Shift+Z"
                disabled={disabled || !historyAvailability.canRedo}
                onClick={redoDocument}
              />
            </div>
            <span className="piqae-tool-divider" />
            <div
              className="piqae-tool-group"
              role="group"
              aria-label="Canvas annotations"
            >
              <ToolButton
                icon="annotations"
                label={
                  compactAnnotations
                    ? "Show detailed logic"
                    : "Use compact logic markers"
                }
                pressed={!compactAnnotations}
                onClick={() => setCompactAnnotations((value) => !value)}
              />
            </div>
            <span className="piqae-tool-divider" />
            <div className="piqae-tool-group">
              <ToolButton
                icon="text"
                label="Text"
                disabled={disabled}
                dragType="paragraph"
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
                dragType="heading"
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
                dragType="table"
                onClick={() => insertBlock(defaultTable())}
              />
              <ToolButton
                icon="repeat"
                label="Repeating content"
                disabled={disabled}
                dragType="repeat"
                onClick={() => insertBlock(defaultRepeat())}
              />
              <ToolButton
                icon="condition"
                label="Conditional section"
                disabled={disabled}
                dragType="conditional"
                onClick={() => insertBlock(defaultConditional())}
              />
            </div>
            <span className="piqae-tool-divider" />
            <div className="piqae-tool-group">
              <InsertImageButton
                disabled={disabled}
                canPickShopify={Boolean(onPickShopifyImage)}
                onInsertDynamic={() =>
                  insertBlock(defaultImage(insertionScope))
                }
                onInsertShopify={insertShopifyImage}
              />
              <ToolButton
                icon="qr"
                label="QR code"
                disabled={disabled}
                dragType="qr"
                onClick={() => insertBlock(defaultQrCode())}
              />
              <ToolButton
                icon="barcode"
                label="Barcode"
                disabled={disabled}
                dragType="barcode"
                onClick={() => insertBlock(defaultBarcode(insertionScope))}
              />
            </div>
            <span className="piqae-tool-divider" />
            <div className="piqae-tool-group">
              <ToolButton
                icon="columns"
                label="Columns"
                disabled={disabled}
                dragType="grid"
                onClick={() => insertBlock(defaultGrid())}
              />
              <ToolButton
                icon="stack"
                label="Stack"
                disabled={disabled}
                dragType="stack"
                onClick={() => insertBlock(defaultContainer("stack"))}
              />
              <ToolButton
                icon="row"
                label="Row"
                disabled={disabled}
                dragType="row"
                onClick={() => insertBlock(defaultContainer("row"))}
              />
              <ToolButton
                icon="divider"
                label="Divider"
                disabled={disabled}
                dragType="divider"
                onClick={() => insert(schema.nodes.divider!.create())}
              />
              <ToolButton
                icon="spacer"
                label="Spacing"
                disabled={disabled}
                dragType="spacer"
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
                {selection.target?.kind === "table_cell"
                  ? `Table cell ${selection.target.columnIndex + 1}`
                  : selection.target?.kind === "table_row"
                    ? "Repeating row"
                    : blockTitle(selection.block)}
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
                    !canMoveBlockAtPath(
                      selectedRegionBlocks,
                      selection.path,
                      -1,
                      selection.region === "body" ? value.media.kind : "label",
                    )
                  }
                  onClick={() => moveSelected(-1)}
                />
                <ToolButton
                  icon="down"
                  label="Move down"
                  disabled={
                    disabled ||
                    !selection.path ||
                    !canMoveBlockAtPath(
                      selectedRegionBlocks,
                      selection.path,
                      1,
                      selection.region === "body" ? value.media.kind : "label",
                    )
                  }
                  onClick={() => moveSelected(1)}
                />
                <ToolButton
                  icon="duplicate"
                  label="Duplicate"
                  disabled={
                    disabled ||
                    !selection.path ||
                    (selection.region === "body" &&
                      isProtectedOrderPageBreakPath(
                        canonicalBody,
                        selection.path,
                        value.media.kind,
                      ))
                  }
                  onClick={duplicateSelected}
                />
                <ToolButton
                  icon="trash"
                  label="Delete"
                  tone="critical"
                  disabled={
                    disabled ||
                    Boolean(
                      selection.path &&
                      selection.region === "body" &&
                      isProtectedOrderPageBreakPath(
                        canonicalBody,
                        selection.path,
                        value.media.kind,
                      ),
                    )
                  }
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
            className={`piqae-page-sheet piqae-rendered-canvas piqae-media-${value.media.kind}${compactAnnotations ? " piqae-compact-annotations" : ""}`}
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
              event.stopPropagation();
              removeSelected();
            }}
          >
            <SafeAreaGuide value={value} stock={stock} />
            <div
              className="piqae-page-content"
              style={canvasContentStyle(value)}
            >
              {value.media.kind === "paged"
                ? renderRepeatingRegion("header")
                : null}
              <main
                className={`piqae-document-body${activeRegion !== "body" ? " is-dimmed" : ""}`}
                data-document-region="body"
              >
                <DocumentCanvas
                  blocks={canonicalBody}
                  resourcePreviewUrls={resourcePreviewUrls}
                  insertionSlots={false}
                  selectedPath={
                    selection?.region === "body" ? selection.path : undefined
                  }
                  editable={!disabled && activeRegion === "body"}
                  preview={activeRegion !== "body"}
                  mediaKind={value.media.kind}
                  authoringFields={allAuthoringFields}
                  onSelect={(block, path, target) =>
                    setSelection({
                      position: -1,
                      block,
                      path,
                      region: "body",
                      target,
                    })
                  }
                  onInsert={(block, path) => insertAtPath(block, path, "body")}
                  onChange={(block, path) => {
                    setSelection({
                      position: -1,
                      block,
                      path,
                      region: "body",
                    });
                    const body = canonicalizeShopifyEditorBody(
                      replaceBlockAtPath(canonicalBody, path, block),
                    );
                    latest.current = { ...latest.current, body };
                    syncBodyEditor(latest.current);
                    publishEditorDocument({ ...latest.current, body });
                  }}
                />
              </main>
              {value.media.kind === "paged"
                ? renderRepeatingRegion("footer")
                : null}
            </div>
            {showsOrderPageBoundary ? (
              <div
                className="piqae-canvas-page-boundary"
                role="note"
                aria-label="Required page break between orders"
              >
                <span>Page break between orders</span>
              </div>
            ) : null}
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
  branch: "root" | "children" | "then" | "else" | "header" | "item" | "empty";
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
  return block.children.at(-1)?.type === "page_break"
    ? "one_order_per_page"
    : "flowing_pages";
}

export type PdfPreviewState =
  | { status: "loading" }
  | { status: "ready"; artifactUrl: string }
  | { status: "empty" }
  | { status: "error"; message: string };

export function isEditorPreviewArtifactUrl(value: string): boolean {
  return /^\/api\/editor-preview-renders\/[A-Za-z0-9_-]{1,128}\/artifact$/.test(
    value,
  );
}

export function PdfPreviewWorkspace({
  state,
  workspaceControls,
}: {
  state: PdfPreviewState;
  workspaceControls?: ReactNode;
}) {
  const ready =
    state.status === "ready" && isEditorPreviewArtifactUrl(state.artifactUrl)
      ? state
      : null;
  const previewImageUrl = ready
    ? ready.artifactUrl.replace(/\/artifact$/, "/image")
    : null;
  const previewImage = useRef<HTMLImageElement>(null);
  const [imageStatus, setImageStatus] = useState<
    "idle" | "loading" | "ready" | "failed"
  >(previewImageUrl ? "loading" : "idle");
  useEffect(() => {
    if (!previewImageUrl) {
      setImageStatus("idle");
      return;
    }
    const image = previewImage.current;
    if (image?.getAttribute("src") === previewImageUrl && image.complete) {
      setImageStatus(image.naturalWidth > 0 ? "ready" : "failed");
      return;
    }
    setImageStatus("loading");
  }, [previewImageUrl]);
  return (
    <div className="piqae-pdf-preview" aria-label="Rendered PDF preview">
      {workspaceControls ? (
        <div className="piqae-workspace-toolbar">{workspaceControls}</div>
      ) : null}
      <div className="piqae-preview-stage" aria-live="polite">
        {state.status === "loading" ? (
          <div className="piqae-preview-status" role="status">
            <strong>Creating PDF preview…</strong>
            <span>Rendering the latest order with current Shopify data.</span>
          </div>
        ) : state.status === "empty" ? (
          <div className="piqae-preview-status">
            <strong>No orders to preview</strong>
            <span>Create an order in this store, then return to Preview.</span>
          </div>
        ) : state.status === "error" ? (
          <div className="piqae-preview-status" role="alert">
            <strong>PDF preview unavailable</strong>
            <span>{state.message}</span>
          </div>
        ) : ready ? (
          <div className="piqae-preview-document">
            {imageStatus === "loading" ? (
              <div
                className="piqae-preview-shimmer"
                role="status"
                aria-label="Loading rendered PDF preview"
              />
            ) : null}
            <img
              ref={previewImage}
              className={`piqae-preview-image is-${imageStatus}`}
              src={previewImageUrl ?? undefined}
              alt="First page of the rendered order PDF"
              onLoad={() => setImageStatus("ready")}
              onError={() => setImageStatus("failed")}
            />
            {imageStatus === "failed" ? (
              <div className="piqae-preview-image-error" role="alert">
                The first-page image could not be shown. The generated PDF is
                still available.
              </div>
            ) : null}
            <a
              className="piqae-preview-open"
              href={ready.artifactUrl}
              target="_blank"
              rel="noreferrer"
            >
              Open full PDF
            </a>
          </div>
        ) : (
          <div className="piqae-preview-status" role="alert">
            <strong>PDF preview unavailable</strong>
            <span>The preview returned an invalid document URL.</span>
          </div>
        )}
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

export function compactCanvasAnnotations(value: PrintPacket): boolean {
  void value;
  return true;
}

export function canvasStyle(value: PrintPacket): CSSProperties {
  const margins = documentMargins(value);
  const { widthMm, heightMm } = canvasGeometry(value);
  const fixed = heightMm !== null;
  const themeFontSize = value.theme?.font_size_pt ?? 10;
  return {
    "--piqae-media-width": `${widthMm}mm`,
    // Width is responsive inside Shopify Admin; aspect-ratio scales the page
    // height by the same factor. Absolute millimetre heights would leave a
    // narrow canvas with an unscaled A4 height.
    "--piqae-media-height": "auto",
    "--piqae-media-min-height": "0",
    "--piqae-mm": `calc(100cqw / ${widthMm})`,
    "--piqae-pt": "calc(var(--piqae-mm) * 0.352777778)",
    "--piqae-theme-font-size": `calc(${themeFontSize} * var(--piqae-pt))`,
    "--piqae-theme-line-height": value.theme?.line_height ?? 1.25,
    "--piqae-margin-top": physicalMm(margins.top_mm),
    "--piqae-margin-right": physicalMm(margins.right_mm),
    "--piqae-margin-bottom": physicalMm(margins.bottom_mm),
    "--piqae-margin-left": physicalMm(margins.left_mm),
    aspectRatio: fixed ? `${widthMm} / ${heightMm}` : undefined,
    padding: 0,
  } as CSSProperties;
}

export function canvasContentStyle(value: PrintPacket): CSSProperties {
  const margins = documentMargins(value);
  const { widthMm, heightMm } = canvasGeometry(value);
  if (heightMm === null)
    return {
      margin: `${physicalMm(margins.top_mm)} ${physicalMm(margins.right_mm)} ${physicalMm(margins.bottom_mm)} ${physicalMm(margins.left_mm)}`,
    };
  return {
    top: `${(margins.top_mm / heightMm) * 100}%`,
    right: `${(margins.right_mm / widthMm) * 100}%`,
    bottom: `${(margins.bottom_mm / heightMm) * 100}%`,
    left: `${(margins.left_mm / widthMm) * 100}%`,
  };
}

function physicalMm(value: number): string {
  return `calc(${value} * var(--piqae-mm))`;
}

function physicalPt(value: number): string {
  return `calc(${value} * var(--piqae-pt))`;
}

export type DocumentMarginEdge = "top" | "right" | "bottom" | "left";
export type DocumentMargins = {
  top_mm: number;
  right_mm: number;
  bottom_mm: number;
  left_mm: number;
};

const DEFAULT_DOCUMENT_MARGINS: DocumentMargins = {
  top_mm: 10,
  right_mm: 10,
  bottom_mm: 10,
  left_mm: 10,
};

/** Matches the renderer's 10 mm default when older documents omit margins. */
export function documentMargins(value: PrintPacket): DocumentMargins {
  return value.media.margins ?? DEFAULT_DOCUMENT_MARGINS;
}

export function maximumDocumentMargin(
  value: PrintPacket,
  edge: DocumentMarginEdge,
): number {
  const margins = documentMargins(value);
  const { widthMm, heightMm } = canvasGeometry(value);
  const vertical = edge === "top" || edge === "bottom";
  const available = vertical ? heightMm : widthMm;
  if (available === null) return 1_000;
  const opposite = vertical
    ? edge === "top"
      ? margins.bottom_mm
      : margins.top_mm
    : edge === "left"
      ? margins.right_mm
      : margins.left_mm;
  return Math.max(0, Math.round((available - opposite - 1) * 10) / 10);
}

export function withDocumentMargin(
  value: PrintPacket,
  edge: DocumentMarginEdge,
  requestedMm: number,
): PrintPacket {
  if (!Number.isFinite(requestedMm)) return value;
  const key = `${edge}_mm` as keyof DocumentMargins;
  const next = Math.min(
    Math.max(requestedMm, 0),
    maximumDocumentMargin(value, edge),
  );
  return {
    ...value,
    media: {
      ...value.media,
      margins: { ...documentMargins(value), [key]: next },
    },
  };
}

/**
 * Keep the authoring placeholder faithful to the bounded renderer. Barcode
 * dimensions remain explicit in PrintPacket, while the canvas clamps a
 * malformed draft to the renderer's 20 x 8 mm minimum and prevents fixed-label
 * overflow. The document itself is never mutated by this presentation helper.
 */
export function barcodeCanvasStyle(
  block: Extract<Block, { type: "barcode" }>,
  _mediaKind: PrintPacket["media"]["kind"],
): CSSProperties {
  const widthMm = Math.min(
    180,
    Math.max(20, Number.isFinite(block.width_mm) ? block.width_mm : 20),
  );
  const heightMm = Math.min(
    80,
    Math.max(8, Number.isFinite(block.height_mm) ? block.height_mm : 8),
  );
  const paddingMm = Math.min(
    50,
    Math.max(
      0,
      Number.isFinite(block.padding_mm) ? (block.padding_mm ?? 0) : 0,
    ),
  );
  const gapMm = Math.min(
    20,
    Math.max(0, Number.isFinite(block.gap_mm) ? (block.gap_mm ?? 1.4) : 1.4),
  );
  const align = block.align ?? "left";
  return {
    width: physicalMm(widthMm + paddingMm * 2),
    maxWidth: "100%",
    marginInline:
      align === "center" ? "auto" : align === "right" ? "auto 0" : "0 auto",
    "--piqae-barcode-width": physicalMm(widthMm),
    "--piqae-barcode-height": physicalMm(heightMm),
    "--piqae-code-padding": physicalMm(paddingMm),
    "--piqae-code-gap": physicalMm(gapMm),
  } as CSSProperties;
}

/** QR placeholders use the same physical square as the PDF renderer. */
export function qrCanvasStyle(
  block: Extract<Block, { type: "qr" }>,
): CSSProperties {
  const sizeMm = Math.min(
    2000,
    Math.max(8, Number.isFinite(block.size_mm) ? block.size_mm : 8),
  );
  return {
    width: physicalMm(sizeMm),
    height: physicalMm(sizeMm),
    "--piqae-qr-size": physicalMm(sizeMm),
  } as CSSProperties;
}

function textCanvasStyle(
  block: Extract<Block, { type: "paragraph" | "heading" }>,
): CSSProperties {
  const style = block.style;
  const defaultHeadingSize =
    block.type === "heading"
      ? (block.level ?? 1) === 1
        ? 22
        : (block.level ?? 1) === 2
          ? 18
          : (block.level ?? 1) === 3
            ? 15
            : 12
      : null;
  const fontSize = style?.font_size_pt ?? defaultHeadingSize;
  return {
    textAlign: style?.align ?? "left",
    fontSize: fontSize === null ? undefined : physicalPt(fontSize),
    fontWeight: block.type === "heading" || style?.bold ? 700 : undefined,
    fontStyle: style?.italic ? "italic" : undefined,
    textDecoration: style?.underline ? "underline" : undefined,
    color: style?.color ? printPacketColor(style.color) : undefined,
  };
}

function printPacketColor(color: {
  red: number;
  green: number;
  blue: number;
}): string {
  return `rgb(${color.red} ${color.green} ${color.blue})`;
}

function boxCanvasStyle(block: Extract<Block, { type: "box" }>): CSSProperties {
  return {
    padding: physicalMm(block.style?.padding_mm ?? 0),
    background: block.style?.background
      ? printPacketColor(block.style.background)
      : undefined,
    borderColor: block.style?.border_color
      ? printPacketColor(block.style.border_color)
      : undefined,
    borderStyle: (block.style?.border_width_pt ?? 0) > 0 ? "solid" : undefined,
    borderWidth: physicalPt(block.style?.border_width_pt ?? 0),
  };
}

function tableCanvasStyle(
  block: Extract<Block, { type: "table" }>,
): CSSProperties {
  return {
    "--piqae-table-cell-padding": physicalMm(block.style?.cell_padding_mm ?? 1),
    "--piqae-table-border-width": physicalPt(
      block.style?.border_width_pt ?? 0.25,
    ),
    "--piqae-table-border-color": block.style?.border_color
      ? printPacketColor(block.style.border_color)
      : "#202223",
    "--piqae-table-header-background": block.style?.header_background
      ? printPacketColor(block.style.header_background)
      : "transparent",
    "--piqae-table-header-color": block.style?.header_text_color
      ? printPacketColor(block.style.header_text_color)
      : "currentColor",
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
      inset: `${physicalMm(safe.top)} ${physicalMm(safe.right)} ${physicalMm(safe.bottom)} ${physicalMm(safe.left)}`,
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
  resourcePreviewUrls,
  path = [],
  branch = "root",
  editable = true,
  mediaKind,
  preview = false,
  protectTerminalPageBreak = false,
  selectedPath,
  authoringFields = AUTHORING_FIELDS,
  scope = "order",
  insertionSlots = true,
  onSelect,
  onInsert,
  onChange,
}: {
  blocks: Block[];
  resourcePreviewUrls: Readonly<Record<string, string>>;
  path?: BlockPath;
  branch?: BlockPathPart["branch"];
  editable?: boolean;
  mediaKind: PrintPacket["media"]["kind"];
  preview?: boolean;
  protectTerminalPageBreak?: boolean;
  selectedPath?: BlockPath;
  authoringFields?: readonly ShopifyDocumentField[];
  scope?: ShopifyAuthoringScope;
  insertionSlots?: boolean;
  onSelect(block: Block, path: BlockPath, target?: CanvasSelectionTarget): void;
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
        const protectedFraming =
          protectTerminalPageBreak &&
          index === blocks.length - 1 &&
          block.type === "page_break";
        const key = `${path.map((part) => `${part.branch}-${part.index}`).join("/")}-${branch}-${index}`;
        return (
          <Fragment key={key}>
            {editable && insertionSlots && onInsert ? (
              <CanvasInsertionSlot path={blockPath} onInsert={onInsert} />
            ) : null}
            <CanvasBlock
              block={block}
              resourcePreviewUrls={resourcePreviewUrls}
              path={blockPath}
              editable={editable}
              mediaKind={mediaKind}
              preview={preview}
              protectedFraming={protectedFraming}
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
      {editable &&
      insertionSlots &&
      onInsert &&
      !(protectTerminalPageBreak && blocks.at(-1)?.type === "page_break") ? (
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
  resourcePreviewUrls,
  path,
  editable,
  mediaKind,
  preview,
  protectedFraming,
  selectedPath,
  selected,
  authoringFields,
  scope,
  onSelect,
  onInsert,
  onChange,
}: {
  block: Block;
  resourcePreviewUrls: Readonly<Record<string, string>>;
  path: BlockPath;
  editable: boolean;
  mediaKind: PrintPacket["media"]["kind"];
  preview: boolean;
  protectedFraming: boolean;
  selectedPath?: BlockPath;
  selected: boolean;
  authoringFields: readonly ShopifyDocumentField[];
  scope: ShopifyAuthoringScope;
  onSelect(block: Block, path: BlockPath, target?: CanvasSelectionTarget): void;
  onInsert?(block: Block, path: BlockPath): void;
  onChange(block: Block, path: BlockPath): void;
}) {
  const [editingText, setEditingText] = useState(false);
  const textBlockElement = useRef<HTMLElement | null>(null);
  const resizeDrag = useRef<TableColumnResizeDrag | null>(null);
  const selectBeforeEdit =
    editable &&
    !preview &&
    (block.type === "paragraph" || block.type === "heading");
  useEffect(() => {
    if (!selected) setEditingText(false);
  }, [selected]);
  const batchPresentation = orderBatchPresentation(block, path, mediaKind);
  const selectableClass = preview ? "" : " piqae-canvas-selectable";
  const select = (event: React.MouseEvent) => {
    if (preview || batchPresentation || protectedFraming) return;
    event.stopPropagation();
    onSelect(
      block,
      path,
      block.type === "grid" && path.at(-1)?.branch === "item"
        ? { kind: "table_row" }
        : undefined,
    );
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
        style={textCanvasStyle(block)}
        tabIndex={selectBeforeEdit ? 0 : undefined}
        aria-label={
          selectBeforeEdit
            ? `${block.type === "heading" ? "Heading" : "Text"} block: ${inlineLabel(block.content)}. Press Enter or F2 to edit.`
            : undefined
        }
        onClick={select}
        onFocus={(event) => {
          if (selectBeforeEdit && event.target === event.currentTarget)
            onSelect(block, path);
        }}
        onDoubleClick={(event) => {
          if (!selectBeforeEdit) return;
          event.stopPropagation();
          onSelect(block, path);
          setEditingText(true);
        }}
        onKeyDown={(event) => {
          if (
            selectBeforeEdit &&
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
          disabled={!editable || (selectBeforeEdit && !editingText)}
          multiline
          autoFocus={selectBeforeEdit && editingText}
          onEscape={() => {
            setEditingText(false);
            requestAnimationFrame(() => textBlockElement.current?.focus());
          }}
          onBlur={() => {
            if (selectBeforeEdit) setEditingText(false);
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
        style={{ borderTopWidth: physicalPt(block.width_pt ?? 0.5) }}
        onClick={preview ? undefined : select}
      />
    );
  if (block.type === "spacer")
    return (
      <div
        className={`piqae-canvas-spacer${selectableClass}${selected ? " piqae-canvas-selected" : ""}`}
        style={{ height: physicalMm(block.height_mm) }}
        onClick={select}
      >
        <span>{block.height_mm} mm space</span>
      </div>
    );
  if (block.type === "page_break")
    return preview ? null : (
      <div
        className={
          protectedFraming
            ? "piqae-canvas-page-break piqae-canvas-protected-framing"
            : selected
              ? "piqae-canvas-page-break piqae-canvas-selected"
              : "piqae-canvas-page-break"
        }
        role={protectedFraming ? "note" : undefined}
        aria-label={
          protectedFraming ? "Required page break between orders" : undefined
        }
        onClick={protectedFraming ? undefined : select}
      >
        {protectedFraming ? "Page break between orders" : "Page break"}
      </div>
    );
  if (block.type === "image" || block.type === "image_value")
    return (
      <CanvasImage
        block={block}
        previewUrl={
          block.type === "image"
            ? resourcePreviewUrls[block.resource]
            : undefined
        }
        className={`${selectableClass}${selected ? " piqae-canvas-selected" : ""}`}
        onClick={select}
      />
    );
  if (block.type === "qr")
    return (
      <div
        className={`piqae-canvas-code piqae-canvas-qr${selectableClass}${selected ? " piqae-canvas-selected" : ""}`}
        style={qrCanvasStyle(block)}
        onClick={select}
      >
        <svg aria-hidden="true" viewBox="0 0 29 29" shapeRendering="crispEdges">
          <rect width="29" height="29" fill="white" />
          <path
            fill="currentColor"
            d="M4 4h7v7H4V4m2 2v3h3V6H6m12-2h7v7h-7V4m2 2v3h3V6h-3M4 18h7v7H4v-7m2 2v3h3v-3H6m7-16h2v2h-2V4m0 4h3v3h-3V8m2 5h2v2h-2v-2m3 0h2v2h-2v-2m4 0h3v3h-3v-3m-9 4h3v2h-3v-2m5 0h2v4h-2v-4m4 1h3v2h-3v-2m-9 3h2v3h-2v-3m4 2h3v2h-3v-2m5 0h2v5h-2v-5m-8 4h3v2h-3v-2m5 0h2v2h-2v-2"
          />
        </svg>
        <small className="piqae-canvas-code-annotation">
          {expressionLabel(block.value)}
        </small>
      </div>
    );
  if (block.type === "barcode")
    return (
      <div
        className={`piqae-canvas-code piqae-canvas-barcode${selectableClass}${selected ? " piqae-canvas-selected" : ""}`}
        style={barcodeCanvasStyle(block, mediaKind)}
        onClick={select}
      >
        <svg
          aria-hidden="true"
          viewBox="0 0 95 32"
          preserveAspectRatio="none"
          shapeRendering="crispEdges"
        >
          <path
            fill="currentColor"
            d="M0 0h2v32H0zm3 0h1v32H3zm3 0h3v32H6zm5 0h1v32h-1zm3 0h2v32h-2zm4 0h4v32h-4zm6 0h1v32h-1zm3 0h3v32h-3zm5 0h2v32h-2zm5 0h1v32h-1zm3 0h4v32h-4zm6 0h2v32h-2zm4 0h1v32h-1zm3 0h3v32h-3zm5 0h1v32h-1zm3 0h4v32h-4zm6 0h2v32h-2zm5 0h3v32h-3zm5 0h1v32h-1zm3 0h3v32h-3zm5 0h2v32h-2zm4 0h4v32h-4zm6 0h1v32h-1z"
          />
        </svg>
        {block.human_readable ? (
          <small className="piqae-canvas-code-value">
            {expressionLabel(block.value)}
          </small>
        ) : (
          <small className="piqae-canvas-code-annotation">
            {expressionLabel(block.value)}
          </small>
        )}
      </div>
    );
  if (block.type === "table") {
    const staticRows = staticTableRows(block);
    return (
      <div
        className={`piqae-canvas-table${selectableClass}${selected ? " piqae-canvas-selected" : ""}`}
        style={tableCanvasStyle(block)}
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
                onBlur={(event) => {
                  if (!editable) return;
                  const nextText = event.currentTarget.textContent ?? "";
                  // Merely tabbing away must not flatten a structured header
                  // (for example a value expression) into rendered text.
                  if (nextText === inlineLabel(column.header)) return;
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
                                  value: nextText,
                                },
                              ],
                            }
                          : item,
                      ),
                    },
                    path,
                  );
                }}
              >
                {inlineLabel(column.header)}
              </strong>
              {editable ? (
                <>
                  <span className="piqae-canvas-column-actions">
                    <button
                      type="button"
                      title={`Move ${inlineLabel(column.header)} left`}
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
                      title={`Move ${inlineLabel(column.header)} right`}
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
                      title={`Fit ${inlineLabel(column.header)} column to content`}
                      aria-label={`Fit ${inlineLabel(column.header)} column to content`}
                      onClick={(event) => {
                        event.stopPropagation();
                        onChange(fitTableColumnToContent(block, index), path);
                      }}
                    >
                      ↔
                    </button>
                    <button
                      type="button"
                      title={`Remove ${inlineLabel(column.header)} column`}
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
                        const table = event.currentTarget.closest<HTMLElement>(
                          ".piqae-canvas-table",
                        );
                        const rect = table?.getBoundingClientRect();
                        const boundaryRatio =
                          tableColumnBoundaryPercent(block.columns, index + 1) /
                          100;
                        const tableLeft = rect?.left ?? event.clientX;
                        const tableWidth = Math.max(1, rect?.width ?? 1);
                        resizeDrag.current = {
                          index,
                          grabOffsetPx:
                            event.clientX -
                            (tableLeft + tableWidth * boundaryRatio),
                          delta: 0,
                          columns: block.columns,
                        };
                        event.currentTarget.style.transform = "";
                        event.currentTarget.setPointerCapture(event.pointerId);
                      }}
                      onPointerMove={(event) => {
                        const drag = resizeDrag.current;
                        if (!drag || drag.index !== index) return;
                        const table = event.currentTarget.closest<HTMLElement>(
                          ".piqae-canvas-table",
                        );
                        const preview = tableColumnResizePreview(
                          drag,
                          event.clientX,
                          table?.getBoundingClientRect(),
                        );
                        drag.delta = preview.delta;
                        event.currentTarget.style.transform = `translateX(${preview.offsetPx}px)`;
                      }}
                      onPointerUp={(event) => {
                        const drag = resizeDrag.current;
                        const table = event.currentTarget.closest<HTMLElement>(
                          ".piqae-canvas-table",
                        );
                        const preview =
                          drag?.index === index
                            ? tableColumnResizePreview(
                                drag,
                                event.clientX,
                                table?.getBoundingClientRect(),
                              )
                            : null;
                        event.currentTarget.style.transform = "";
                        if (drag?.index === index) resizeDrag.current = null;
                        if (
                          event.currentTarget.hasPointerCapture(event.pointerId)
                        )
                          event.currentTarget.releasePointerCapture(
                            event.pointerId,
                          );
                        if (
                          preview &&
                          drag &&
                          Math.abs(preview.delta) > Number.EPSILON
                        )
                          onChange(
                            {
                              ...block,
                              columns: resizeColumns(
                                drag.columns,
                                index,
                                preview.delta,
                              ),
                            },
                            path,
                          );
                      }}
                      onPointerCancel={(event) => {
                        event.currentTarget.style.transform = "";
                        if (resizeDrag.current?.index === index)
                          resizeDrag.current = null;
                      }}
                      onLostPointerCapture={(event) => {
                        event.currentTarget.style.transform = "";
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
            <div
              className="piqae-canvas-column-insertion-layer"
              role="group"
              aria-label="Table column insertion controls"
            >
              {Array.from({ length: block.columns.length + 1 }, (_, index) => {
                const label = tableColumnInsertionLabel(block.columns, index);
                return (
                  <div
                    key={index}
                    className="piqae-canvas-column-insertion-boundary"
                    data-column-insertion-index={index}
                    style={{
                      left: `${tableColumnBoundaryPercent(block.columns, index)}%`,
                    }}
                  >
                    <div
                      className="piqae-canvas-column-insertion-guide"
                      aria-hidden="true"
                    />
                    <button
                      className="piqae-canvas-add-column"
                      type="button"
                      aria-label={label}
                      title={label}
                      onClick={(event) => {
                        event.stopPropagation();
                        onChange(insertTableColumnAt(block, index), path);
                      }}
                    >
                      <Icon name="plus" />
                    </button>
                  </div>
                );
              })}
            </div>
          ) : null}
        </div>
        {staticRows ? (
          <div
            className="piqae-canvas-table-static-body"
            aria-label="Static table rows"
          >
            {editable ? (
              <TableRowInsertionBoundary
                index={0}
                location="before first row"
                onInsert={() =>
                  onChange(insertStaticTableRowAt(block, 0), path)
                }
              />
            ) : null}
            {staticRows.length ? (
              staticRows.map((row, rowIndex) => (
                <div
                  className="piqae-canvas-table-row piqae-canvas-table-static-row"
                  data-table-row-index={rowIndex}
                  key={rowIndex}
                  onClick={(event) => {
                    event.stopPropagation();
                    if (!preview) onSelect(block, path, { kind: "table_row" });
                  }}
                >
                  {block.columns.map((column, columnIndex) => {
                    const editablePath = editableStaticCellPath(column.cell);
                    return (
                      <span
                        key={columnIndex}
                        className="piqae-canvas-table-static-cell"
                        style={{
                          flex: column.width ?? 1,
                          textAlign: column.align,
                        }}
                        role="textbox"
                        aria-label={`${inlineLabel(column.header)} row ${rowIndex + 1}`}
                        aria-readonly={!editable || !editablePath}
                        contentEditable={Boolean(editable && editablePath)}
                        suppressContentEditableWarning
                        onClick={(event) => {
                          event.stopPropagation();
                          if (!preview)
                            onSelect(block, path, {
                              kind: "table_cell",
                              columnIndex,
                            });
                        }}
                        onBlur={(event) => {
                          if (!editable || !editablePath) return;
                          const next = updateStaticTableCell(
                            block,
                            rowIndex,
                            editablePath,
                            event.currentTarget.textContent ?? "",
                          );
                          if (next !== block) onChange(next, path);
                        }}
                      >
                        {staticTableCellLabel(column.cell, row)}
                      </span>
                    );
                  })}
                  {editable ? (
                    <TableRowInsertionBoundary
                      index={rowIndex + 1}
                      location={
                        rowIndex === staticRows.length - 1
                          ? "after last row"
                          : `between rows ${rowIndex + 1} and ${rowIndex + 2}`
                      }
                      onInsert={() =>
                        onChange(
                          insertStaticTableRowAt(block, rowIndex + 1),
                          path,
                        )
                      }
                    />
                  ) : null}
                </div>
              ))
            ) : (
              <div className="piqae-canvas-table-static-empty">No rows</div>
            )}
          </div>
        ) : (
          <div
            className="piqae-canvas-table-row piqae-canvas-table-binding-row"
            aria-label={`Repeating table row from ${expressionLabel(block.items)}`}
            onClick={(event) => {
              event.stopPropagation();
              if (!preview) onSelect(block, path, { kind: "table_row" });
            }}
          >
            {block.columns.map((column, index) => (
              <div
                key={index}
                style={{ flex: column.width ?? 1, textAlign: column.align }}
                onClick={(event) => {
                  event.stopPropagation();
                  if (!preview)
                    onSelect(block, path, {
                      kind: "table_cell",
                      columnIndex: index,
                    });
                }}
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
        )}
        {!preview ? (
          <div
            className="piqae-canvas-collection-branch piqae-canvas-table-empty"
            data-collection-branch="empty"
          >
            <span
              className="piqae-canvas-badge"
              data-label="Empty state"
              data-symbol="∅"
              title="Empty state"
              role="note"
              tabIndex={0}
            >
              Empty state
            </span>
            {block.empty?.length ? (
              <DocumentCanvas
                blocks={block.empty}
                resourcePreviewUrls={resourcePreviewUrls}
                path={path}
                branch="empty"
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
                label="Add empty state"
                editable={editable}
                onAdd={(child) => onChange({ ...block, empty: [child] }, path)}
              />
            )}
          </div>
        ) : null}
      </div>
    );
  }
  if (block.type === "data_list")
    return (
      <section
        className={`piqae-canvas-data-list${preview ? "" : " piqae-canvas-data-list-editor"}${selectableClass}${selected ? " piqae-canvas-selected" : ""}`}
        onClick={select}
      >
        {preview ? null : (
          <span
            className="piqae-canvas-badge"
            data-label={`Data list · ${expressionLabel(block.items)}`}
            data-symbol="↻"
            title={`Data list · ${expressionLabel(block.items)}`}
            role="note"
            tabIndex={0}
          >
            Data list · {expressionLabel(block.items)}
          </span>
        )}
        <CollectionCanvasBranch
          resourcePreviewUrls={resourcePreviewUrls}
          label="List header"
          emptyLabel="Add list header"
          blocks={block.header ?? []}
          path={path}
          branch="header"
          editable={editable}
          mediaKind={mediaKind}
          preview={preview}
          selectedPath={selectedPath}
          authoringFields={authoringFields}
          scope={scope}
          onSelect={onSelect}
          onInsert={onInsert}
          onChange={onChange}
          onAdd={(child) => onChange({ ...block, header: [child] }, path)}
        />
        <CollectionCanvasBranch
          resourcePreviewUrls={resourcePreviewUrls}
          label="Representative item"
          emptyLabel="Add item content"
          blocks={block.item}
          path={path}
          branch="item"
          editable={editable}
          mediaKind={mediaKind}
          preview={preview}
          selectedPath={selectedPath}
          authoringFields={authoringFields}
          scope="item"
          onSelect={onSelect}
          onInsert={onInsert}
          onChange={onChange}
          onAdd={(child) => onChange({ ...block, item: [child] }, path)}
        />
        {preview ? null : (
          <CollectionCanvasBranch
            resourcePreviewUrls={resourcePreviewUrls}
            label="Empty state"
            emptyLabel="Add empty state"
            blocks={block.empty ?? []}
            path={path}
            branch="empty"
            editable={editable}
            mediaKind={mediaKind}
            preview={preview}
            selectedPath={selectedPath}
            authoringFields={authoringFields}
            scope={scope}
            onSelect={onSelect}
            onInsert={onInsert}
            onChange={onChange}
            onAdd={(child) => onChange({ ...block, empty: [child] }, path)}
          />
        )}
      </section>
    );
  if (block.type === "conditional")
    return (
      <section
        className={`piqae-canvas-conditional${selectableClass}${selected ? " piqae-canvas-selected" : ""}`}
        onClick={select}
      >
        {preview ? null : (
          <span
            className="piqae-canvas-badge"
            data-label={`Shown when ${expressionLabel(block.condition)}`}
            data-symbol="◇"
            title={`Shown when ${expressionLabel(block.condition)}`}
            role="note"
            tabIndex={0}
          >
            Shown when {expressionLabel(block.condition)}
          </span>
        )}
        {block.then.length ? (
          <DocumentCanvas
            blocks={block.then}
            resourcePreviewUrls={resourcePreviewUrls}
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
          <div
            className="piqae-canvas-conditional-else"
            data-conditional-branch="else"
          >
            <span
              className="piqae-canvas-badge"
              data-label="Otherwise"
              data-symbol="↳"
              title="Otherwise"
              role="note"
              tabIndex={0}
            >
              Otherwise
            </span>
            <DocumentCanvas
              blocks={block.else}
              resourcePreviewUrls={resourcePreviewUrls}
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
          </div>
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
        : block.type === "box"
          ? "piqae-canvas-box"
          : "piqae-canvas-stack";
  const style =
    block.type === "grid"
      ? {
          gridTemplateColumns: block.columns
            .map((column) => `${column}fr`)
            .join(" "),
          gap: physicalMm(block.gap_mm ?? 0),
        }
      : block.type === "repeat"
        ? undefined
        : block.type === "box"
          ? boxCanvasStyle(block)
          : {
              gap: physicalMm("gap_mm" in block ? (block.gap_mm ?? 0) : 0),
            };
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
          data-label={batchLabel}
          data-symbol="↻"
          title={batchLabel}
          tabIndex={0}
        >
          {batchLabel}
        </span>
      ) : !preview && block.type === "repeat" ? (
        <span
          className="piqae-canvas-badge"
          data-label={`Repeats for each ${expressionLabel(block.items)}`}
          data-symbol="↻"
          title={`Repeats for each ${expressionLabel(block.items)}`}
          role="note"
          tabIndex={0}
        >
          Repeats for each {expressionLabel(block.items)}
        </span>
      ) : null}
      {editable && block.type === "grid" ? (
        <GridResizeHandles block={block} path={path} onChange={onChange} />
      ) : null}
      {children.length ? (
        <DocumentCanvas
          blocks={children}
          resourcePreviewUrls={resourcePreviewUrls}
          path={path}
          branch="children"
          editable={editable}
          mediaKind={mediaKind}
          preview={preview}
          selectedPath={selectedPath}
          authoringFields={authoringFields}
          scope={childScope}
          insertionSlots={block.type !== "grid" && block.type !== "row"}
          protectTerminalPageBreak={batchPresentation === "one_order_per_page"}
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

function CanvasImage({
  block,
  previewUrl,
  className,
  onClick,
}: {
  block: Extract<Block, { type: "image" | "image_value" }>;
  previewUrl?: string;
  className: string;
  onClick(event: React.MouseEvent): void;
}) {
  const [failedUrl, setFailedUrl] = useState<string | null>(null);
  const showImage = Boolean(previewUrl && failedUrl !== previewUrl);
  const label =
    block.type === "image" ? block.resource : expressionLabel(block.resource);
  return (
    <div
      className={`piqae-canvas-image${block.type === "image_value" ? " piqae-canvas-image-dynamic" : ""}${showImage ? " piqae-canvas-image-resolved" : ""}${className}`}
      data-image-fit={block.fit ?? "contain"}
      style={{
        width: physicalMm(block.width_mm),
        height: physicalMm(block.height_mm),
      }}
      onClick={onClick}
    >
      {showImage ? (
        <img
          src={previewUrl}
          alt=""
          referrerPolicy="no-referrer"
          onError={() => setFailedUrl(previewUrl ?? null)}
        />
      ) : (
        <>
          <span aria-hidden="true">▧</span>
          <small>{label}</small>
        </>
      )}
    </div>
  );
}

function TableRowInsertionBoundary({
  index,
  location,
  onInsert,
}: {
  index: number;
  location: string;
  onInsert(): void;
}) {
  const label = `Add table row ${location}`;
  return (
    <div
      className="piqae-canvas-row-insertion-boundary"
      data-row-insertion-index={index}
    >
      <div className="piqae-canvas-row-insertion-guide" aria-hidden="true" />
      <button
        className="piqae-canvas-add-row"
        type="button"
        aria-label={label}
        title={label}
        onClick={(event) => {
          event.stopPropagation();
          onInsert();
        }}
      >
        <Icon name="plus" />
      </button>
    </div>
  );
}

function CollectionCanvasBranch({
  resourcePreviewUrls,
  label,
  emptyLabel,
  blocks,
  path,
  branch,
  editable,
  mediaKind,
  preview,
  selectedPath,
  authoringFields,
  scope,
  onSelect,
  onInsert,
  onChange,
  onAdd,
}: {
  resourcePreviewUrls: Readonly<Record<string, string>>;
  label: string;
  emptyLabel: string;
  blocks: Block[];
  path: BlockPath;
  branch: "header" | "item" | "empty";
  editable: boolean;
  mediaKind: PrintPacket["media"]["kind"];
  preview: boolean;
  selectedPath?: BlockPath;
  authoringFields: readonly ShopifyDocumentField[];
  scope: ShopifyAuthoringScope;
  onSelect(block: Block, path: BlockPath, target?: CanvasSelectionTarget): void;
  onInsert?(block: Block, path: BlockPath): void;
  onChange(block: Block, path: BlockPath): void;
  onAdd(block: Block): void;
}) {
  if (preview && !blocks.length) return null;
  return (
    <div
      className="piqae-canvas-collection-branch"
      data-collection-branch={branch}
    >
      {preview ? null : (
        <span
          className="piqae-canvas-badge"
          data-label={label}
          data-symbol={
            branch === "empty" ? "∅" : branch === "header" ? "H" : "I"
          }
          title={label}
          role="note"
          tabIndex={0}
        >
          {label}
        </span>
      )}
      {blocks.length ? (
        <DocumentCanvas
          blocks={blocks}
          resourcePreviewUrls={resourcePreviewUrls}
          path={path}
          branch={branch}
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
          label={emptyLabel}
          editable={editable}
          preview={preview}
          onAdd={onAdd}
        />
      )}
    </div>
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
  const slot = useRef<HTMLSpanElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (!open) return;
    const closeOnPointerAway = (event: PointerEvent) => {
      const target = event.target;
      if (target instanceof Node && !slot.current?.contains(target))
        setOpen(false);
    };
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      event.stopPropagation();
      setOpen(false);
      requestAnimationFrame(() => trigger.current?.focus());
    };
    document.addEventListener("pointerdown", closeOnPointerAway, true);
    document.addEventListener("keydown", closeOnEscape, true);
    return () => {
      document.removeEventListener("pointerdown", closeOnPointerAway, true);
      document.removeEventListener("keydown", closeOnEscape, true);
    };
  }, [open]);
  const insert = (block: Block) => {
    onInsert(block, path);
    setOpen(false);
    setDragOver(false);
  };
  return (
    <span
      ref={slot}
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
        ref={trigger}
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
              event.stopPropagation();
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
  undo: "M6 4.2 2.8 7.4 6 10.6M3 7.4h5.3a4.2 4.2 0 0 1 4.2 4.2",
  redo: "M10 4.2 13.2 7.4 10 10.6M13 7.4H7.7a4.2 4.2 0 0 0-4.2 4.2",
  duplicate: "M5.6 5.6h7.8v7.8H5.6zM10.6 5.6V2.6H2.8v7.8h2.8",
  trash: "M2.8 4.4h10.4M6.3 4.4V2.9h3.4v1.5M4.4 4.4l.6 8.7h6l.6-8.7",
  settings: "M2.6 5.2h10.8M2.6 10.8h10.8M6 3.6v3.2M10.4 9.2v3.2",
  design: "M2.5 3.5h11v9h-11zM2.5 6.6h11M6.2 6.6v5.9",
  code: "M5.9 4.4 2.6 8l3.3 3.6M10.1 4.4 13.4 8l-3.3 3.6",
  preview:
    "M1.6 8s2.4-4.2 6.4-4.2S14.4 8 14.4 8s-2.4 4.2-6.4 4.2S1.6 8 1.6 8ZM9.8 8a1.8 1.8 0 1 1-3.6 0 1.8 1.8 0 0 1 3.6 0Z",
  annotations: "M3 3.2h10v9.6H3zM5.2 6.1h5.6M5.2 8h3.7M5.2 9.9h4.8",
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
  ariaKeyShortcuts,
  tone,
  disabled,
  pressed,
  dragType,
  onClick,
}: {
  icon: IconName;
  label: string;
  ariaKeyShortcuts?: string;
  tone?: "critical";
  disabled?: boolean;
  pressed?: boolean;
  dragType?: DragInsertType;
  onClick(): void;
}) {
  return (
    <button
      className={`piqae-tool-button${tone === "critical" ? " piqae-tool-critical" : ""}`}
      type="button"
      aria-label={label}
      aria-keyshortcuts={ariaKeyShortcuts}
      aria-pressed={pressed}
      data-tooltip={label}
      disabled={disabled}
      draggable={Boolean(dragType) && !disabled}
      onDragStart={(event) => {
        if (!dragType || disabled) {
          event.preventDefault();
          return;
        }
        event.dataTransfer.effectAllowed = "copy";
        event.dataTransfer.setData(PIQAE_BLOCK_DRAG_TYPE, dragType);
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
  const menu = useRef<HTMLSpanElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const matches = searchDocumentFields(fields, query, 40);
  useEffect(() => {
    if (!open) return;
    const closeOnPointerAway = (event: PointerEvent) => {
      const target = event.target;
      if (target instanceof Node && !menu.current?.contains(target))
        setOpen(false);
    };
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      event.stopPropagation();
      setOpen(false);
      requestAnimationFrame(() => trigger.current?.focus());
    };
    document.addEventListener("pointerdown", closeOnPointerAway, true);
    document.addEventListener("keydown", closeOnEscape, true);
    return () => {
      document.removeEventListener("pointerdown", closeOnPointerAway, true);
      document.removeEventListener("keydown", closeOnEscape, true);
    };
  }, [open]);
  return (
    <span
      ref={menu}
      className="piqae-tool-menu"
      onBlur={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget as Node | null))
          setOpen(false);
      }}
    >
      <button
        ref={trigger}
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
              if (event.key === "Escape") {
                event.preventDefault();
                event.stopPropagation();
                setOpen(false);
                requestAnimationFrame(() => trigger.current?.focus());
              }
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

function InsertImageButton({
  disabled,
  canPickShopify,
  onInsertDynamic,
  onInsertShopify,
}: {
  disabled?: boolean;
  canPickShopify: boolean;
  onInsertDynamic(): void;
  onInsertShopify(): Promise<void>;
}) {
  const [open, setOpen] = useState(false);
  const menu = useRef<HTMLSpanElement>(null);
  useEffect(() => {
    if (!open) return;
    const close = (event: PointerEvent) => {
      if (event.target instanceof Node && !menu.current?.contains(event.target))
        setOpen(false);
    };
    document.addEventListener("pointerdown", close, true);
    return () => document.removeEventListener("pointerdown", close, true);
  }, [open]);
  return (
    <span ref={menu} className="piqae-tool-menu">
      <button
        className="piqae-tool-button"
        type="button"
        aria-label="Image"
        aria-haspopup="menu"
        aria-expanded={open}
        data-tooltip="Image"
        disabled={disabled}
        onClick={() => setOpen((value) => !value)}
      >
        <Icon name="image" />
      </button>
      {open ? (
        <span className="piqae-popover piqae-image-popover" role="menu">
          <button
            type="button"
            role="menuitem"
            disabled={disabled}
            onClick={() => {
              setOpen(false);
              if (disabled) return;
              onInsertDynamic();
            }}
          >
            <Icon name="data" />
            <span>Dynamic Shopify image</span>
            <small>Product, variant, or shop logo</small>
          </button>
          {canPickShopify ? (
            <button
              type="button"
              role="menuitem"
              disabled={disabled}
              onClick={() => {
                setOpen(false);
                if (disabled) return;
                void onInsertShopify();
              }}
            >
              <Icon name="image" />
              <span>Choose from Shopify Files</span>
              <small>Browse or upload an image</small>
            </button>
          ) : null}
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
  return (
    <fieldset className="piqae-settings-group piqae-field-wide">
      <legend>Document margins</legend>
      <p>Sets the printable inset and updates the visual page immediately.</p>
      <div className="piqae-margin-fields">
        {(["top", "right", "bottom", "left"] as const).map((edge) => (
          <label className="piqae-field" key={edge}>
            <span>{edge[0]!.toUpperCase() + edge.slice(1)} (mm)</span>
            <input
              name={`margin${edge[0]!.toUpperCase()}${edge.slice(1)}Mm`}
              type="number"
              inputMode="decimal"
              min={0}
              max={maximumDocumentMargin(value, edge)}
              step={0.5}
              value={documentMargins(value)[`${edge}_mm`]}
              disabled={disabled}
              onChange={(event) => {
                const next = event.currentTarget.valueAsNumber;
                if (Number.isFinite(next))
                  onChange(withDocumentMargin(value, edge, next));
              }}
            />
          </label>
        ))}
      </div>
    </fieldset>
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
            event.stopPropagation();
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
      <datalist id={`${listId}-images`}>
        {authoringFields
          .filter((field) => field.image)
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
  if (block.type === "data_list")
    return (
      <>
        <BarPath
          label="Items"
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
          checked={block.repeat_header ?? true}
          disabled={disabled}
          onChange={(repeat_header) => onChange({ ...block, repeat_header })}
        />
        <BarNumber
          label="Row gap"
          unit="mm"
          value={block.gap_mm ?? 0}
          min={0}
          max={40}
          disabled={disabled}
          onChange={(gap_mm) => onChange({ ...block, gap_mm })}
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
  if (block.type === "image_value")
    return (
      <>
        <BarPath
          label="Dynamic image"
          list={`${listId}-images`}
          value={authoringExpressionLabel(block.resource, scope)}
          disabled={disabled}
          onChange={(resource) =>
            onChange({
              ...block,
              resource: authoringPathExpression(resource, scope),
            })
          }
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
        <BarSegmented
          label="Alignment"
          value={block.align ?? "left"}
          disabled={disabled}
          options={[
            ["left", "Left"],
            ["center", "Centre"],
            ["right", "Right"],
          ]}
          onChange={(align) =>
            onChange({
              ...block,
              align: align as "left" | "center" | "right",
            })
          }
        />
        <BarNumber
          label="Padding"
          unit="mm"
          value={block.padding_mm ?? 0}
          min={0}
          max={50}
          disabled={disabled}
          onChange={(padding_mm) => onChange({ ...block, padding_mm })}
        />
        <BarNumber
          label="Value gap"
          unit="mm"
          value={block.gap_mm ?? 1.4}
          min={0}
          max={20}
          disabled={disabled}
          onChange={(gap_mm) => onChange({ ...block, gap_mm })}
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
  const [draft, setDraft] = useState(value);
  const focused = useRef(false);
  const cancelled = useRef(false);
  useEffect(() => {
    if (!focused.current) setDraft(value);
  }, [value]);
  const commit = () => {
    focused.current = false;
    if (cancelled.current) {
      cancelled.current = false;
      setDraft(value);
      return;
    }
    const next = draft.trim();
    if (next && next !== value) onChange(next);
    else setDraft(value);
  };
  return (
    <BarField label={label}>
      <input
        className="piqae-bar-input piqae-bar-path"
        type="text"
        list={list}
        spellCheck={false}
        value={draft}
        disabled={disabled}
        onFocus={() => {
          focused.current = true;
          cancelled.current = false;
        }}
        onChange={(event) => setDraft(event.currentTarget.value)}
        onBlur={commit}
        onKeyDown={(event) => {
          if (event.key === "Enter") {
            event.preventDefault();
            event.currentTarget.blur();
          } else if (event.key === "Escape") {
            cancelled.current = true;
            setDraft(value);
            event.currentTarget.blur();
          }
        }}
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

function editorInputTarget(target: EventTarget | null) {
  return target instanceof HTMLElement
    ? target.closest<HTMLElement>(
        '[contenteditable="true"], input, textarea, select',
      )
    : null;
}

function nativeHistoryTarget(target: EventTarget | null) {
  const editingControl = editorInputTarget(target);
  return Boolean(
    editingControl && !editingControl.closest(".piqae-prosemirror-source"),
  );
}

function moveItem<T>(items: T[], index: number, direction: -1 | 1) {
  const target = index + direction;
  if (target < 0 || target >= items.length) return items;
  const next = [...items];
  [next[index], next[target]] = [next[target]!, next[index]!];
  return next;
}

type TableBlock = Extract<Block, { type: "table" }>;
type TableColumn = TableBlock["columns"][number];

/**
 * Adds a column at an exact visual boundary. The control layer is separate from
 * the flex columns, so merely revealing it cannot alter the table geometry.
 */
export function insertTableColumnAt(
  block: TableBlock,
  requestedIndex: number,
): TableBlock {
  const index = Math.min(
    Math.max(Math.trunc(requestedIndex), 0),
    block.columns.length,
  );
  const columns = [...block.columns];
  columns.splice(index, 0, defaultColumn());
  return { ...block, columns };
}

/**
 * Text tables are intentionally compact in PrintPacket/v1. When a merchant
 * inserts a block (for example a product image) into one of their cells,
 * promote the table to the richer data-list representation instead of placing
 * the new block after the table or silently discarding its layout.
 */
export function tableToRichDataList(
  block: TableBlock,
  requestedColumn: number,
  inserted: Block,
): Extract<Block, { type: "data_list" }> {
  const columnIndex = Math.min(
    Math.max(Math.trunc(requestedColumn), 0),
    block.columns.length - 1,
  );
  const columns = block.columns.map((column) => column.width ?? 1);
  const paragraphFor = (
    content: Inline[],
    align?: TableColumn["align"],
  ): Block => ({
    type: "paragraph",
    content: structuredClone(content),
    ...(align ? { style: { align } } : {}),
  });
  return {
    type: "data_list",
    items: structuredClone(block.items),
    repeat_header: block.repeat_header ?? false,
    gap_mm: 0,
    header: [
      {
        type: "grid",
        columns,
        gap_mm: 0,
        children: block.columns.map((column) =>
          paragraphFor(column.header, column.align),
        ),
      },
      { type: "divider", width_pt: 0.35 },
    ],
    item: [
      {
        type: "grid",
        columns,
        gap_mm: 1.5,
        children: block.columns.map((column, index) => ({
          type: "stack" as const,
          gap_mm: 1,
          children: [
            ...(index === columnIndex ? [structuredClone(inserted)] : []),
            paragraphFor(column.cell, column.align),
          ],
        })),
      },
      { type: "divider", width_pt: 0.35 },
    ],
    empty: structuredClone(block.empty ?? []),
  };
}

/** Set a useful content-sized ratio without adding a renderer-version field. */
export function fitTableColumnToContent(
  block: TableBlock,
  columnIndex: number,
): TableBlock {
  const target = block.columns[columnIndex];
  if (!target) return block;
  const longestLine = Math.max(
    1,
    ...[inlineLabel(target.header), inlineLabel(target.cell)]
      .flatMap((value) => value.split("\n"))
      .map((value) => value.trim().length),
  );
  const fittedWidth = Math.min(4, Math.max(0.65, longestLine / 6));
  return {
    ...block,
    columns: block.columns.map((column, index) =>
      index === columnIndex ? { ...column, width: fittedWidth } : column,
    ),
  };
}

/** Literal arrays are the PrintPacket/v1 representation of non-repeating rows. */
function staticTableRows(block: TableBlock): unknown[] | null {
  return block.items.type === "literal" && Array.isArray(block.items.value)
    ? block.items.value
    : null;
}

/** Dynamic collection tables stay read-only at the row model level. */
export function insertStaticTableRowAt(
  block: TableBlock,
  requestedIndex: number,
): TableBlock {
  const rows = staticTableRows(block);
  if (!rows) return block;
  const index = Math.min(Math.max(Math.trunc(requestedIndex), 0), rows.length);
  const nextRows = [...rows];
  nextRows.splice(index, 0, {});
  return {
    ...block,
    items: { type: "literal", value: nextRows },
  };
}

function tableColumnInsertionLabel(columns: TableColumn[], index: number) {
  if (index === 0)
    return `Add table column before ${inlineLabel(columns[0]?.header ?? [])}`;
  if (index === columns.length)
    return `Add table column after ${inlineLabel(columns.at(-1)?.header ?? [])}`;
  return `Add table column between ${inlineLabel(columns[index - 1]?.header ?? [])} and ${inlineLabel(columns[index]?.header ?? [])}`;
}

function tableColumnBoundaryPercent(columns: TableColumn[], index: number) {
  const total = columns.reduce(
    (sum, column) => sum + Math.max(column.width ?? 1, 0.01),
    0,
  );
  const before = columns
    .slice(0, index)
    .reduce((sum, column) => sum + Math.max(column.width ?? 1, 0.01), 0);
  return total > 0 ? (before / total) * 100 : 0;
}

function tableColumnResizePreview(
  drag: TableColumnResizeDrag,
  clientX: number,
  rect?: Pick<DOMRect, "left" | "width">,
) {
  if (!rect || !Number.isFinite(rect.width) || rect.width <= 0)
    return { delta: drag.delta, offsetPx: 0 };
  return tableColumnResizeGeometry(
    drag.columns,
    drag.index,
    clientX,
    rect.left,
    rect.width,
    drag.grabOffsetPx,
  );
}

export function tableColumnResizeGeometry(
  columns: TableEditorBlock["columns"],
  index: number,
  clientX: number,
  tableLeft: number,
  tableWidth: number,
  grabOffsetPx = 0,
) {
  const width = Math.max(1, tableWidth);
  const total = columns.reduce((sum, column) => sum + (column.width ?? 1), 0);
  const boundaryRatio = tableColumnBoundaryPercent(columns, index + 1) / 100;
  const boundaryX = tableLeft + width * boundaryRatio;
  const requestedDelta = ((clientX - grabOffsetPx - boundaryX) / width) * total;
  const resized = resizeColumns(columns, index, requestedDelta);
  const originalLeft = columns[index]?.width ?? 1;
  const resizedLeft = resized[index]?.width ?? originalLeft;
  const delta = resizedLeft - originalLeft;
  return {
    delta,
    offsetPx: (delta / total) * width,
  };
}

function editableStaticCellPath(content: Inline[]) {
  const only = content.length === 1 ? content[0] : undefined;
  return only?.type === "value" && only.value.type === "current_path"
    ? only.value.path
    : null;
}

function staticTableCellLabel(content: Inline[], row: unknown) {
  return content
    .map((item) => {
      if (item.type === "text") return item.value;
      if (item.type === "line_break") return "\n";
      if (item.value.type !== "current_path")
        return `{{ ${expressionLabel(item.value)} }}`;
      return printableLiteralValue(literalValueAtPath(row, item.value.path));
    })
    .join("");
}

function literalValueAtPath(value: unknown, path: string[]): unknown {
  let current = value;
  for (const part of path) {
    if (!isLiteralRecord(current)) return undefined;
    current = current[part];
  }
  return current;
}

function printableLiteralValue(value: unknown) {
  if (value === undefined || value === null) return "";
  if (typeof value === "string") return value;
  if (
    typeof value === "number" ||
    typeof value === "boolean" ||
    typeof value === "bigint"
  )
    return String(value);
  return JSON.stringify(value);
}

function updateStaticTableCell(
  block: TableBlock,
  rowIndex: number,
  path: string[],
  value: string,
): TableBlock {
  const rows = staticTableRows(block);
  if (!rows || rows[rowIndex] === undefined || !path.length) return block;
  const previous = literalValueAtPath(rows[rowIndex], path);
  if (value === printableLiteralValue(previous)) return block;
  const nextValue = staticCellValueFromEdit(previous, value);
  if (Object.is(previous, nextValue)) return block;
  const nextRows = rows.map((row, index) =>
    index === rowIndex ? setLiteralValueAtPath(row, path, nextValue) : row,
  );
  return {
    ...block,
    items: { type: "literal", value: nextRows },
  };
}

function setLiteralValueAtPath(
  current: unknown,
  path: string[],
  value: string | number | boolean,
): Record<string, unknown> {
  const [head, ...rest] = path;
  const source = isLiteralRecord(current) ? current : {};
  if (!head) return { ...source };
  return {
    ...source,
    [head]: rest.length
      ? setLiteralValueAtPath(source[head], rest, value)
      : value,
  };
}

function staticCellValueFromEdit(
  previous: unknown,
  source: string,
): string | number | boolean {
  const trimmed = source.trim();
  if (typeof previous === "number" && trimmed) {
    const parsed = Number(trimmed);
    if (Number.isFinite(parsed)) return parsed;
  }
  if (typeof previous === "boolean") {
    if (trimmed.toLowerCase() === "true") return true;
    if (trimmed.toLowerCase() === "false") return false;
  }
  return source;
}

function isLiteralRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
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

function rootOrderRepeat(
  blocks: Block[],
): Extract<Block, { type: "repeat" }> | null {
  const block = blocks.length === 1 ? blocks[0] : undefined;
  return block?.type === "repeat" &&
    block.items.type === "path" &&
    block.items.path.length === 1 &&
    block.items.path[0] === "orders"
    ? block
    : null;
}

/** Inserts author content before the non-editable page separator, when present. */
export function insertBeforeTerminalOrderPageBreak(
  blocks: Block[],
  inserted: Block,
  mediaKind: PrintPacket["media"]["kind"],
): Block[] {
  const repeat = rootOrderRepeat(blocks);
  if (
    mediaKind !== "paged" ||
    !repeat ||
    repeat.children.at(-1)?.type !== "page_break"
  )
    return [...blocks, inserted];
  return [
    {
      ...repeat,
      children: [
        ...repeat.children.slice(0, -1),
        inserted,
        repeat.children.at(-1)!,
      ],
    },
  ];
}

/** True only for the structural separator at the end of the root orders batch. */
export function isProtectedOrderPageBreakPath(
  blocks: Block[],
  path: BlockPath,
  mediaKind: PrintPacket["media"]["kind"],
) {
  if (mediaKind !== "paged" || path.length !== 2) return false;
  const [rootPart, childPart] = path;
  const repeat = rootOrderRepeat(blocks);
  return Boolean(
    repeat &&
    rootPart?.branch === "root" &&
    rootPart.index === 0 &&
    childPart?.branch === "children" &&
    childPart.index === repeat.children.length - 1 &&
    repeat.children.at(-1)?.type === "page_break",
  );
}

/** Prevents content from being moved across the structural order separator. */
export function canMoveBlockAtPath(
  blocks: Block[],
  path: BlockPath,
  direction: -1 | 1,
  mediaKind: PrintPacket["media"]["kind"],
) {
  const part = path.at(-1);
  if (!part || isProtectedOrderPageBreakPath(blocks, path, mediaKind))
    return false;
  const siblings = siblingsAtPath(blocks, path);
  const target = part.index + direction;
  if (target < 0 || target >= siblings.length) return false;
  const repeat = rootOrderRepeat(blocks);
  const isRootOrderChild =
    path.length === 2 &&
    path[0]?.branch === "root" &&
    path[0]?.index === 0 &&
    part.branch === "children";
  return !(
    direction === 1 &&
    mediaKind === "paged" &&
    repeat &&
    isRootOrderChild &&
    repeat.children.at(-1)?.type === "page_break" &&
    target === repeat.children.length - 1
  );
}

type NestedBlockBranch = Exclude<BlockPathPart["branch"], "root">;

function blocksInBranch(
  block: Block,
  branch: BlockPathPart["branch"],
): Block[] | null {
  if (branch === "children" && "children" in block) return block.children;
  if (branch === "then" && block.type === "conditional") return block.then;
  if (branch === "else" && block.type === "conditional")
    return block.else ?? [];
  if (branch === "header" && block.type === "data_list")
    return block.header ?? [];
  if (branch === "item" && block.type === "data_list") return block.item;
  if (branch === "empty" && block.type === "data_list")
    return block.empty ?? [];
  if (branch === "empty" && block.type === "table") return block.empty ?? [];
  return null;
}

function withBlocksInBranch(
  block: Block,
  branch: NestedBlockBranch,
  blocks: Block[],
): Block {
  if (branch === "children" && "children" in block)
    return { ...block, children: blocks };
  if (branch === "then" && block.type === "conditional")
    return { ...block, then: blocks };
  if (branch === "else" && block.type === "conditional")
    return { ...block, else: blocks };
  if (branch === "header" && block.type === "data_list")
    return { ...block, header: blocks };
  if (branch === "item" && block.type === "data_list")
    return { ...block, item: blocks };
  if (branch === "empty" && block.type === "data_list")
    return { ...block, empty: blocks };
  if (branch === "empty" && block.type === "table")
    return { ...block, empty: blocks };
  return block;
}

/** The list a path's final segment indexes into, so move limits are accurate. */
export function siblingsAtPath(blocks: Block[], path: BlockPath): Block[] {
  const [part, ...rest] = path;
  if (!part || !rest.length) return blocks;
  const block = blocks[part.index];
  if (!block) return blocks;
  const next = rest[0]!;
  const nested = blocksInBranch(block, next.branch);
  return nested ? siblingsAtPath(nested, rest) : blocks;
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
    if (nextPart.branch === "root") return block;
    const nested = blocksInBranch(block, nextPart.branch);
    return nested
      ? withBlocksInBranch(
          block,
          nextPart.branch,
          moveBlockAtPath(nested, rest, direction),
        )
      : block;
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
    if (nextPart.branch === "root") return block;
    const nested = blocksInBranch(block, nextPart.branch);
    return nested
      ? withBlocksInBranch(
          block,
          nextPart.branch,
          replaceBlockAtPath(nested, rest, replacement),
        )
      : block;
  });
}
export function blockAtPath(blocks: Block[], path: BlockPath): Block | null {
  const [part, ...rest] = path;
  if (!part) return null;
  const block = blocks[part.index];
  if (!block) return null;
  if (!rest.length) return block;
  const nextPart = rest[0]!;
  if (nextPart.branch === "root") return null;
  const nested = blocksInBranch(block, nextPart.branch);
  return nested ? blockAtPath(nested, rest) : null;
}

/** Keep toolbar insertion inside the selected grid cell. */
export function insertBlockIntoGridCell(
  blocks: Block[],
  path: BlockPath,
  inserted: Block,
): Block[] | null {
  if (path.length < 2) return null;
  const cellPart = path.at(-1)!;
  if (cellPart.branch !== "children") return null;
  const parent = blockAtPath(blocks, path.slice(0, -1));
  if (
    parent?.type !== "grid" ||
    cellPart.index < 0 ||
    cellPart.index >= parent.columns.length
  )
    return null;
  const cell = parent.children[cellPart.index];
  if (!cell) return null;
  const emptyParagraph =
    cell.type === "paragraph" &&
    cell.content.every(
      (inline) => inline.type === "text" && inline.value.trim() === "",
    );
  const replacement: Block = emptyParagraph
    ? structuredClone(inserted)
    : cell.type === "stack"
      ? {
          ...cell,
          children: [...cell.children, structuredClone(inserted)],
        }
      : {
          type: "stack",
          gap_mm: 1,
          children: [cell, structuredClone(inserted)],
        };
  return replaceBlockAtPath(blocks, path, replacement);
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
    if (nextPart.branch === "root") return block;
    const nested = blocksInBranch(block, nextPart.branch);
    return nested
      ? withBlocksInBranch(
          block,
          nextPart.branch,
          insertBlockAfterPath(nested, rest, inserted),
        )
      : block;
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
    if (nextPart.branch === "root") return block;
    const nested = blocksInBranch(block, nextPart.branch);
    return nested
      ? withBlocksInBranch(
          block,
          nextPart.branch,
          insertBlockAtPath(nested, rest, inserted),
        )
      : block;
  });
}
export function removeBlockAtPath(blocks: Block[], path: BlockPath): Block[] {
  const [part, ...rest] = path;
  if (!part) return blocks;
  if (!rest.length) return blocks.filter((_, index) => index !== part.index);
  return blocks.map((block, index) => {
    if (index !== part.index) return block;
    const nextPart = rest[0]!;
    if (nextPart.branch === "root") return block;
    const nested = blocksInBranch(block, nextPart.branch);
    return nested
      ? withBlocksInBranch(
          block,
          nextPart.branch,
          removeBlockAtPath(nested, rest),
        )
      : block;
  });
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
    data_list: "Data list",
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
    image_value: "Dynamic image",
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
    data_list: "repeat",
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
    image_value: "image",
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
    if (block.type === "data_list" && next.branch === "item") scope = "item";
    siblings = blocksInBranch(block, next.branch) ?? [];
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
  const type = dataTransfer.getData(PIQAE_BLOCK_DRAG_TYPE);
  if (!DRAG_INSERT_TYPES.some((candidate) => candidate === type)) return null;
  return dragInsertBlock(type as DragInsertType);
}
function dragInsertBlock(type: DragInsertType): Block {
  switch (type) {
    case "paragraph":
    case "heading":
    case "image":
    case "divider":
    case "spacer":
      return quickInsertBlock(type);
    case "table":
      return defaultTable();
    case "repeat":
      return defaultRepeat();
    case "conditional":
      return defaultConditional();
    case "qr":
      return defaultQrCode();
    case "barcode":
      return defaultBarcode();
    case "grid":
      return defaultGrid();
    case "stack":
    case "row":
      return defaultContainer(type);
  }
}
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
function defaultImage(scope: ShopifyAuthoringScope): Block {
  return {
    type: "image_value",
    resource:
      scope === "item"
        ? currentPathExpression("imageResource")
        : pathExpression("shop.logo"),
    width_mm: scope === "item" ? 16 : 42,
    height_mm: scope === "item" ? 16 : 18,
    fit: "contain",
  };
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
function defaultBarcode(scope: ShopifyAuthoringScope = "order"): Block {
  return {
    type: "barcode",
    value: currentPathExpression(
      scope === "item" ? "labelCode128" : "referenceCode128",
    ),
    symbology: "code128",
    width_mm: 48,
    height_mm: 16,
    human_readable: true,
    align: "center",
    padding_mm: 1.5,
    gap_mm: 1.2,
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
