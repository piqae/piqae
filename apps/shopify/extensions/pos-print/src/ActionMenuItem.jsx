import { render } from "preact";

export default async () => {
  render(
    <s-button onClick={() => shopify.action.presentModal()}>
      Print receipt
    </s-button>,
    document.body,
  );
};
