# Piqae UI

Piqae UI is the dashboard's small, native Svelte component system. It adopts
the useful qualities of Linear's product UI—neutral chrome, compact controls,
quiet borders, strong alignment, and semantic status colour—without copying
Linear's workflows or depending on a React-only component library.

## Foundations

- `app.css` owns semantic colour, typography, radius, border, shadow, and focus
  tokens. Components use semantic tokens instead of raw palette values.
- Type sizes come from the scale tokens (`--text-title`, `--text-section`,
  `--text-body`, `--text-compact`, `--text-meta`, `--text-code`) and control
  sizes from the geometry tokens (`--control-compact`, `--control-normal`,
  `--control-primary`, `--row-dense`, `--row-normal`). **Never write a literal
  font size below 12 px** — that is the accessibility floor recorded in
  `docs/15-linear-aligned-visual-system.md`.
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
  explicit pressed state. Pass `onchange` when the selection is owned elsewhere,
  such as the query string.
- `DataPanel` provides the bordered, horizontally safe container for
  `.ui-data-table`.
- `Panel` and `SectionHeader` provide the standard bordered section and its
  title/description/actions row.
- `Dialog` owns modal chrome, the escape/backdrop behaviour, and the header and
  footer bands. Put the `<form>` in the body and wire footer submit buttons with
  the `form="…"` attribute so the dialog keeps its own structure.
- `Drawer` is the inline-end detail panel used for query-string-addressed
  detail on the operations page.
- `Field` pairs a label, control, and optional hint.
- `DefinitionList` renders term/value metadata in one or two columns.
- `Metric` renders a headline number with its label and supporting detail.
  Pass a snippet for `detail` when part of it needs emphasis, and
  `tone="attention"` only while the tile actually needs an operator. A tile
  that reads zero stays neutral: healthy systems show it constantly, and one
  that shouts while nothing is wrong is one operators learn to ignore.
- `EmptyState` renders the standard empty message, optionally compact.

Build new dashboard views from these primitives before adding page-local CSS.
Keep product-specific content and behavior in the route; only promote a pattern
when it appears in more than one workflow.

## Structure

The dashboard is two pages — see the information-architecture section of
`docs/15-linear-aligned-visual-system.md`. Adding a route for a list, a detail
view, or a settings sub-page is a regression; add a view or a section instead.
