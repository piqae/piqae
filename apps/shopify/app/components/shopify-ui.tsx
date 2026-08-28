import type { ReactNode } from "react";

export type ConnectionState = "ready" | "offline" | "not_connected";
export type OrderRow = {
  id: string;
  name: string;
  date: string;
  customer: string;
  total: string;
  status: string;
  payment: string;
  paymentTone: "success" | "warning" | "critical" | "neutral";
};

export const editorDocument = {
  format: "printpacket/v1",
  media: {
    kind: "paged",
    size: "A4",
    marginsMm: { top: 14, right: 14, bottom: 16, left: 14 },
  },
  body: [
    {
      type: "heading",
      level: 1,
      content: [{ type: "text", value: "Invoice" }],
    },
    {
      type: "section",
      children: [
        { type: "text", content: "Order {{ order.name }}" },
        { type: "text", content: "{{ order.created_at }}" },
      ],
    },
    {
      type: "repeat",
      items: { type: "path", path: ["order", "lineItems"] },
      as: "item",
      children: [
        {
          type: "row",
          children: [
            { type: "text", content: "{{ item.title }} × {{ item.quantity }}" },
            { type: "text", content: "{{ item.total }}" },
          ],
        },
      ],
    },
  ],
} as const;

export function StatusBadge({ state }: { state: ConnectionState }) {
  const label =
    state === "ready"
      ? "Ready"
      : state === "offline"
        ? "Offline"
        : "Not connected";
  const tone =
    state === "ready" ? "success" : state === "offline" ? "warning" : "neutral";
  return <s-badge tone={tone}>{label}</s-badge>;
}

export function EmptyHint({
  heading,
  children,
  action,
  href,
}: {
  heading: string;
  children: ReactNode;
  action: string;
  href: string;
}) {
  return (
    <div className="piqae-card">
      <s-stack direction="block" gap="base">
        <s-heading>{heading}</s-heading>
        <s-paragraph>{children}</s-paragraph>
        <s-button href={href}>{action}</s-button>
      </s-stack>
    </div>
  );
}

export function OrderTable({
  orders,
  selected,
  onSelectionChange,
}: {
  orders: OrderRow[];
  selected: ReadonlySet<string>;
  onSelectionChange: (ids: Set<string>) => void;
}) {
  const allSelected =
    orders.length > 0 && orders.every(({ id }) => selected.has(id));
  const toggleAll = (checked: boolean) =>
    onSelectionChange(
      checked ? new Set(orders.map(({ id }) => id)) : new Set(),
    );
  return (
    <div className="piqae-card">
      <table className="piqae-list">
        <thead>
          <tr>
            <th>
              <input
                aria-label="Select all orders"
                type="checkbox"
                checked={allSelected}
                onChange={(event) => toggleAll(event.currentTarget.checked)}
              />
            </th>
            <th>Order</th>
            <th>Date</th>
            <th>Customer</th>
            <th>Payment</th>
            <th>Total</th>
          </tr>
        </thead>
        <tbody>
          {orders.map((order) => (
            <tr key={order.id}>
              <td data-label="Select">
                <input
                  name="orderIds"
                  value={order.id}
                  aria-label={`Select ${order.name}`}
                  type="checkbox"
                  checked={selected.has(order.id)}
                  onChange={(event) => {
                    const next = new Set(selected);
                    if (event.currentTarget.checked) next.add(order.id);
                    else next.delete(order.id);
                    onSelectionChange(next);
                  }}
                />
              </td>
              <td data-label="Order">
                <strong>{order.name}</strong>
                <div className="piqae-muted">{order.status}</div>
              </td>
              <td data-label="Date">{order.date}</td>
              <td data-label="Customer">{order.customer}</td>
              <td data-label="Payment">
                <s-badge tone={order.paymentTone}>{order.payment}</s-badge>
              </td>
              <td data-label="Total">{order.total}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

export function TemplatePreview() {
  return (
    <div className="piqae-preview" aria-label="Document preview">
      <div className="piqae-paper">
        <h1>Invoice</h1>
        <div className="piqae-actions">
          <strong>Order #1048</strong>
          <span className="piqae-muted">9 August 2026</span>
        </div>
        <hr />
        <p>
          Example product × 2 <span style={{ float: "right" }}>$48.00</span>
        </p>
        <hr />
        <p>
          <strong>Total</strong>
          <strong style={{ float: "right" }}>$48.00</strong>
        </p>
      </div>
    </div>
  );
}
