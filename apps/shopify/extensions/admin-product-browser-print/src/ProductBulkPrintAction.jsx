/** @jsxImportSource preact */
import { render } from "preact";

import { AdminProductBrowserPrintAction } from "../../shared/AdminOrderBrowserPrintAction.jsx";

export default async () => {
  render(<AdminProductBrowserPrintAction bulk />, document.body);
};
