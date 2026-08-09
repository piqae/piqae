import type {
  CapabilityDocument,
  CapabilityFacet,
  DocumentManifest,
  JobOptions,
  PrintIntent,
  PrintIntentFinding,
  PrintIntentValidation,
} from "./types.js";

export interface PrintIntentBuilderInput {
  printerId: string;
  capabilityRevision: number;
  documentManifest: DocumentManifest;
}

/**
 * Small immutable builder for normalized intent data. It deliberately has no
 * native-option escape hatch: opaque driver configuration remains node-local.
 */
export class PrintIntentBuilder {
  private constructor(private readonly value: PrintIntent) {}

  static create(input: PrintIntentBuilderInput): PrintIntentBuilder {
    if (
      !input.printerId ||
      !Number.isInteger(input.capabilityRevision) ||
      input.capabilityRevision < 1
    ) {
      throw new TypeError(
        "Printer ID and positive capability revision are required",
      );
    }
    return new PrintIntentBuilder({
      schema_version: 1,
      printer_id: input.printerId,
      capability_revision: input.capabilityRevision,
      portable_options: {},
      semantic_options: {},
      document_manifest: structuredClone(input.documentManifest),
    });
  }

  portable(options: JobOptions): PrintIntentBuilder {
    assertNoNativeFields(options, "portable_options");
    return this.with({ portable_options: structuredClone(options) });
  }

  semantic(name: string, value: unknown): PrintIntentBuilder {
    assertFacetName(name);
    assertNoNativeFields(value, `semantic_options.${name}`);
    return this.with({
      semantic_options: {
        ...this.value.semantic_options,
        [name]: structuredClone(value),
      },
    });
  }

  workflow(id: string, revision: number): PrintIntentBuilder {
    return this.with({ workflow: resourceRevision(id, revision) });
  }

  stock(id: string, revision: number): PrintIntentBuilder {
    return this.with({ stock: resourceRevision(id, revision) });
  }

  build(): PrintIntent {
    return structuredClone(this.value);
  }

  private with(change: Partial<PrintIntent>): PrintIntentBuilder {
    return new PrintIntentBuilder({ ...this.value, ...change });
  }
}

/**
 * Pure preliminary validation for responsive integrator UIs. This never
 * replaces authoritative API validation, live driver normalization, stock
 * observation, permissions, or operator checks.
 */
export function preliminarilyValidatePrintIntent(
  intent: PrintIntent,
  capabilities: CapabilityDocument,
): PrintIntentValidation {
  const errors: PrintIntentFinding[] = [];
  const warnings: PrintIntentFinding[] = [];
  let operatorActionRequired = false;
  if (intent.printer_id !== capabilities.printer_id) {
    errors.push(
      finding(
        "printer_mismatch",
        "printer_id",
        "Intent and capability printer differ.",
      ),
    );
  }
  if (intent.capability_revision !== capabilities.revision) {
    errors.push(
      finding(
        "stale_capability_revision",
        "capability_revision",
        "Capabilities must be refreshed before submission.",
      ),
    );
  }
  try {
    assertNoNativeFields(intent.portable_options, "portable_options");
    assertNoNativeFields(intent.semantic_options, "semantic_options");
  } catch (error) {
    errors.push(
      finding(
        "native_options_forbidden",
        "print_intent",
        String((error as Error).message),
      ),
    );
  }
  for (const [name, value] of Object.entries(intent.semantic_options)) {
    try {
      assertFacetName(name);
    } catch (error) {
      errors.push(
        finding(
          "invalid_facet_name",
          `semantic_options.${name}`,
          String((error as Error).message),
        ),
      );
      continue;
    }
    const facet = capabilities.facets[name];
    if (!facet) {
      errors.push(
        finding(
          "unknown_capability_facet",
          `semantic_options.${name}`,
          "The capability document does not declare this normalized facet.",
        ),
      );
      continue;
    }
    if (facet.mutability === "operator_only") {
      operatorActionRequired = true;
      warnings.push(
        finding(
          "operator_authorization_required",
          `semantic_options.${name}`,
          "The node requires an operator to authorize this option.",
        ),
      );
    }
    validateFacet(name, value, facet, errors);
    for (const dependency of facet.dependencies ?? []) {
      if (!(dependency in intent.semantic_options)) {
        errors.push(
          finding(
            "facet_dependency_missing",
            `semantic_options.${name}`,
            `The ${dependency} facet is also required.`,
          ),
        );
      }
    }
    for (const conflict of facet.conflicts ?? []) {
      if (conflict in intent.semantic_options) {
        errors.push(
          finding(
            "facet_conflict",
            `semantic_options.${name}`,
            `The ${conflict} facet cannot be requested at the same time.`,
          ),
        );
      }
    }
  }
  return {
    status:
      errors.length > 0
        ? "invalid"
        : operatorActionRequired
          ? "operator_action_required"
          : "valid",
    capability_revision: capabilities.revision,
    errors,
    warnings,
    normalized_intent: errors.length === 0 ? structuredClone(intent) : null,
  };
}

