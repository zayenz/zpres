# Keep Chromium for the first conference decks

zpres will keep Chromium as the PDF renderer for the first real conference decks. The current fixture deck exercises the v1 content that matters most: math, code, tables, figures, charts, direct layouts, steps, detail slides, and HTML-only static content. Chromium produces a presentable PDF for that fixture when zpres renders the PDF artifact from the deck model and checks readiness before capture.

This is not a decision to make Chromium the definition of PDF export. Chromium remains behind the `PdfRenderer` boundary. zpres now treats PDF export as a gated pipeline: source diagnostics must be clean, math must render through KaTeX, chart blocks must have a reliable static renderer, local images and data dependencies must resolve, the print HTML must declare `zpres-ready`, and smoke checks must verify file existence, page count, nonblank page content, and unresolved-content markers.

The main trade-off is speed against determinism. Chromium lets zpres share enough visual behavior with the HTML presentation to ship useful decks quickly. A native or Typst-based renderer might be more deterministic later, but it would slow down the first usable version and duplicate theme work before the content model has settled.

The pivot condition remains explicit: if real deck fixtures show repeated clipping, unresolved content, renderer-specific layout collapse, or unacceptable font/chart behavior after the current gates pass, zpres should add a second PDF renderer rather than weakening the PDF quality bar.
