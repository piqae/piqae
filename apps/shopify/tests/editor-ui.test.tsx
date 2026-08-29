// @vitest-environment jsdom

import { act, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { createRoutesStub } from "react-router";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  insertBlockAfterPath,
  insertBlockAtPath,
  moveBlockAtPath,
  PrintPacketEditor,
  PrintPacketPreview,
  removeBlockAtPath,
  replaceBlockAtPath,
  siblingsAtPath,
  orderBatchPresentation,
} from "../app/components/PrintPacketEditor";
import type { PrintPacket } from "../app/core/template-model";
import { starterTemplates } from "../app/core/starter-templates";
import Templates from "../app/routes/app.templates";
import {
  documentNameError,
  editorTitleBarActions,
  templateFlowNote,
} from "../app/routes/app.templates.$templateId";

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });

const packet: PrintPacket = {
  format: "printpacket/v1",
  media: { kind: "paged", size: "a4" },
  body: [
    {
      type: "paragraph",
      content: [{ type: "text", value: "Hello" }],
    },
    {
      type: "image",
      resource: "shop.logo",
      width_mm: 42,
      height_mm: 18,
      fit: "contain",
    },
  ],
};

const tablePacket: PrintPacket = {
  format: "printpacket/v1",
  media: { kind: "paged", size: "a4" },
  body: [
    {
      type: "table",
      items: { type: "path", path: ["order", "lineItems"] },
      columns: [
        {
          header: [{ type: "text", value: "Item" }],
          cell: [
            {
              type: "value",
              value: { type: "current_path", path: ["title"] },
            },
          ],
          width: 1,
          align: "left",
        },
        {
          header: [{ type: "text", value: "Quantity" }],
          cell: [
            {
              type: "value",
              value: { type: "current_path", path: ["quantity"] },
            },
          ],
          width: 1,
          align: "right",
        },
      ],
    },
  ],
};

const gridPacket: PrintPacket = {
  format: "printpacket/v1",
  media: { kind: "paged", size: "a4" },
  body: [
    {
      type: "grid",
      columns: [2, 1],
      gap_mm: 8,
      children: [
        {
          type: "paragraph",
          content: [{ type: "text", value: "Left" }],
        },
        {
          type: "paragraph",
          content: [{ type: "text", value: "Right" }],
        },
      ],
    },
  ],
};

const collectionPacket: PrintPacket = {
  format: "printpacket/v1",
  media: { kind: "paged", size: "a4" },
  body: [
    {
      type: "data_list",
      items: { type: "path", path: ["order", "lineItems"] },
      header: [
        {
          type: "paragraph",
          content: [{ type: "text", value: "List header" }],
        },
      ],
      item: [
        {
          type: "paragraph",
          content: [
            { type: "text", value: "Item " },
            {
              type: "value",
              value: { type: "current_path", path: ["title"] },
            },
          ],
        },
      ],
      empty: [
        {
          type: "paragraph",
          content: [{ type: "text", value: "No list items" }],
        },
      ],
    },
    {
      ...(tablePacket.body[0] as Extract<
        PrintPacket["body"][number],
        { type: "table" }
      >),
      empty: [
        {
          type: "paragraph",
          content: [{ type: "text", value: "No table items" }],
        },
      ],
    },
  ],
};

let root: Root | null = null;
let host: HTMLDivElement | null = null;

async function render(node: ReactNode) {
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
  await act(async () => root?.render(node));
  return host;
}

function authoredBody(value: PrintPacket) {
  const wrapper = value.body[0];
  return wrapper?.type === "repeat" &&
    wrapper.items.type === "path" &&
    wrapper.items.path.join(".") === "orders"
    ? wrapper.children
    : value.body;
}

afterEach(async () => {
  await act(async () => root?.unmount());
  host?.remove();
  root = null;
  host = null;
});

