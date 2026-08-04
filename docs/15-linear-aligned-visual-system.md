# Linear-aligned visual system

Status: required for production v0.1  
Reference date: 29 July 2026

## Intent

The hosted dashboard and loopback web interface should feel extremely close to
Linear's current application at first glance and during extended use.

The target is near one-to-one **visual grammar**:

- palette character;
- contrast and elevation;
- density and spacing rhythm;
- typography and numeric treatment;
- border, radius, and shadow treatment;
- icon scale and optical weight;
- control sizing;
- hover, focus, selected, disabled, and loading states;
- animation timing and restraint;
- the balance between quiet chrome and focused content.

The target is not one-to-one UX or layout. Printer fleet navigation, job
submission, queue operations, diagnostic timelines, and agent enrollment must
use the information architecture that best fits this product.

Do not copy:

- Linear's logo, name, marks, illustrations, or marketing assets;
- proprietary icon drawings;
- exact product screen compositions;
- issue/project terminology where it does not fit printing;
- empty-state copy, onboarding copy, or distinctive artwork;
- source CSS, private tokens, or reverse-engineered application code.

Our product should be recognizable as its own open-source printing platform
while having the same level of calmness, precision, density, and finish.

## Current Linear reference

Linear's March 2026 public design refresh describes the relevant direction:

- navigation recedes so working content remains visually dominant;
- inactive text is muted and icons are smaller;
- tabs and controls are more compact;
- unnecessary icon treatments are removed;
- separators are fewer, softer, and more rounded;
- the default palette moved from cool blue toward a warmer crisp gray;
- hue, chroma, lightness, and contrast are controlled through design tokens;
- dense information remains scannable without giving every element equal
  visual weight.

The 2024 redesign notes also describe LCH-based elevation, base/accent/contrast
theme inputs, Inter Display for headings, and Inter for interface text.

These public principles are our reference. Screenshot comparison is used for
optical tuning, not pixel extraction or copying product layouts.

## Product-specific visual principles

### 1. Content earns attention

The job state, failure reason, selected printer, and required action receive the
highest contrast. Sidebar destinations, secondary metadata, timestamps, and
ambient fleet information recede.

Do not use bright cards and colored badges for every metric. Most screens
should be neutral. Color communicates state or action.

### 2. Structure is felt rather than outlined

Prefer spacing, surface changes, and alignment over boxed sections. Use borders
only where they explain containment or selection.

Avoid:

- a card around every group;
- thick divider lines;
- nested rounded rectangles;
- large shadows;
- gradients on routine controls;
- decorative glass effects on operational screens.

### 3. Dense but not cramped

The dashboard is an operational tool. A user should scan many agents, printers,
and jobs without excessive scrolling. Density comes from consistent alignment,
small icons, quiet metadata, and controlled row heights—not tiny unreadable
text.

### 4. Fast and quiet

Transitions clarify state and location. They do not perform for the user.
Routine interactions should feel immediate, with no springy overshoot or long
page entrance animation.

### 5. Dark and light are first-class

Dark mode is the visual north star, but light mode is not an inversion after the
fact. Both themes share semantic tokens and pass accessibility checks.

## Token architecture

Use CSS custom properties as the canonical runtime tokens. Store source values
as OKLCH so hue, chroma, lightness, and contrast can be tuned systematically.
Components consume semantic tokens and never reference raw colors.

```text
primitive
  neutral lightness/chroma steps
  accent scale
  red, amber, green, blue semantic ramps

semantic
  canvas
  sidebar
  surface
  surface-raised
  surface-overlay
  surface-hover
  surface-selected
  border-subtle
  border-default
  text-primary
  text-secondary
  text-tertiary
  icon-primary
  icon-muted
  focus-ring
  status-success/warning/danger/info

component
  button
  input
  menu
  dialog
  table-row
  status
  timeline
```

Theme generation begins from three user-facing inputs:

- neutral base hue/temperature;
- accent color;
- contrast preference.

The production defaults use warm, nearly achromatic gray surfaces with a
restrained violet/indigo accent. The theme engine must also support increased
contrast without changing component code.

### Initial dark theme targets

These are starting optical targets, not extracted Linear values:

- canvas: very dark warm neutral, never pure black;
- sidebar: slightly darker or lower-contrast than the working surface;
- primary surface: one small lightness step above canvas;
- raised surface: another small step, with a soft border;
- primary text: off-white rather than pure white;
- secondary text: clearly readable but visually recessed;
- tertiary text: reserved for nonessential metadata;
- separators: low-chroma white at very low alpha;
- accent: used for primary action, focus, links, and selected emphasis only.

### Initial light theme targets

- canvas: soft warm gray rather than stark white;
- main working surface: near-white with a subtle temperature;
- sidebar: slightly dimmer than main content;
- primary text: near-black warm neutral;
- separators: neutral dark at low alpha;
- accent chroma reduced enough to remain calm on light surfaces.

### Contrast

