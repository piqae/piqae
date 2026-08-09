import { render } from "preact";

export default async () => {
  render(<HomeModal />, document.body);
};

export function HomeModal() {
  return (
    <s-page heading="Print documents">
      <s-scroll-box>
        <s-stack direction="block" gap="base">
          <s-banner tone="info">
            Open a completed order and choose Print receipt from its actions.
            Piqae never starts a physical print from this tile.
          </s-banner>
          <s-text>
            The order action confirms the destination before submitting. PDF
            printing remains available through the system dialog.
          </s-text>
          <s-button onClick={() => shopify.action.dismissModal()}>
            Close
          </s-button>
        </s-stack>
      </s-scroll-box>
    </s-page>
  );
}
