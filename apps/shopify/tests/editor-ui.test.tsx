// @vitest-environment jsdom

import { act, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { createRoutesStub } from "react-router";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  insertBlockAtPath,
  PrintPacketEditor,
  PrintPacketPreview,
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

    expect(stage?.querySelector(".piqae-workspace-toolbar [role=group]")).not
      .toBeNull;
    expect(canvas).not.toBeNull();
    expect(canvas?.querySelector(".piqae-canvas-selectable")).toBeNull();
    expect(canvas?.querySelector(".piqae-canvas-badge")).toBeNull();
    expect(canvas?.querySelector(".piqae-canvas-page-break")).toBeNull();
    expect(canvas?.querySelector(".piqae-canvas-empty")).toBeNull();
    expect(batch?.style.gap).toBe("");
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
});
