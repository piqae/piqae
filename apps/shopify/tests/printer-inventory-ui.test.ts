import { describe, expect, it, vi } from "vitest";

vi.mock("../app/shopify.server", () => ({
  default: { authenticate: { admin: vi.fn() } },
}));
vi.mock("../app/services.server", () => ({
  createProductionServices: vi.fn(),
}));

import {
  filterPrinterInventory,
  openPreparedPiqaeConnection,
  preparePiqaeConnectionWindow,
  printerAvailability,
} from "../app/routes/app.printers";

const nodes = new Map([
  [
    "agt_mac",
    { id: "agt_mac", name: "Packing Mac", platform: "macos/aarch64" },
  ],
  ["agt_win", { id: "agt_win", name: "Office PC", platform: "windows/x86_64" }],
]);
const printers = [
  {
    id: "prt_label",
    name: "Shipping labels",
    agent_id: "agt_mac",
    state: "online",
  },
  { id: "prt_a4", name: "Office A4", agent_id: "agt_win", state: "paper_out" },
];

describe("Shopify printer inventory", () => {
  it("groups live spooler states into merchant-facing availability", () => {
    expect(printerAvailability("online")).toBe("available");
    expect(printerAvailability("busy")).toBe("available");
    expect(printerAvailability("offline")).toBe("offline");
    expect(printerAvailability("paper_out")).toBe("attention");
  });

  it("searches printer and computer identity and combines it with status", () => {
    expect(filterPrinterInventory(printers, nodes, "packing", "all")).toEqual([
      printers[0],
    ]);
    expect(
      filterPrinterInventory(printers, nodes, "office", "attention"),
    ).toEqual([printers[1]]);
    expect(
      filterPrinterInventory(printers, nodes, "labels", "attention"),
    ).toEqual([]);
  });

  it("reserves one connection window directly from the merchant gesture", () => {
    const status = { textContent: "", style: { cssText: "" } };
    const replaceChildren = vi.fn();
    const popup = {
      closed: false,
      opener: {},
      document: {
        title: "",
        createElement: vi.fn(() => status),
        body: { replaceChildren },
      },
    } as unknown as Window;
    const openWindow = vi.fn(() => popup) as unknown as typeof window.open;

    expect(preparePiqaeConnectionWindow(openWindow)).toBe(popup);
    expect(openWindow).toHaveBeenCalledWith(
      "",
      "piqae-node-connection",
      "popup,width=560,height=720",
    );
    expect(popup.opener).toBeNull();
    expect(status.textContent).toContain("Preparing");
    expect(replaceChildren).toHaveBeenCalledWith(status);
  });

  it("navigates only a reserved window to the trusted Piqae handoff", () => {
    const replace = vi.fn();
    const popup = {
      closed: false,
      location: { replace },
    } as unknown as Window;

    expect(
      openPreparedPiqaeConnection(
        popup,
        "https://app.piqae.com/connect#one-time-fragment",
      ),
    ).toBe(true);
    expect(replace).toHaveBeenCalledWith(
      "https://app.piqae.com/connect#one-time-fragment",
    );
    expect(
      openPreparedPiqaeConnection(
        popup,
        "https://attacker.example/connect#one-time-fragment",
      ),
    ).toBe(false);
    expect(replace).toHaveBeenCalledTimes(1);
  });
});
