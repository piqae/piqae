/** @jsxImportSource preact */
import { render } from "preact";

export default async () => {
  render(
    <s-button onClick={() => shopify.action.presentModal()}>
      Print product label
    </s-button>,
    document.body,
  );
};
