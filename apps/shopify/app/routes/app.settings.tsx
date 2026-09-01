import type { ActionFunctionArgs, LoaderFunctionArgs } from "react-router";
import { Form, useActionData, useLoaderData } from "react-router";
import { useMemo, useRef, useState } from "react";

import shopify from "../shopify.server";
import {
  PRINT_GROUPING_DIMENSIONS,
  parsePrintOrderForm,
  type PrintGroupingDimension,
  type PrintOrderSettings,
} from "../core/print-order";
import { workflows } from "../core/workflows.server";

const DIMENSIONS: Record<
  PrintGroupingDimension,
  { label: string; description: string }
> = {
  primary_product: {
    label: "Primary product",
    description: "Groups by the product with the greatest item quantity.",
  },
  product_mix: {
    label: "Product mix",
    description: "Keeps orders containing the same set of products together.",
  },
  taxonomy: {
    label: "Shopify product category",
    description: "Uses Shopify's standard multi-level product taxonomy.",
  },
  customer: {
    label: "Customer",
    description: "Keeps orders for the same customer together.",
  },
  vendor: {
    label: "Product vendor",
    description: "Groups using the vendor recorded on each Shopify product.",
  },
  product_type: {
    label: "Product type",
    description: "Uses the store's native Shopify product type.",
  },
  product_tags: {
    label: "Product tags",
    description: "Groups by tags attached to products in each order.",
  },
  order_tags: {
    label: "Order tags",
    description: "Groups by the tags attached directly to the order.",
  },
};

export async function loader({ request }: LoaderFunctionArgs) {
  const { session } = await shopify.authenticate.admin(request);
  return { settings: await workflows().getSettings(session.shop) };
}

export async function action({ request }: ActionFunctionArgs) {
  const { session } = await shopify.authenticate.admin(request);
  try {
    const form = await request.formData();
    if (form.get("intent") !== "save-print-order")
      throw new Error("Unsupported settings action");
    const printOrder = parsePrintOrderForm(form.get("printOrder"));
    await workflows().updateSettings(session.shop, { printOrder });
    return { ok: true, error: "" };
  } catch (error) {
    return Response.json(
      {
        ok: false,
        error:
          error instanceof Error
            ? error.message
            : "Settings could not be saved",
      },
      { status: 400 },
    );
  }
}