- functional text and controls meet WCAG AA;
- focus indicators remain visible in both themes;
- error and warning meaning never depends on hue alone;
- the high-contrast theme is generated from the same semantic model;
- decorative tertiary text may be softer, but no required job data may fall
  below the accessible threshold.

## Typography

Use the open Inter family:

- Inter Display for page titles and selected high-level headings;
- Inter for UI labels, body, menus, tables, and forms;
- a system monospace stack for job IDs, hashes, raw formats, and logs;
- tabular numerals for counts, latency, pages, timestamps, and queue depth.

Initial scale:

| Role | Size | Weight | Line height |
| --- | ---: | ---: | ---: |
| Page title | 18–20 px | 550–600 | 24–28 px |
| Section title | 13–14 px | 550–600 | 20 px |
| Body/UI | 13 px | 400–500 | 19–20 px |
| Compact row | 12–13 px | 400–500 | 18 px |
| Metadata | 11–12 px | 400–500 | 16–18 px |
| Code/log | 12 px | 400–500 | 18 px |

Use weight and tone before increasing size. Avoid oversized SaaS-dashboard
headings and excessively bold labels.

These sizes are published as tokens in `apps/web/src/app.css`
(`--text-title`, `--text-section`, `--text-body`, `--text-compact`,
`--text-meta`, `--text-code`). **12 px is a hard floor**: no dashboard component
may set a smaller size, because functional text below it fails the WCAG AA gate
listed under Visual release gates. Prefer a token over a literal `px` value so a
future scale change stays global.

## Geometry and density

Use a 4 px base spacing grid with deliberate 2 px optical corrections.

Recommended starting dimensions:

- compact control height: 28 px;
- normal control height: 32 px;
- primary form control: 36 px where clarity requires;
- dense data row: 32 px;
- normal data row: 36 px;
- sidebar destination: 28–30 px;
- icon-only button: 28 px;
- common icon: 14 px;
- prominent icon: 16 px;
- corner radii: 4, 6, 8, and 10 px;
- overlay radius: 10–12 px;
- sidebar width: determined by our labels, not copied from Linear.

Pills are reserved for state, filters, and genuinely compact grouped controls.
Routine rectangular buttons use modest radii rather than becoming capsules.

## Surfaces, borders, and shadows

Surface hierarchy:

1. application canvas;
2. navigation/sidebar;
3. main working surface;
4. raised panels and sticky controls;
5. menus/dialogs/popovers.

Each step uses small lightness and chroma changes. Avoid dramatic surface jumps.

Borders:

- one CSS pixel;
- low contrast;
- slightly stronger only for input focus, selected rows, and destructive
  confirmation;
- rounded where a line terminates visibly;
- removed when spacing already communicates the grouping.

Shadows:

- none for routine panels;
- a short, low-alpha shadow plus border for menus and popovers;
- a broader but still restrained shadow for modal dialogs;
- no colored glow around ordinary cards.

## Icons and status

Use an open-source icon set as the base, normalized through our own wrapper:

- 14 or 16 px standard view box;
- consistent 1.5 px optical stroke;
- `currentColor`;
- no filled colored background by default;
- exact baseline and label gap tokens;
- custom printing glyphs designed in our own visual language.

Do not trace Linear icons.

Operational status must combine:

- icon or simple geometric marker;
- concise label;
- color;
- tooltip/detail where ambiguity remains.

Reserve saturated red for required intervention and dangerous actions. Offline
is usually neutral or amber, not automatically catastrophic. Green should not
cover every healthy item; healthy is often the quiet default.

## Motion

Initial timing:

- hover/color response: 80–120 ms;
- menu/popover: 120–160 ms;
- dialog/drawer: 160–200 ms;
- row insertion or state change: 140–180 ms;
- skeleton shimmer: subtle and slow, with reduced-motion alternative.

Use ease-out for entry and ease-in for exit. Movement distances stay below
8 px for routine overlays. Respect `prefers-reduced-motion`.

Live job events should update without flashing the entire row. Animate only the
changed marker or newly inserted event, then settle immediately.

## Dashboard information architecture

The hosted dashboard is deliberately small. It has two destinations:

1. `/dashboard` — the operational surface. Jobs, printers, nodes (and customers
   where the accounts feature is enabled) are **views of one page**, selected
   with `?view=`. Detail opens in a right-hand drawer addressed by
   `?job=`/`?printer=`/`?node=`/`?customer=`, so a link to a failing job stays
   shareable without a route of its own.
2. `/dashboard/settings` — every configuration surface as anchored sections:
   API keys, webhooks, team, billing, printing policy, retention, deployment.

`/dashboard/local` remains separate: it is the loopback profile-capture view and
is only meaningful on a local install.

Routes that previously existed for each list, each detail view, and each
settings sub-page are kept as 308 redirects into the above. Do not reintroduce a
page whose only job is to link to other pages.

### Recorded deviation: top bar instead of sidebar

