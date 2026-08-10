import type {
  DocumentCanvasElement,
  DocumentNode,
  DocumentPointer,
  DocumentSpec,
} from "@piqae/sdk";

export type LiquidDiagnostic = {
  code: string;
  line: number;
  message: string;
};

export type LiquidConversion =
  | {
      ok: true;
      document: DocumentSpec;
      normalizedSource: string;
      diagnostics: [];
    }
  | { ok: false; diagnostics: LiquidDiagnostic[] };

const MAX_SOURCE_BYTES = 32_768;
const MAX_LINES = 500;
const MAX_DEPTH = 8;
const MAX_NODES = 500;
const PATH = "[a-zA-Z_][a-zA-Z0-9_]*(?:\\.[a-zA-Z0-9_]+)*";
const OUTPUT = new RegExp(`^\\{\\{\\s*(${PATH})\\s*\\}\\}$`);
const FOR = new RegExp(
  `^\\{%\\s*for\\s+([a-zA-Z_][a-zA-Z0-9_]*)\\s+in\\s+(${PATH})\\s*%\\}$`,
);
const IF = new RegExp(`^\\{%\\s*if\\s+(${PATH})\\s*%\\}$`);
const QR = new RegExp(
  `^\\{%\\s*piqae_qr\\s+(${PATH})(?:\\s+size_mm:\\s*(\\d+(?:\\.\\d+)?))?\\s*%\\}$`,
);
const SPACER = /^\{%\s*piqae_spacer\s+(\d+(?:\.\d+)?)\s*%\}$/;
const CANVAS_START = /^\{%\s*piqae_canvas\s*%\}$/;
const CANVAS_END = /^\{%\s*endpiqae_canvas\s*%\}$/;
const CANVAS_ITEM = /^\{%\s*piqae_canvas_(text|qr|line)\s+(.+?)\s*%\}$/;
const CANVAS_VALUE = /^("(?:[^"\\]|\\.)*"|[a-zA-Z_][a-zA-Z0-9_.]*)\s*/;
const CANVAS_ARG = /^(x|y|width|height|font_size):\s*(\d+(?:\.\d+)?)\s*/;

type Frame = {
  kind: "root" | "for" | "if" | "canvas";
  variable?: string;
  children: DocumentNode[];
  line: number;
};

/**
 * Compiles a non-executing Liquid subset into the canonical Piqae document.
 * There are intentionally no filters, HTML, includes, assignments or plugins.
 */
