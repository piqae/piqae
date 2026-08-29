// @vitest-environment jsdom

import { act, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { createRoutesStub } from "react-router";
import { afterEach, describe, expect, it, vi } from "vitest";
import { PrintPacketEditor } from "../app/components/PrintPacketEditor";
import type { PrintPacket } from "../app/core/template-model";
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

let root: Root | null = null;
let host: HTMLDivElement | null = null;

async function render(node: ReactNode) {
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
  await act(async () => root?.render(node));
  return host;
}

afterEach(async () => {
  await act(async () => root?.unmount());
  host?.remove();
  root = null;
  host = null;
});

describe("Shopify document editor layout", () => {
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
    expect(toolbar?.querySelector(".piqae-selection-rail")).not.toBeNull();

    await act(async () => {
      page
        .querySelector<HTMLElement>(".piqae-canvas-text")
        ?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expect(
      toolbar?.querySelector(".piqae-selection-title")?.textContent,
    ).toContain("Text");
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

    expect(card?.children).toHaveLength(2);
    expect(Array.from(buttons).every((button) => button.tabIndex >= 0)).toBe(
      true,
    );
    expect(page.querySelector(".piqae-page-sheet")).not.toBeNull();
  });
});