This document originally required a "quiet sidebar destination and section"
primitive. With two destinations a 218 px sidebar is mostly empty chrome, so the
dashboard uses a 48 px top bar and reclaims the horizontal space for dense
tables. The sidebar primitive is therefore **not** part of the required set
below. Everything else in this document still applies.

## Required component set

No production screen invents local CSS before these primitives exist:

- application frame;
- location/view header;
- button and icon button;
- text field, select, combobox, checkbox, and switch;
- segmented control;
- command/search palette;
- menu and context menu;
- tooltip;
- popover;
- dialog and confirmation dialog;
- toast;
- tabs;
- data table and grouped list;
- empty/loading/error state;
- status marker and status label;
- agent/printer presence indicator;
- job timeline event;
- log/code viewer;
- metric value;
- filter chip;
- keyboard shortcut hint;
- skeleton.

Build these as Svelte components consuming semantic tokens. Prefer headless
behavior primitives where they save accessibility work, but own all visible
styling. Do not import a pre-styled component kit that makes the product look
like a generic template.

## Representative production screens

The visual system is stress-tested against our own workflows:

1. agent/printer fleet overview;
2. dense jobs table with filters and mixed states;
3. job detail with event timeline, content, options, and diagnostics;
4. agent enrollment;
5. printer capabilities and defaults;
6. API key and webhook settings;
7. command palette;
8. destructive retry/cancel/revoke confirmation;
9. offline, empty, partial-failure, and permission-denied states;
10. local diagnostics view.

The page structures are designed for printing operations. Only their visual
treatment aligns closely with Linear.

## Native tray/menu relationship

The Windows tray, macOS menu, and Linux notifier follow native OS conventions.
Do not force web styling into an operating-system menu.

Alignment comes through:

- our product icon and status symbols;
- the same concise terminology;
- quiet status hierarchy;
- hosted/loopback pages opened from the shell;
- consistent dark/light icon assets where the OS permits.

Native conventions take priority over visual imitation.

## Forty-eight-hour visual workstream

### Hours 0–2: visual contract

- capture approved public Linear references in a private reference board;
- write the do-copy/do-not-copy boundary;
- freeze fonts, base spacing, radius, icon sizes, motion, and theme inputs;
- define the initial dark/light semantic token JSON;
- select three stress-test screens.

### Hours 2–6: foundations

- implement CSS token generation and theme switching;
- install and self-host Inter/Inter Display with licence files;
- create typography, icon, focus, surface, and motion foundations;
- create an internal `/design-system` component-gallery route;
- set up fixed-size dark/light screenshot tests.

### Hours 6–14: primitives in parallel

- build the required component set in owned groups;
- merge every component through the gallery;
- test hover/focus/disabled/loading/error/selected states;
- implement keyboard and screen-reader behavior with the component;
- prevent feature lanes from adding unreviewed visual primitives.

### Hours 14–24: production screens

- apply components to onboarding, fleet, jobs, job detail, and settings;
- perform optical comparison at matching viewport and theme;
- tune token values globally rather than patching individual pages;
- complete responsive and long-content states;
- freeze feature styling at hour 24.

### Hours 24–32: visual and accessibility gate

- run dark/light screenshot regression;
- test 100%, 125%, and 200% zoom;
- test keyboard-only navigation and visible focus;
- test reduced motion;
- test high-contrast generation and color-vision simulations;
- resolve overflow, truncation, loading shift, and empty/error-state defects.

### Hours 32–40: production polish

- tune typography baselines, icon alignment, row density, and overlay placement;
- verify real job timelines do not flash or reflow excessively;
- remove one-off CSS and unused variants;
- performance-test fonts, icons, and animation;
- confirm no Linear asset or proprietary icon entered the repository.

### Hours 40–48: canary

- include UI snapshots in the release gate;
- test production data density and unusually long printer names/errors;
- dogfood both themes during the operational canary;
- publish screenshots using our own data and identity;
- accept only launch-blocking consistency/accessibility corrections.

## Visual release gates

- all production screens use semantic tokens and approved primitives;
- no dashboard component sets a font size below 12 px;
- approved dark/light screenshots have no unexplained regression;
- visual hierarchy remains intact with realistic dense data;
- no one-off hard-coded color in a feature component;
- no copied Linear asset, logo, illustration, icon, or product-specific text;
- keyboard flow and focus are complete;
- functional text and controls meet WCAG AA;
- reduced-motion mode is complete;
- 200% zoom remains usable;
- loading and live updates avoid destructive layout shift;
- designers/owners approve the perceptual Linear-style alignment at the three
  frozen reference viewports.

## Sources

- [Linear's 2026 interface refresh](https://linear.app/now/behind-the-latest-design-refresh)
- [Linear's 2026 UI refresh changelog](https://linear.app/changelog/2026-03-12-ui-refresh)
- [How Linear redesigned its UI in 2024](https://linear.app/now/how-we-redesigned-the-linear-ui)
- [Linear appearance and theme preferences](https://linear.app/docs/account-preferences)
