import type { DetailedHTMLProps, HTMLAttributes } from "react";
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