export function liquidToCanonical(
  source: string,
  page: DocumentSpec["page"],
): LiquidConversion {
  const diagnostics: LiquidDiagnostic[] = [];
  if (new TextEncoder().encode(source).byteLength > MAX_SOURCE_BYTES)
    return failure("source_too_large", 1, "Liquid source exceeds 32 KiB.");
  const lines = source.replaceAll("\r\n", "\n").split("\n");
  if (lines.length > MAX_LINES)
    return failure(
      "too_many_lines",
      MAX_LINES + 1,
      `Liquid source supports at most ${MAX_LINES} lines.`,
    );
  const root: Frame = { kind: "root", children: [], line: 1 };
  const stack: Frame[] = [root];
  let nodes = 0;
  const add = (node: DocumentNode, line: number) => {
    nodes += 1;
    if (nodes > MAX_NODES)
      diagnostics.push({
        code: "too_many_nodes",
        line,
        message: `Liquid source supports at most ${MAX_NODES} nodes.`,
      });
    else stack.at(-1)!.children.push(node);
  };
  for (
    let index = 0;
    index < lines.length && diagnostics.length === 0;
    index += 1
  ) {
    const lineNumber = index + 1;
    const text = lines[index]!.trim();
    if (
      !text ||
      (text.startsWith("{% comment %}") && text.endsWith("{% endcomment %}"))
    )
      continue;
    const output = OUTPUT.exec(text);
    if (output) {
      const pointer = pointerFor(output[1]!, stack);
      if (!pointer)
        diagnostics.push({
          code: "unknown_variable",
          line: lineNumber,
          message: `Variable '${output[1]}' is not in the document or current loop scope.`,
        });
      else add({ type: "text", value: { pointer } }, lineNumber);
      continue;
    }
    const loop = FOR.exec(text);
    if (loop) {
      if (stack.length > MAX_DEPTH)
        diagnostics.push({
          code: "nesting_too_deep",
          line: lineNumber,
          message: `Liquid blocks support at most ${MAX_DEPTH} levels.`,
        });
      else {
        const pointer = absolutePointer(loop[2]!);
        const frame: Frame = {
          kind: "for",
          variable: loop[1],
          children: [],
          line: lineNumber,
        };
        add({ type: "repeat", pointer, children: frame.children }, lineNumber);
        stack.push(frame);
      }
      continue;
    }
    const condition = IF.exec(text);
    if (condition) {
      if (stack.length > MAX_DEPTH)
        diagnostics.push({
          code: "nesting_too_deep",
          line: lineNumber,
          message: `Liquid blocks support at most ${MAX_DEPTH} levels.`,
        });
      else {
        const pointer = pointerFor(condition[1]!, stack);
        if (!pointer || pointer === "." || pointer.startsWith("./"))
          diagnostics.push({
            code: "unsupported_condition",
            line: lineNumber,
            message: "Conditions must use a root document variable.",
          });
        else {
          const frame: Frame = { kind: "if", children: [], line: lineNumber };
          add(
            {
              type: "when",
              pointer: pointer as `/${string}`,
              children: frame.children,
            },
            lineNumber,
          );
          stack.push(frame);
        }
      }
      continue;
    }
    if (text === "{% endfor %}" || text === "{% endif %}") {
      const expected = text === "{% endfor %}" ? "for" : "if";
      if (stack.at(-1)!.kind !== expected)
        diagnostics.push({
          code: "unmatched_end_tag",
          line: lineNumber,
          message: `${text} does not close the current block.`,
        });
      else stack.pop();
      continue;
    }
    if (text === "{% piqae_line %}") {
      add({ type: "line" }, lineNumber);
      continue;
    }
    if (CANVAS_START.test(text)) {
      if (stack.some((frame) => frame.kind === "canvas"))
        diagnostics.push({
          code: "nested_canvas",
          line: lineNumber,
          message: "Canvas blocks cannot be nested.",
        });
      else {
        const frame: Frame = {
          kind: "canvas",
          children: [],
          line: lineNumber,
        };
        add(
          {
            type: "canvas",
            children: frame.children as DocumentCanvasElement[],
          },
          lineNumber,
        );
        stack.push(frame);
      }
      continue;
    }
    if (CANVAS_END.test(text)) {
      if (stack.at(-1)!.kind !== "canvas")
        diagnostics.push({
          code: "unmatched_end_tag",
          line: lineNumber,
          message: "{% endpiqae_canvas %} does not close a canvas block.",
        });
      else stack.pop();
      continue;
    }
    const canvasItem = CANVAS_ITEM.exec(text);
    if (canvasItem) {
      if (stack.at(-1)!.kind !== "canvas") {
        diagnostics.push({
          code: "canvas_item_outside_canvas",
          line: lineNumber,
          message: "Canvas items must be inside a piqae_canvas block.",
        });
        continue;
      }
      const parsed = parseCanvasItem(canvasItem[1]!, canvasItem[2]!, stack);
      if ("diagnostic" in parsed)
        diagnostics.push({ ...parsed.diagnostic, line: lineNumber });
      else add(parsed.node, lineNumber);
      continue;
    }
    if (text === "{% piqae_page_break %}") {
      add({ type: "page_break" }, lineNumber);
      continue;
    }
    const qr = QR.exec(text);
    if (qr) {
      const pointer = pointerFor(qr[1]!, stack);
      const size = qr[2] === undefined ? undefined : Number(qr[2]);
      if (!pointer)
        diagnostics.push({
          code: "unknown_variable",
          line: lineNumber,
          message: `Variable '${qr[1]}' is not in the document or current loop scope.`,
        });
      else if (size !== undefined && (size < 5 || size > 100))
        diagnostics.push({
          code: "invalid_qr_size",
          line: lineNumber,
          message: "QR size must be between 5 and 100 mm.",
        });
      else
        add(
          {
            type: "qr",
            value: { pointer },
            ...(size === undefined ? {} : { size_mm: size }),
          },
          lineNumber,
        );
      continue;
    }
    const spacer = SPACER.exec(text);
    if (spacer) {
      const height = Number(spacer[1]);
      if (height > 100)
        diagnostics.push({
          code: "invalid_spacer",
          line: lineNumber,
          message: "Spacer height must be at most 100 mm.",
        });
      else add({ type: "spacer", height_mm: height }, lineNumber);
      continue;
    }
    if (text.includes("{{") || text.includes("{%") || /<[^>]+>/.test(text)) {
      diagnostics.push({
        code: "unsupported_construct",
        line: lineNumber,
        message:
          "Only whole-line variables, for/if blocks, and documented piqae_* tags are supported; HTML, filters and other Liquid are disabled.",
      });
      continue;
    }
    add({ type: "text", value: text }, lineNumber);
  }
  if (diagnostics.length === 0 && stack.length > 1) {
    const frame = stack.at(-1)!;
    diagnostics.push({
      code: "unclosed_block",
      line: frame.line,
      message: `The ${frame.kind} block is not closed.`,
    });
  }
  if (diagnostics.length) return { ok: false, diagnostics };
  const document: DocumentSpec = {
    spec_version: "piqae.document/v1",
    page,
    body: root.children,
  };
  return {
    ok: true,
    document,
    normalizedSource: canonicalToLiquid(document).source!,
    diagnostics: [],
  };
}

