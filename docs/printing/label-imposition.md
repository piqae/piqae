# Label design and future sheet imposition

Status: product architecture note. Sheet-label imposition is not implemented.

## Keep one logical label in the template

A label template describes one logical label in the open `printpacket/v1`
format. It owns the label's content and nominal dimensions, but it does not
embed Avery coordinates, sheet cells, printer-driver options, or copy ordering.
The same design can therefore target a compatible die-cut roll, continuous
stock, or sheet-label workflow without creating a second template language.

The Shopify Product Label starter follows this rule. It is a 100 x 50 mm
logical label containing the product title, optional variant title, localized
unit price, and a normalized Code 128-safe barcode candidate. If Shopify data
does not contain a candidate that is both encodable and able to fit the label,
the barcode is omitted; generation must not invent or truncate a machine code.

## Impose labels during generation planning

A future generation/print-planning stage should combine the logical label with
the selected immutable target/profile and stock revision. For sheet stock, the
stock definition supplies sheet size, rows and columns, margins, pitch, gaps,
feed direction, rotation, printable bounds, and any registration constraints.
Avery is a family of stock presets, not a template format.

The plan should explicitly contain:

- the ordered logical labels and requested copy count;
- grouped or collated copy ordering;
- the selected stock/profile revisions and compatible printer target;
- the resulting page/cell placement, waste, and page count; and
- the operator-selected starting cell for a partially used sheet.

Mixed products and differing copy counts are expanded into logical labels
before placement. The planner then fills compatible cells deterministically and
produces the final paged artifact or bounded placement plan before the node or
driver handoff.

## Partial-sheet safety

The system must not silently assume that a physical sheet is unused. When an
operator chooses a partial sheet, preview should show the cells that will be
consumed and require an explicit starting row/column or cell index. An unknown
starting position is a blocking warning, not permission to begin at cell one.
Incompatible geometry, overflow, or printable-area violations must fail before
printing. This validation remains non-printing and does not mutate queue or
driver defaults.

## Compatibility boundary

Existing `printpacket/v1` label documents continue to mean one logical label at
their declared dimensions. Roll-label generation and direct one-label output
must keep their current behavior. Future imposition fields should be additive
and versioned in the generation plan and stock/profile contracts; they must not
reinterpret existing template nodes or require a proprietary document format.
Nodes should continue to receive the final renderable artifact or an explicitly
supported plan, while accepted, printing, reported-complete, and uncertain
delivery states remain distinct.

No Avery preset catalogue, sheet planner, starting-position UI, or mixed-label
collation UI is provided by this note.
