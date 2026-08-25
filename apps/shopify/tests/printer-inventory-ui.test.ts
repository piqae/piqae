import { describe, expect, it, vi } from "vitest";

vi.mock("../app/shopify.server", () => ({
  default: { authenticate: { admin: vi.fn() } },
}));
vi.mock("../app/services.server", () => ({
  createProductionServices: vi.fn(),
}));

import {
  filterPrinterInventory,
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
});