export function canonicalToLiquid(document: DocumentSpec): {
  source?: string;
  diagnostics: LiquidDiagnostic[];
} {
  const diagnostics: LiquidDiagnostic[] = [];
  const lines: string[] = [];
  let loopIndex = 0;
  const emit = (nodes: DocumentNode[], scope?: string) => {
    for (const node of nodes) {
      if (node.type === "text") {
        if (typeof node.value === "string") lines.push(node.value);
        else lines.push(`{{ ${pathFor(node.value.pointer, scope)} }}`);
      } else if (node.type === "line") lines.push("{% piqae_line %}");
      else if (node.type === "page_break") lines.push("{% piqae_page_break %}");
      else if (node.type === "spacer")
        lines.push(`{% piqae_spacer ${node.height_mm} %}`);
      else if (node.type === "qr") {
        if (typeof node.value === "string")
          diagnostics.push({
            code: "literal_qr_unsupported",
            line: lines.length + 1,
            message: "Liquid QR tags require a variable binding.",
          });
        else
          lines.push(
            `{% piqae_qr ${pathFor(node.value.pointer, scope)}${node.size_mm === undefined ? "" : ` size_mm: ${node.size_mm}`} %}`,
          );
      } else if (node.type === "repeat") {
        const variable = `item${loopIndex++}`;
        lines.push(`{% for ${variable} in ${pathFor(node.pointer)} %}`);
        emit(node.children, variable);
        lines.push("{% endfor %}");
      } else if (node.type === "when") {
        lines.push(`{% if ${pathFor(node.pointer)} %}`);
        emit(node.children, scope);
        lines.push("{% endif %}");
      } else if (node.type === "canvas") {
        lines.push("{% piqae_canvas %}");
        for (const child of node.children) {
          const box = `x: ${child.x_mm} y: ${child.y_mm} width: ${child.width_mm} height: ${child.height_mm}`;
          if (child.type === "line")
            lines.push(`{% piqae_canvas_line ${box} %}`);
          else if (child.type === "text")
            lines.push(
              `{% piqae_canvas_text ${liquidCanvasValue(child.value, scope)} ${box} font_size: ${child.font_size ?? 10} %}`,
            );
          else
            lines.push(
              `{% piqae_canvas_qr ${liquidCanvasValue(child.value, scope)} ${box} %}`,
            );
        }
        lines.push("{% endpiqae_canvas %}");
      } else
        diagnostics.push({
          code: "unsupported_node",
          line: lines.length + 1,
          message: `${node.type} cannot be represented by bounded Liquid.`,
        });
    }
  };
  emit(document.body);
  return diagnostics.length
    ? { diagnostics }
    : { source: lines.join("\n"), diagnostics };
}

