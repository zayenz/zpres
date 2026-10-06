# Changelog

## 0.2.0 - 2026-10-06

- Add a separate live presenter view with current and next slide previews,
  speaker notes, navigation controls, a timer, and synchronized presentation state.
- Enable the presenter view by default in `serve`; support `--speaker-port`,
  `--no-speaker`, and `--open`.
- Keep slide geometry stable across browser and presenter-preview sizes;
  fit mobile viewports and support touch navigation.
- Preserve authored slide IDs across live rebuilds and isolate authenticated
  presenter previews from interactive media and HTML-only content.

## 0.1.0 - 2026-09-30

- Parse `.zp.md` Source files into typed Decks with Sections, Main slides,
  Detail slides, Steps, speaker notes, and source-located diagnostics.
- Build interactive HTML presentations with bundled local assets. Serve live
  previews that reload successful edits and retain the Last good deck after errors.
- Export browser-validated PDF, PNG, JPEG, and contact sheets. Export print
  HTML and speaker-note text without requiring a browser.
- Render math, code, tables, figures, galleries, charts, diagrams, media, and
  semantic Layouts.
- Ship twelve embedded Themes using the single Theme API 1 contract.
- Publish HTML generations and raster page sets through owned atomic output
  boundaries on macOS and Linux.
