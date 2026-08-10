/** @jsxImportSource preact */
import { render } from "preact";

export default async () => {
  render(<HomeTile />, document.body);
};

export function HomeTile() {
  return (
    <s-tile
      heading="Print documents"
      subheading="Orders and receipts"
      onClick={() => shopify.action.presentModal()}
    />
  );
}