function liquidCanvasValue(
  value: string | { pointer: DocumentPointer },
  scope?: string,
): string {
  return typeof value === "string"
    ? JSON.stringify(value)
    : pathFor(value.pointer, scope);
}

function parseCanvasItem(
  type: string,
  source: string,
  stack: Frame[],
): { node: DocumentNode } | { diagnostic: Omit<LiquidDiagnostic, "line"> } {
  let rest = source;
  let value: string | { pointer: DocumentPointer } | undefined;
  if (type !== "line") {
    const match = CANVAS_VALUE.exec(rest);
    if (!match)
      return canvasFailure(
        "invalid_canvas_value",
        "Canvas text and QR require a quoted literal or variable.",
      );
    rest = rest.slice(match[0].length);
    if (match[1]!.startsWith('"')) {
      try {
        value = JSON.parse(match[1]!) as string;
      } catch {
        return canvasFailure(
          "invalid_canvas_value",
          "Canvas literal is not valid JSON text.",
        );
      }
    } else {
      const pointer = pointerFor(match[1]!, stack);
      if (!pointer)
        return canvasFailure(
          "unknown_variable",
          `Variable '${match[1]}' is not in scope.`,
        );
      value = { pointer };
    }
  }
  const args = new Map<string, number>();
  while (rest.trim()) {
    const match = CANVAS_ARG.exec(rest.trimStart());
    if (!match)
      return canvasFailure(
        "invalid_canvas_argument",
        "Canvas arguments must be x, y, width, height or font_size numbers.",
      );
    if (args.has(match[1]!))
      return canvasFailure(
        "duplicate_canvas_argument",
        `Canvas argument '${match[1]}' is duplicated.`,
      );
    args.set(match[1]!, Number(match[2]));
    rest = rest.trimStart().slice(match[0].length);
  }
  for (const required of ["x", "y", "width", "height"])
    if (!args.has(required))
      return canvasFailure(
        "missing_canvas_argument",
        `Canvas argument '${required}' is required.`,
      );
  const box = {
    x_mm: args.get("x")!,
    y_mm: args.get("y")!,
    width_mm: args.get("width")!,
    height_mm: args.get("height")!,
  };
  if (box.width_mm <= 0 || box.height_mm <= 0)
    return canvasFailure(
      "invalid_canvas_box",
      "Canvas width and height must be positive.",
    );
  if (type === "line") return { node: { type: "line", ...box } };
  if (type === "text")
    return {
      node: {
        type: "text",
        value: value!,
        font_size: args.get("font_size") ?? 10,
        ...box,
      },
    };
  return { node: { type: "qr", value: value!, ...box } };
}

function canvasFailure(code: string, message: string) {
  return { diagnostic: { code, message } };
}

function absolutePointer(path: string): `/${string}` {
  return `/${path.replaceAll(".", "/")}`;
}
function pointerFor(path: string, stack: Frame[]): DocumentPointer | undefined {
  const loop = [...stack].reverse().find((frame) => frame.kind === "for");
  if (
    loop?.variable &&
    (path === loop.variable || path.startsWith(`${loop.variable}.`))
  ) {
    const rest = path.slice(loop.variable.length).replace(/^\./, "");
    return rest ? `./${rest.replaceAll(".", "/")}` : ".";
  }
  return absolutePointer(path);
}
function pathFor(pointer: DocumentPointer, scope?: string): string {
  if (pointer === ".") return scope ?? "item";
  if (pointer.startsWith("./"))
    return `${scope ?? "item"}.${pointer.slice(2).replaceAll("/", ".")}`;
  return pointer.slice(1).replaceAll("/", ".");
}
function failure(
  code: string,
  line: number,
  message: string,
): LiquidConversion {
  return { ok: false, diagnostics: [{ code, line, message }] };
}
