# Piqae UI

Piqae UI is the dashboard's small, native Svelte component system. It adopts
the useful qualities of Linear's product UI—neutral chrome, compact controls,
quiet borders, strong alignment, and semantic status colour—without copying
Linear's workflows or depending on a React-only component library.

## Foundations

- `app.css` owns semantic colour, typography, radius, border, shadow, and focus
  tokens. Components use semantic tokens instead of raw palette values.
- Inter Variable is the interface face. Display text uses the optical sizing
  axis through `--font-display`; dense UI uses `--font-sans`.
- Blue is Piqae's brand and focus hue. It is intentionally reserved for
  identity, selection, focus, and enabled controls. Status colours keep their
  operational meaning.
- Dark and light themes share the same component contract and meet the same
  focus and state requirements.

## Primitives

- `Toolbar` aligns filters, search, and result metadata.
- `SearchField` provides the common compact search control.
- `SegmentedControl` provides small mutually exclusive view filters with
  explicit pressed state.
- `DataPanel` provides the bordered, horizontally safe container for
  `.ui-data-table`.

Build new dashboard views from these primitives before adding page-local CSS.
Keep product-specific content and behavior in the route; only promote a pattern
when it appears in more than one workflow.
