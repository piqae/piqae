import { useEffect, useRef, useState } from "react";
import type { Template } from "@pdfme/common";
import {
  visualCompatibility,
  visualTemplate,
  type PdfmeVisualModel,
} from "../core/template-model";

export function PdfmeDesigner({
  value,
  disabled,
  onChange,
}: {
  value: PdfmeVisualModel;
  disabled: boolean;
  onChange(value: PdfmeVisualModel): void;
}) {
  const host = useRef<HTMLDivElement>(null);
  const onChangeRef = useRef(onChange);
  const [error, setError] = useState("");
  onChangeRef.current = onChange;
  useEffect(() => {
    if (!host.current || disabled) return;
    let disposed = false;
    let designer:
      | { onChangeTemplate(cb: (template: Template) => void): void }
      | undefined;
    void Promise.all([import("@pdfme/ui"), import("@pdfme/schemas")])
      .then(([ui, schemas]) => {
        if (disposed || !host.current) return;
        designer = new ui.Designer({
          domContainer: host.current,
          template: visualTemplate(value) as Template,
          plugins: {
            text: schemas.text,
            qrcode: schemas.barcodes.qrcode,
            line: schemas.line,
          },
        });
        designer.onChangeTemplate((template) => {
          const next = {
            ...value,
            fields: [],
            template: template as PdfmeVisualModel["template"],
          };
          onChangeRef.current(next);
          setError(visualCompatibility(next).warnings.join(" "));
        });
      })
      .catch((cause: unknown) =>
        setError(
          cause instanceof Error
            ? cause.message
            : "Visual editor failed to load",
        ),
      );
    return () => {
      disposed = true;
      host.current?.replaceChildren();
      designer = undefined;
    };
    // The Designer owns its state after initialisation.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [disabled]);
  return (
    <div>
      {error ? <s-banner tone="warning">{error}</s-banner> : null}
      {disabled ? (
        <s-banner tone="info">
          Customize this system document to edit its layout.
        </s-banner>
      ) : null}
      <div
        ref={host}
        style={{
          height: "min(76vh, 960px)",
          minHeight: 640,
          width: "100%",
          border: "1px solid #d8d8d8",
          borderRadius: 8,
        }}
      />
    </div>
  );
}