export default function Settings() {
  const { settings } = useLoaderData<typeof loader>();
  const result = useActionData<typeof action>();
  const initialOrder = useMemo(
    () => [
      ...settings.printOrder.hierarchy,
      ...PRINT_GROUPING_DIMENSIONS.filter(
        (dimension) => !settings.printOrder.hierarchy.includes(dimension),
      ),
    ],
    [settings.printOrder.hierarchy],
  );
  const [ordered, setOrdered] =
    useState<PrintGroupingDimension[]>(initialOrder);
  const [active, setActive] = useState<PrintGroupingDimension[]>(
    settings.printOrder.hierarchy,
  );
  const [taxonomyDepth, setTaxonomyDepth] = useState<
    PrintOrderSettings["taxonomyDepth"]
  >(settings.printOrder.taxonomyDepth);
  const [mixedOrderMode, setMixedOrderMode] = useState<
    PrintOrderSettings["mixedOrderMode"]
  >(settings.printOrder.mixedOrderMode);
  const dragging = useRef<PrintGroupingDimension | null>(null);

  const hierarchy = ordered.filter((dimension) => active.includes(dimension));
  const serialized = JSON.stringify({
    hierarchy,
    taxonomyDepth,
    mixedOrderMode,
  } satisfies PrintOrderSettings);

  const move = (dimension: PrintGroupingDimension, delta: number) => {
    setOrdered((current) => {
      const enabled = current.filter((item) => active.includes(item));
      const priority = enabled.indexOf(dimension);
      const target = enabled[priority + delta];
      if (priority < 0 || !target) return current;
      const from = current.indexOf(dimension);
      const to = current.indexOf(target);
      const next = [...current];
      next[from] = target;
      next[to] = dimension;
      return next;
    });
  };
  const toggle = (dimension: PrintGroupingDimension, enabled: boolean) => {
    if (!enabled) {
      setActive((current) => current.filter((item) => item !== dimension));
      return;
    }
    setOrdered((current) => {
      const next = current.filter((item) => item !== dimension);
      const lastEnabledIndex = next.reduce(
        (last, item, index) => (active.includes(item) ? index : last),
        -1,
      );
      next.splice(lastEnabledIndex + 1, 0, dimension);
      return next;
    });
    setActive((current) =>
      current.includes(dimension) ? current : [...current, dimension],
    );
  };
  const moveRelative = (
    dimension: PrintGroupingDimension,
    target: PrintGroupingDimension,
    after: boolean,
  ) => {
    if (dimension === target) return;
    setOrdered((current) => {
      const next = current.filter((item) => item !== dimension);
      const targetIndex = next.indexOf(target);
      next.splice(targetIndex + (after ? 1 : 0), 0, dimension);
      return next;
    });
  };

  return (
    <s-page heading="Settings">
      <s-section heading="Print order">
        <Form method="post">
          <input type="hidden" name="intent" value="save-print-order" />
          <input type="hidden" name="printOrder" value={serialized} />
          <s-stack direction="block" gap="base">
            {result?.ok ? (
              <s-banner tone="success">Print order saved.</s-banner>
            ) : result?.error ? (
              <s-banner tone="critical">{result.error}</s-banner>
            ) : null}
            <s-paragraph>
              Choose the hierarchy used to arrange selected orders before a
              batch PDF or Node job is generated. Drag enabled rules into
              priority order; the first rule is the broadest grouping.
            </s-paragraph>
            <div className="piqae-grouping-list">
              {ordered.map((dimension) => {
                const enabled = active.includes(dimension);
                const priority = hierarchy.indexOf(dimension);
                return (
                  <div
                    key={dimension}
                    className={`piqae-grouping-rule${enabled ? " is-active" : ""}`}
                    draggable={enabled}
                    onDragStart={() => {
                      dragging.current = dimension;
                    }}
                    onDragEnd={() => {
                      dragging.current = null;
                    }}
                    onDragOver={(event) => {
                      if (!enabled || !dragging.current) return;
                      event.preventDefault();
                    }}
                    onDrop={(event) => {
                      if (!enabled || !dragging.current) return;
                      event.preventDefault();
                      const bounds =
                        event.currentTarget.getBoundingClientRect();
                      moveRelative(
                        dragging.current,
                        dimension,
                        event.clientY >= bounds.top + bounds.height / 2,
                      );
                    }}
                  >
                    <span className="piqae-grouping-handle" aria-hidden="true">
                      ⋮⋮
                    </span>
                    <label className="piqae-grouping-copy">
                      <input
                        type="checkbox"
                        checked={enabled}
                        onChange={(event) =>
                          toggle(dimension, event.currentTarget.checked)
                        }
                      />
                      <span>
                        <strong>
                          {enabled ? `${priority + 1}. ` : ""}
                          {DIMENSIONS[dimension].label}
                        </strong>
                        <small>{DIMENSIONS[dimension].description}</small>
                      </span>
                    </label>
                    <span className="piqae-grouping-buttons">
                      <button
                        type="button"
                        aria-label={`Move ${DIMENSIONS[dimension].label} up`}
                        disabled={!enabled || priority <= 0}
                        onClick={() => move(dimension, -1)}
                      >
                        ↑
                      </button>
                      <button
                        type="button"
                        aria-label={`Move ${DIMENSIONS[dimension].label} down`}
                        disabled={!enabled || priority === hierarchy.length - 1}
                        onClick={() => move(dimension, 1)}
                      >
                        ↓
                      </button>
                    </span>
                  </div>
                );
              })}
            </div>
            <div className="piqae-grouping-options">
              <label className="piqae-field">
                Category level
                <select
                  value={taxonomyDepth}
                  onChange={(event) =>
                    setTaxonomyDepth(
                      event.currentTarget
                        .value as PrintOrderSettings["taxonomyDepth"],
                    )
                  }
                >
                  <option value="broad">Broad department</option>
                  <option value="family">Product family (recommended)</option>
                  <option value="specific">Most specific category</option>
                </select>
              </label>
              <label className="piqae-field">
                Orders containing several groups
                <select
                  value={mixedOrderMode}
                  onChange={(event) =>
                    setMixedOrderMode(
                      event.currentTarget
                        .value as PrintOrderSettings["mixedOrderMode"],
                    )
                  }
                >
                  <option value="dominant">
                    Use the group with the greatest item quantity
                  </option>
                  <option value="contains">
                    Group orders with the same complete mix
                  </option>
                </select>
              </label>
            </div>
            <s-banner tone="info">
              An order is never split or duplicated. “Greatest item quantity” is
              deterministic and handles mixed-category baskets without guessing
              an average taxonomy. Orders with missing metadata are kept
              together at the end.
            </s-banner>
            <s-button type="submit" variant="primary">
              Save print order
            </s-button>
          </s-stack>
        </Form>
      </s-section>
    </s-page>
  );
}
