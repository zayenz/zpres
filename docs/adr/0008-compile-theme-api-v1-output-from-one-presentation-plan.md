# Compile Theme API v1 output from one Presentation plan

Theme API 1 compiles one typed Presentation plan from the prepared Deck.
The plan is the authority for screen stacks and Step routes, static pages and
note pages, logical slide numbers, background application, page counts, and
expected browser identities. Renderers and validators adapt that plan; they do
not infer policy from loop indexes.

The first Section's Main slide is always the Title phase. It is clean by
default. `background_image.title: paint` paints the Deck background, while a
slide-local background wins; neither choice turns the Slide into a Content
phase. Theme API v1 generates a Splash only for explicit `splash: true`. The
Splash follows the complete first Section. A speaker-notes page follows its
owning Slide's final static Step, so Details and their notes remain before the
Splash.

A Deck background that no planned page paints is valid but dormant. zpres
warns, yet still validates, copies, watches, and hashes it. Unknown, malformed,
duplicate, and Deck-only fields used in a slide background are fatal for v1 at
their Source location. An omitted Splash value means false.

Browser evidence is order-sensitive. Screen validation checks route hashes and
every section, Detail, and Step coordinate. Print validation checks Slide,
role, generated kind, Step policy, and Step number. A planned absence is also a
contract: a clean title or notes page fails if Chromium finds an unexpected
semantic background layer.

Visual provenance schema 4 records the Presentation-plan digest and hashes all
Deck and slide background files. Source and Theme package hashes remain
separate, so changing only a background asset is visible in release evidence.
