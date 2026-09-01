/** @jsxImportSource preact */
import { render } from "preact";
import { AdminOrderBrowserPrintAction } from "../../shared/AdminOrderBrowserPrintAction.jsx";

export default async () => {
  render(<AdminOrderBrowserPrintAction bulk />, document.body);
};
