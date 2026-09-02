/** @jsxImportSource preact */
import { render } from "preact";

import { AdminProductPrintAction } from "../../shared/AdminOrderPrintAction.jsx";

export default async () => {
  render(<AdminProductPrintAction bulk />, document.body);
};