function validateFacet(
  name: string,
  value: unknown,
  facet: CapabilityFacet,
  errors: PrintIntentFinding[],
): void {
  const path = `semantic_options.${name}`;
  if (
    !facet.supported ||
    facet.mutability === "unsupported" ||
    facet.mutability === "read_only" ||
    facet.mutability === "profile_only"
  ) {
    errors.push(
      finding(
        "facet_not_job_writable",
        path,
        "This facet is not available as a job override.",
      ),
    );
    return;
  }
  const expected =
    facet.type === "integer" || facet.type === "number" ? "number" : facet.type;
  if (
    expected !== "enum" &&
    expected !== "dimensions" &&
    expected !== "object" &&
    typeof value !== expected
  ) {
    errors.push(
      finding("facet_type_mismatch", path, `Expected ${facet.type}.`),
    );
    return;
  }
  if (
    facet.type === "integer" &&
    (!Number.isInteger(value) || typeof value !== "number")
  ) {
    errors.push(finding("facet_type_mismatch", path, "Expected an integer."));
    return;
  }
  if (
    (facet.type === "dimensions" || facet.type === "object") &&
    (typeof value !== "object" || value === null || Array.isArray(value))
  ) {
    errors.push(
      finding("facet_type_mismatch", path, `Expected ${facet.type}.`),
    );
    return;
  }
  if (
    facet.type === "enum" &&
    !(facet.values ?? []).some((candidate) => Object.is(candidate, value))
  ) {
    errors.push(
      finding(
        "facet_value_not_allowed",
        path,
        "Value is not in the advertised enum.",
      ),
    );
  }
  if (typeof value === "number") {
    if (!Number.isFinite(value)) {
      errors.push(
        finding("facet_value_not_finite", path, "Value must be finite."),
      );
      return;
    }
    if (facet.minimum != null && value < facet.minimum)
      errors.push(
        finding(
          "facet_below_minimum",
          path,
          `Value must be at least ${facet.minimum}.`,
        ),
      );
    if (facet.maximum != null && value > facet.maximum)
      errors.push(
        finding(
          "facet_above_maximum",
          path,
          `Value must be at most ${facet.maximum}.`,
        ),
      );
  }
}

function assertNoNativeFields(value: unknown, path: string): void {
  if (Array.isArray(value)) {
    value.forEach((child, index) =>
      assertNoNativeFields(child, `${path}[${index}]`),
    );
    return;
  }
  if (typeof value !== "object" || value === null) return;
  for (const [key, child] of Object.entries(value)) {
    const childPath = `${path}.${key}`;
    if (/^native(?:$|[._-]|[A-Z])/.test(key)) {
      throw new TypeError(`Driver-native field is forbidden: ${childPath}`);
    }
    assertNoNativeFields(child, childPath);
  }
}

function assertFacetName(name: string): void {
  if (
    !/^[a-z][a-z0-9_]*(?:\.[a-z][a-z0-9_]*)+$/.test(name) ||
    name.startsWith("native.")
  ) {
    throw new TypeError("Facet names must be normalized dotted identifiers");
  }
}

function resourceRevision(id: string, revision: number) {
  if (!id || !Number.isInteger(revision) || revision < 1)
    throw new TypeError("Resource revision is invalid");
  return { id, revision };
}

function finding(
  code: string,
  path: string,
  message: string,
): PrintIntentFinding {
  return { code, path, message };
}
