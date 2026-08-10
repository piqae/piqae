import type { DetailedHTMLProps, HTMLAttributes } from "react";
import type {} from "@shopify/polaris-types";

declare module "react" {
  namespace JSX {
    interface IntrinsicElements {
      "ui-nav-menu": DetailedHTMLProps<
        HTMLAttributes<HTMLElement>,
        HTMLElement
      >;
    }
  }
}