describe("Shopify document editor layout", () => {
  it("keeps starter order batching explicit without changing open source semantics", () => {
    const invoice = starterTemplates.find(
      ({ id }) => id === "invoice",
    )!.specification;
    const packingSlip = starterTemplates.find(
      ({ id }) => id === "packing-slip",
    )!.specification;
    const receipt = starterTemplates.find(
      ({ id }) => id === "receipt",
    )!.specification;
    const rootPath = [{ branch: "root" as const, index: 0 }];

    expect(
      orderBatchPresentation(invoice.body[0]!, rootPath, invoice.media.kind),
    ).toBe("one_order_per_page");
    expect(
      orderBatchPresentation(
        packingSlip.body[0]!,
        rootPath,
        packingSlip.media.kind,
      ),
    ).toBe("one_order_per_page");
    expect(
      orderBatchPresentation(
        receipt.body.find((block) => block.type === "repeat")!,
        [
          {
            branch: "root",
            index: receipt.body.findIndex((block) => block.type === "repeat"),
          },
        ],
        receipt.media.kind,
      ),
    ).toBe("continuous");

    const flowingInvoice = structuredClone(invoice);
    const repeat = flowingInvoice.body[0];
    if (repeat?.type !== "repeat") throw new Error("invoice repeat missing");
    repeat.children = repeat.children.filter(
      (child) => child.type !== "page_break",
    );
    expect(
      orderBatchPresentation(repeat, rootPath, flowingInvoice.media.kind),
    ).toBe("flowing_pages");
  });

  it("presents the order batch as document structure, not editable content", async () => {
    const invoice = starterTemplates.find(
      ({ id }) => id === "invoice",
    )!.specification;
    const page = await render(
      <PrintPacketEditor value={invoice} onChange={() => undefined} />,
    );
    const batch = page.querySelector<HTMLElement>(".piqae-canvas-order-batch");

    expect(batch).not.toBeNull();
    expect(batch?.classList.contains("piqae-canvas-selectable")).toBe(false);
    expect(
      batch?.querySelector('[aria-label="Order batching behavior"]')
        ?.textContent,
    ).toBe("One page per order");
    expect(page.textContent).not.toContain("Repeats for each orders");

    await act(async () => {
      batch?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expect(page.querySelector(".piqae-selection-rail")).toBeNull();
  });

  it("protects the terminal order page break and keeps insertion before it", async () => {
    const invoice = structuredClone(
      starterTemplates.find(({ id }) => id === "invoice")!.specification,
    );
    const onChange = vi.fn();
    const page = await render(
      <PrintPacketEditor value={invoice} onChange={onChange} />,
    );
    const framing = page.querySelector<HTMLElement>(
      ".piqae-canvas-protected-framing",
    )!;
    const repeat = invoice.body[0];
    if (repeat?.type !== "repeat") throw new Error("invoice repeat missing");
    const finalAuthorSlot = framing.previousElementSibling as HTMLElement;

    expect(framing.getAttribute("role")).toBe("note");
    expect(framing.tabIndex).toBe(-1);
    expect(
      finalAuthorSlot.classList.contains("piqae-canvas-insertion-slot"),
    ).toBe(true);
    expect(finalAuthorSlot.dataset.insertionIndex).toBe(
      String(repeat.children.length - 1),
    );
    expect(
      framing.parentElement?.querySelector(
        `.piqae-canvas-insertion-slot[data-insertion-index="${repeat.children.length}"]`,
      ),
    ).toBeNull();

    await act(async () => framing.click());
    expect(page.querySelector(".piqae-selection-rail")).toBeNull();

    await act(async () => {
      finalAuthorSlot
        .querySelector<HTMLButtonElement>('[aria-label="Add content here"]')
        ?.click();
    });
    await act(async () => {
      Array.from(finalAuthorSlot.querySelectorAll("[role=menuitem]"))
        .find((item) => item.textContent?.includes("Text"))
        ?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    const next = onChange.mock.lastCall?.[0] as PrintPacket;
    const nextRepeat = next.body[0];
    expect(nextRepeat?.type).toBe("repeat");
    if (nextRepeat?.type !== "repeat") return;
    expect(nextRepeat.children.at(-1)?.type).toBe("page_break");
    expect(nextRepeat.children.at(-2)?.type).toBe("paragraph");
  });

  it("keeps toolbar insertion and movement inside the terminal order separator", async () => {
    const invoice = structuredClone(
      starterTemplates.find(({ id }) => id === "invoice")!.specification,
    );
    const onChange = vi.fn();
    const page = await render(
      <PrintPacketEditor value={invoice} onChange={onChange} />,
    );
    const framing = page.querySelector<HTMLElement>(
      ".piqae-canvas-protected-framing",
    )!;
    const finalAuthorSlot = framing.previousElementSibling as HTMLElement;
    const finalAuthorBlock =
      finalAuthorSlot.previousElementSibling as HTMLElement;

    await act(async () => {
      page
        .querySelector<HTMLButtonElement>('button[aria-label="Text"]')
        ?.click();
    });
    const inserted = onChange.mock.lastCall?.[0] as PrintPacket;
    const insertedRepeat = inserted.body[0];
    expect(insertedRepeat?.type).toBe("repeat");
    if (insertedRepeat?.type !== "repeat") return;
    expect(insertedRepeat.children.at(-1)?.type).toBe("page_break");
    expect(insertedRepeat.children.at(-2)?.type).toBe("paragraph");

    await act(async () => finalAuthorBlock.click());
    expect(
      page.querySelector<HTMLButtonElement>('button[aria-label="Move down"]')
        ?.disabled,
    ).toBe(true);
  });

  it("keeps Preview chrome-free and aligned with the editor workspace", async () => {
    const invoice = starterTemplates.find(
      ({ id }) => id === "invoice",
    )!.specification;
    const page = await render(
      <PrintPacketPreview
        value={invoice}
        workspaceControls={
          <div role="group" aria-label="Editor workspace">
            Design Code Preview
          </div>
        }
      />,
    );
    const stage = page.querySelector(".piqae-preview-stage");
    const canvas = stage?.querySelector(".piqae-presentation-canvas");
    const batch = canvas?.querySelector<HTMLElement>(
      ".piqae-canvas-order-batch",
    );

    expect(
      stage?.querySelector(".piqae-workspace-toolbar [role=group]"),
    ).not.toBeNull();
    expect(canvas).not.toBeNull();
    expect(canvas?.querySelector(".piqae-canvas-selectable")).toBeNull();
    expect(canvas?.querySelector(".piqae-canvas-badge")).toBeNull();
    expect(canvas?.querySelector(".piqae-canvas-page-break")).toBeNull();
    expect(canvas?.querySelector(".piqae-canvas-empty")).toBeNull();
    expect(batch?.style.gap).toBe("");
  });

  it("previews collection headers and one representative item without empty-state chrome", async () => {
    const page = await render(<PrintPacketPreview value={collectionPacket} />);
    const canvas = page.querySelector<HTMLElement>(
      ".piqae-presentation-canvas",
    )!;
    const list = canvas.querySelector<HTMLElement>(".piqae-canvas-data-list")!;
    const table = canvas.querySelector<HTMLElement>(".piqae-canvas-table")!;

    expect(list.textContent).toContain("List header");
    expect(list.textContent).toContain("Item");
    expect(list.textContent).not.toContain("No list items");
    expect(table.textContent).not.toContain("No table items");
    expect(canvas.querySelector(".piqae-canvas-badge")).toBeNull();
    expect(canvas.querySelector('[data-collection-branch="empty"]')).toBeNull();
    expect(canvas.querySelector(".piqae-canvas-insertion-slot")).toBeNull();
  });

  it("keeps starter and editable actions in the Shopify title bar contract", () => {
    expect(editorTitleBarActions(true)).toEqual({
      primary: { label: "Save as copy", intent: "draft" },
      secondary: { label: "Publish copy", intent: "publish" },
    });
    expect(editorTitleBarActions(false)).toEqual({
      primary: { label: "Publish", intent: "publish" },
      secondary: { label: "Save draft", intent: "draft" },
    });
    expect(templateFlowNote(null, true)).toBeNull();
    expect(documentNameError("", "publish")).toBe(
      "Enter a document name in Settings before saving.",
    );
    expect(documentNameError("", "delete")).toBeNull();
  });

  it("removes portable import from the templates page", async () => {
    const Stub = createRoutesStub([
      {
        path: "/",
        Component: Templates,
        HydrateFallback: () => null,
        loader: () => ({ templates: [] }),
      },
    ]);
    const page = await render(<Stub />);

    expect(page.textContent).toContain("Create your first template");
    expect(page.textContent).not.toContain("Import template");
    expect(page.querySelector('textarea[name="import"]')).toBeNull();
  });

  it("shows workspace, insert, and changing selection tools in one card", async () => {
    const onChange = vi.fn();
    const page = await render(
      <PrintPacketEditor
        value={packet}
        workspaceControls={
          <div role="group" aria-label="Editor workspace">
            Design Code Preview
          </div>
        }
        onChange={onChange}
      />,
    );
    const toolbar = page.querySelector(".piqae-editor-toolbar");
    expect(toolbar).not.toBeNull();
    expect(toolbar?.querySelector('[role="group"]')?.textContent).toContain(
      "Design Code Preview",
    );
    expect(
      toolbar?.querySelector(
        '[role="toolbar"][aria-label="Insert into document"]',
      ),
    ).not.toBeNull();
    expect(toolbar?.querySelector(".piqae-selection-rail")).toBeNull();
    expect(toolbar?.children).toHaveLength(1);

    await act(async () => {
      page
        .querySelector<HTMLElement>(".piqae-canvas-text")
        ?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expect(
      toolbar?.querySelector(".piqae-selection-title")?.textContent,
    ).toContain("Text");
    expect(toolbar?.querySelector(".piqae-selection-rail")).not.toBeNull();
    expect(toolbar?.children).toHaveLength(2);
    expect(toolbar?.querySelector("select.piqae-bar-input")).not.toBeNull();

    await act(async () => {
      page
        .querySelector<HTMLElement>(".piqae-canvas-image")
        ?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expect(
      toolbar?.querySelector(".piqae-selection-title")?.textContent,
    ).toContain("Image");
    expect(
      toolbar?.querySelectorAll(".piqae-bar-input").length,
    ).toBeGreaterThan(1);
  });

  it("keeps both toolbar rows keyboard reachable at a narrow viewport", async () => {
    Object.defineProperty(window, "innerWidth", {
      configurable: true,
      value: 320,
    });
    const page = await render(
      <PrintPacketEditor value={packet} onChange={() => undefined} />,
    );
    const card = page.querySelector(".piqae-editor-toolbar");
    const buttons = card?.querySelectorAll<HTMLButtonElement>("button") ?? [];

    expect(card?.children).toHaveLength(1);
    expect(Array.from(buttons).every((button) => button.tabIndex >= 0)).toBe(
      true,
    );
    expect(page.querySelector(".piqae-page-sheet")).not.toBeNull();

    await act(async () => {
      page
        .querySelector<HTMLElement>(".piqae-canvas-text")
        ?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expect(card?.children).toHaveLength(2);
    expect(card?.querySelector(".piqae-selection-rail")).not.toBeNull();
  });

  it("selects top-level text once and edits only on double-click or keyboard intent", async () => {
    const page = await render(
      <PrintPacketEditor value={packet} onChange={() => undefined} />,
    );
    const text = page.querySelector<HTMLElement>(".piqae-canvas-text")!;
    const editor = text.querySelector<HTMLElement>("[role='textbox']")!;

    expect(text.tabIndex).toBe(0);
    expect(editor.getAttribute("contenteditable")).toBe("false");
    await act(async () => text.click());
    expect(page.querySelector(".piqae-selection-title")?.textContent).toContain(
      "Text",
    );
    expect(editor.getAttribute("contenteditable")).toBe("false");

    await act(async () => {
      text.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
    });
    expect(editor.getAttribute("contenteditable")).toBe("true");
    expect(document.activeElement).toBe(editor);

    await act(async () => {
      editor.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
      );
    });
    await act(
      () =>
        new Promise<void>((resolve) => requestAnimationFrame(() => resolve())),
    );
    expect(editor.getAttribute("contenteditable")).toBe("false");

    await act(async () => {
      text.focus();
      text.dispatchEvent(
        new KeyboardEvent("keydown", { key: "F2", bubbles: true }),
      );
    });
    expect(editor.getAttribute("contenteditable")).toBe("true");

    await act(async () => {
      editor.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
      );
    });
    await act(
      () =>
        new Promise<void>((resolve) => requestAnimationFrame(() => resolve())),
    );
    await act(async () => {
      text.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Enter", bubbles: true }),
      );
    });
    expect(editor.getAttribute("contenteditable")).toBe("true");
  });

  it("uses the same select-then-edit behavior for top-level headings", async () => {
    const headingPacket: PrintPacket = {
      ...packet,
      body: [
        {
          type: "heading",
          level: 2,
          content: [{ type: "text", value: "Order summary" }],
        },
      ],
    };
    const page = await render(
      <PrintPacketEditor value={headingPacket} onChange={() => undefined} />,
    );
    const heading = page.querySelector<HTMLElement>(".piqae-canvas-text")!;
    const editor = heading.querySelector<HTMLElement>("[role='textbox']")!;

    await act(async () => heading.click());
    expect(page.querySelector(".piqae-selection-title")?.textContent).toContain(
      "Heading",
    );
    expect(editor.getAttribute("contenteditable")).toBe("false");
    await act(async () => {
      heading.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
    });
    expect(editor.getAttribute("contenteditable")).toBe("true");
  });

  it("selects nested text before editing and can delete the nested element", async () => {
    const onChange = vi.fn();
    const page = await render(
      <PrintPacketEditor value={gridPacket} onChange={onChange} />,
    );
    const nestedText = page.querySelector<HTMLElement>(
      ".piqae-canvas-grid .piqae-canvas-text",
    )!;
    const editor = nestedText.querySelector<HTMLElement>("[role=textbox]")!;

    expect(editor.getAttribute("contenteditable")).toBe("false");
    await act(async () => nestedText.click());
    expect(page.querySelector(".piqae-selection-title")?.textContent).toContain(
      "Text",
    );
    expect(editor.getAttribute("contenteditable")).toBe("false");

    await act(async () => {
      nestedText.focus();
      nestedText.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Delete", bubbles: true }),
      );
    });
    const next = onChange.mock.lastCall?.[0] as PrintPacket;
    const grid = authoredBody(next)[0];
    expect(grid.type).toBe("grid");
    if (grid.type !== "grid") return;
    expect(grid.children).toHaveLength(1);
    expect(grid.children[0]?.type).toBe("paragraph");
  });

  it("selects and deletes a nested data-list item without changing its other branches", async () => {
    const onChange = vi.fn();
    const page = await render(
      <PrintPacketEditor value={collectionPacket} onChange={onChange} />,
    );
    const list = page.querySelector<HTMLElement>(".piqae-canvas-data-list")!;
    const itemText = list.querySelector<HTMLElement>(
      '[data-collection-branch="item"] .piqae-canvas-text',
    )!;

    expect(list.textContent).toContain("List header");
    expect(list.textContent).toContain("Representative item");
    expect(list.textContent).toContain("Empty state");
    await act(async () => itemText.click());
    expect(page.querySelector(".piqae-selection-title")?.textContent).toContain(
      "Text",
    );
    await act(async () => {
      itemText.focus();
      itemText.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Delete", bubbles: true }),
      );
    });

    const next = onChange.mock.lastCall?.[0] as PrintPacket;
    const listBlock = authoredBody(next)[0];
    expect(listBlock.type).toBe("data_list");
    if (listBlock.type !== "data_list") return;
    expect(listBlock.items).toEqual({
      type: "current_path",
      path: ["lineItems"],
    });
    expect(listBlock.header?.[0]).toMatchObject({ type: "paragraph" });
    expect(listBlock.item).toEqual([]);
    expect(listBlock.empty?.[0]).toMatchObject({ type: "paragraph" });
  });

  it("inserts into the representative data-list item and preserves scoped expressions", async () => {
    const onChange = vi.fn();
    const page = await render(
      <PrintPacketEditor value={collectionPacket} onChange={onChange} />,
    );
    const itemBranch = page.querySelector<HTMLElement>(
      '.piqae-canvas-data-list [data-collection-branch="item"]',
    )!;
    const finalSlot = Array.from(
      itemBranch.querySelectorAll<HTMLElement>(".piqae-canvas-insertion-slot"),
    ).at(-1)!;

    await act(async () => {
      finalSlot
        .querySelector<HTMLButtonElement>('[aria-label="Add content here"]')
        ?.click();
    });
    await act(async () => {
      Array.from(finalSlot.querySelectorAll("[role=menuitem]"))
        .find((item) => item.textContent?.includes("Heading"))
        ?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });

    const next = onChange.mock.lastCall?.[0] as PrintPacket;
    const listBlock = authoredBody(next)[0];
    expect(listBlock.type).toBe("data_list");
    if (listBlock.type !== "data_list") return;
    expect(listBlock.item.map((block) => block.type)).toEqual([
      "paragraph",
      "heading",
    ]);
    expect(listBlock.item[0]).toMatchObject({
      type: "paragraph",
      content: [
        { type: "text", value: "Item " },
        {
          type: "value",
          value: { type: "current_path", path: ["title"] },
        },
      ],
    });
    expect(listBlock.header?.[0]).toMatchObject({ type: "paragraph" });
    expect(listBlock.empty?.[0]).toMatchObject({ type: "paragraph" });
  });

  it("removes a selected non-editing block from keyboard or contextual delete", async () => {
    const onChange = vi.fn();
    const page = await render(
      <PrintPacketEditor value={packet} onChange={onChange} />,
    );
    const text = page.querySelector<HTMLElement>(".piqae-canvas-text")!;

    await act(async () => text.click());
    await act(async () => {
      text.focus();
      text.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Delete", bubbles: true }),
      );
    });
    const keyboardDocument = onChange.mock.lastCall?.[0] as PrintPacket;
    expect(authoredBody(keyboardDocument).map((block) => block.type)).toEqual([
      "image",
    ]);

    onChange.mockClear();
    await act(async () =>
      root?.render(<PrintPacketEditor value={packet} onChange={onChange} />),
    );
    const rerenderedText =
      page.querySelector<HTMLElement>(".piqae-canvas-text")!;
    await act(async () => rerenderedText.click());
    const deleteAction = page.querySelector<HTMLButtonElement>(
      'button[aria-label="Delete"]',
    )!;
    expect(deleteAction.disabled).toBe(false);
    expect(deleteAction.tabIndex).toBeGreaterThanOrEqual(0);
    await act(async () => deleteAction.click());
    const contextualDocument = onChange.mock.lastCall?.[0] as PrintPacket;
    expect(authoredBody(contextualDocument).map((block) => block.type)).toEqual(
      ["image"],
    );
  });

  it("keeps table cells directly editable and moves add-column into the header", async () => {
    const onChange = vi.fn();
    const page = await render(
      <PrintPacketEditor value={tablePacket} onChange={onChange} />,
    );
    const table = page.querySelector(".piqae-canvas-table")!;
    const header = table.querySelector(".piqae-canvas-table-head")!;
    const addColumn = table.querySelector<HTMLButtonElement>(
      'button[aria-label="Add table column"]',
    )!;

    expect(header.contains(addColumn)).toBe(true);
    expect(addColumn.textContent).not.toContain("Add column");
    expect(
      table
        .querySelector<HTMLElement>("strong[contenteditable]")
        ?.getAttribute("contenteditable"),
    ).toBe("true");
    expect(
      table
        .querySelector<HTMLElement>(
          ".piqae-canvas-table-binding-row [role='textbox']",
        )
        ?.getAttribute("contenteditable"),
    ).toBe("true");

    await act(async () => addColumn.click());
    const nextDocument = onChange.mock.lastCall?.[0] as PrintPacket;
    const nextTable = authoredBody(nextDocument)[0];
    expect(nextTable.type).toBe("table");
    if (nextTable.type === "table") expect(nextTable.columns).toHaveLength(3);
  });

  it("selects and removes table empty-state content without changing the table", async () => {
    const onChange = vi.fn();
    const page = await render(
      <PrintPacketEditor value={collectionPacket} onChange={onChange} />,
    );
    const table = page.querySelector<HTMLElement>(".piqae-canvas-table")!;
    const emptyText = table.querySelector<HTMLElement>(
      '[data-collection-branch="empty"] .piqae-canvas-text',
    )!;

    expect(table.textContent).toContain("Empty state");
    await act(async () => emptyText.click());
    await act(async () => {
      emptyText.focus();
      emptyText.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Backspace", bubbles: true }),
      );
    });

    const next = onChange.mock.lastCall?.[0] as PrintPacket;
    const tableBlock = authoredBody(next)[1];
    expect(tableBlock.type).toBe("table");
    if (tableBlock.type !== "table") return;
    expect(tableBlock.empty).toEqual([]);
    expect(tableBlock.items).toEqual({
      type: "current_path",
      path: ["lineItems"],
    });
    expect(tableBlock.columns).toEqual(
      (
        tablePacket.body[0] as Extract<
          PrintPacket["body"][number],
          { type: "table" }
        >
      ).columns,
    );
  });

  it("resizes table columns from an accessible keyboard separator without changing expressions", async () => {
    const onChange = vi.fn();
    const page = await render(
      <PrintPacketEditor value={tablePacket} onChange={onChange} />,
    );
    const separator = page.querySelector<HTMLElement>(
      '[role="separator"][aria-label="Resize Item column"]',
    )!;

    expect(separator.getAttribute("aria-orientation")).toBe("vertical");
    expect(separator.getAttribute("aria-valuenow")).toBe("50");
    expect(separator.tabIndex).toBe(0);
    await act(async () => {
      separator.focus();
      separator.dispatchEvent(
        new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }),
      );
    });

    const nextDocument = onChange.mock.lastCall?.[0] as PrintPacket;
    const nextTable = authoredBody(nextDocument)[0];
    expect(nextTable.type).toBe("table");
    if (nextTable.type !== "table") return;
    expect(nextTable.columns[0]?.width).toBeCloseTo(1.1);
    expect(nextTable.columns[1]?.width).toBeCloseTo(0.9);
    expect(nextTable.items).toEqual({
      type: "current_path",
      path: ["lineItems"],
    });
    expect(nextTable.columns.map((column) => column.cell)).toEqual(
      (
        tablePacket.body[0] as Extract<
          PrintPacket["body"][number],
          { type: "table" }
        >
      ).columns.map((column) => column.cell),
    );
  });

  it("resizes a visual columns layout directly on the canvas", async () => {
    const onChange = vi.fn();
    const page = await render(
      <PrintPacketEditor value={gridPacket} onChange={onChange} />,
    );
    const separator = page.querySelector<HTMLElement>(
      '[role="separator"][aria-label="Resize column 1"]',
    )!;

    expect(separator.getAttribute("aria-valuenow")).toBe("67");
    expect(separator.tabIndex).toBe(0);
    await act(async () => {
      separator.dispatchEvent(
        new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }),
      );
    });
    const next = onChange.mock.lastCall?.[0] as PrintPacket;
    const grid = authoredBody(next)[0];
    expect(grid.type).toBe("grid");
    if (grid.type !== "grid") throw new Error("unreachable");
    expect(grid.columns[0]).toBeCloseTo(2.15);
    expect(grid.columns[1]).toBeCloseTo(0.85);
    expect(grid.children.map((child) => child.type)).toEqual([
      "paragraph",
      "paragraph",
    ]);
  });

  it("quick-adds common content into the hovered gap", async () => {
    const onChange = vi.fn();
    const page = await render(
      <PrintPacketEditor value={packet} onChange={onChange} />,
    );
    const slots = page.querySelectorAll<HTMLElement>(
      ".piqae-canvas-insertion-slot",
    );
    expect(slots).toHaveLength(3);

    await act(async () => {
      slots[1]
        ?.querySelector<HTMLButtonElement>('[aria-label="Add content here"]')
        ?.click();
    });
    const menu = slots[1]?.querySelector('[role="menu"]');
    expect(menu?.textContent).toContain("Text");
    expect(menu?.textContent).toContain("Heading");

    await act(async () => {
      Array.from(menu?.querySelectorAll("button") ?? [])
        .find((button) => button.textContent?.includes("Heading"))
        ?.click();
    });
    const next = onChange.mock.lastCall?.[0] as PrintPacket;
    expect(authoredBody(next).map((block) => block.type)).toEqual([
      "paragraph",
      "heading",
      "image",
    ]);
  });

  it("makes toolbar blocks draggable into an exact insertion slot", async () => {
    const onChange = vi.fn();
    const page = await render(
      <PrintPacketEditor value={packet} onChange={onChange} />,
    );
    const textTool = page.querySelector<HTMLButtonElement>(
      'button[aria-label="Text"]',
    );
    const finalSlot = page.querySelectorAll<HTMLElement>(
      ".piqae-canvas-insertion-slot",
    )[2];
    const values = new Map<string, string>();
    const dataTransfer = {
      effectAllowed: "none",
      dropEffect: "none",
      get types() {
        return [...values.keys()];
      },
      setData(type: string, value: string) {
        values.set(type, value);
      },
      getData(type: string) {
        return values.get(type) ?? "";
      },
    } as DataTransfer;

    expect(textTool?.draggable).toBe(true);
    await act(async () => {
      const dragStart = new Event("dragstart", { bubbles: true });
      Object.defineProperty(dragStart, "dataTransfer", { value: dataTransfer });
      textTool?.dispatchEvent(dragStart);
      const drop = new Event("drop", { bubbles: true, cancelable: true });
      Object.defineProperty(drop, "dataTransfer", { value: dataTransfer });
      finalSlot?.dispatchEvent(drop);
    });
    const next = onChange.mock.lastCall?.[0] as PrintPacket;
    expect(authoredBody(next).map((block) => block.type)).toEqual([
      "paragraph",
      "image",
      "paragraph",
    ]);
  });

  it("rejects malformed or untrusted drag payloads", async () => {
    const onChange = vi.fn();
    const page = await render(
      <PrintPacketEditor value={packet} onChange={onChange} />,
    );
    const finalSlot = page.querySelectorAll<HTMLElement>(
      ".piqae-canvas-insertion-slot",
    )[2];
    const dataTransfer = {
      types: ["application/x-piqae-printpacket-block"],
      getData: () => '{"type":"paragraph"}',
      dropEffect: "none",
    } as DataTransfer;

    await act(async () => {
      const drop = new Event("drop", { bubbles: true, cancelable: true });
      Object.defineProperty(drop, "dataTransfer", { value: dataTransfer });
      finalSlot?.dispatchEvent(drop);
    });
    expect(onChange).not.toHaveBeenCalled();
  });

  it("inserts at nested branch paths without flattening the document", () => {
    const conditional = {
      type: "conditional" as const,
      condition: { type: "literal" as const, value: true },
      then: [packet.body[0]!],
      else: [],
    };
    const next = insertBlockAtPath(
      [conditional],
      [
        { branch: "root", index: 0 },
        { branch: "then", index: 0 },
      ],
      { type: "divider" },
    );
    expect(next[0]?.type).toBe("conditional");
    if (next[0]?.type !== "conditional") throw new Error("unreachable");
    expect(next[0].then.map((block) => block.type)).toEqual([
      "divider",
      "paragraph",
    ]);
  });

  it("applies every nested path operation to data-list and table collection branches", () => {
    const paragraph = {
      type: "paragraph" as const,
      content: [{ type: "text" as const, value: "Original" }],
    };
    const replacement = {
      type: "heading" as const,
      level: 2,
      content: [{ type: "text" as const, value: "Replacement" }],
    };
    const list = {
      type: "data_list" as const,
      items: { type: "path" as const, path: ["order", "lineItems"] },
      header: [paragraph, { type: "divider" as const }],
      item: [paragraph, { type: "divider" as const }],
      empty: [paragraph, { type: "divider" as const }],
    };
    const branchBlocks = (
      block: typeof list,
      branch: "header" | "item" | "empty",
    ) => block[branch];

    for (const branch of ["header", "item", "empty"] as const) {
      const path = [
        { branch: "root" as const, index: 0 },
        { branch, index: 0 },
      ];
      expect(siblingsAtPath([list], path)).toEqual(branchBlocks(list, branch));
      const moved = moveBlockAtPath([list], path, 1)[0];
      const replaced = replaceBlockAtPath([list], path, replacement)[0];
      const inserted = insertBlockAtPath(
        [list],
        [
          { branch: "root", index: 0 },
          { branch, index: 1 },
        ],
        { type: "spacer", height_mm: 4 },
      )[0];
      const insertedAfter = insertBlockAfterPath([list], path, {
        type: "spacer",
        height_mm: 4,
      })[0];
      const removed = removeBlockAtPath([list], path)[0];
      expect(moved?.type).toBe("data_list");
      expect(replaced?.type).toBe("data_list");
      expect(inserted?.type).toBe("data_list");
      expect(insertedAfter?.type).toBe("data_list");
      expect(removed?.type).toBe("data_list");
      if (
        moved?.type !== "data_list" ||
        replaced?.type !== "data_list" ||
        inserted?.type !== "data_list" ||
        insertedAfter?.type !== "data_list" ||
        removed?.type !== "data_list"
      )
        throw new Error("unreachable");
      expect(branchBlocks(moved, branch).map((block) => block.type)).toEqual([
        "divider",
        "paragraph",
      ]);
      expect(branchBlocks(replaced, branch)[0]).toEqual(replacement);
      expect(branchBlocks(inserted, branch).map((block) => block.type)).toEqual(
        ["paragraph", "spacer", "divider"],
      );
      expect(
        branchBlocks(insertedAfter, branch).map((block) => block.type),
      ).toEqual(["paragraph", "spacer", "divider"]);
      expect(branchBlocks(removed, branch).map((block) => block.type)).toEqual([
        "divider",
      ]);
    }

    const table = {
      ...(tablePacket.body[0] as Extract<
        PrintPacket["body"][number],
        { type: "table" }
      >),
      empty: [paragraph, { type: "divider" as const }],
    };
    const tablePath = [
      { branch: "root" as const, index: 0 },
      { branch: "empty" as const, index: 0 },
    ];
    expect(siblingsAtPath([table], tablePath)).toEqual(table.empty);
    const nextTable = removeBlockAtPath([table], tablePath)[0];
    expect(nextTable?.type).toBe("table");
    if (nextTable?.type === "table")
      expect(nextTable.empty).toEqual([{ type: "divider" }]);
  });
});
