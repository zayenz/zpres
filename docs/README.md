# Documentation

Start with [getting started](getting-started.md) to write, preview, and export
one Deck. A Deck is one presentation, defined by a `.zp.md` Source file and its
local assets. Main slides form the talk path; Detail slides hold supporting
material. A Theme controls the visual treatment of both HTML and PDF output.

## Write a talk

| Guide | Use it for |
| --- | --- |
| [Getting started](getting-started.md) | Source structure, navigation, Themes, configuration, and publishing |
| [Text and Layouts](text-authoring.md) | Lists, Steps, speaker notes, slide metadata, and spatial relationships |
| [Charts and math](scientific-content.md) | Equations, local chart data, code, and static-rendering limits |
| [Figures](figure-authoring.md) | Images, captions, galleries, sizing, and static fallbacks |
| [Backgrounds](background-authoring.md) | Slide and Deck images, split backgrounds, and image meaning |
| [Diagrams](diagram-authoring.md) | The supported Mermaid flowchart syntax |
| [Media](media-authoring.md) | Video, audio, embeds, and their static representations |
| [Example Decks](../examples/README.md) | Complete Sources you can run from a checkout |

## Choose or build a Theme

The [Theme inventory](theme-inventory.md) describes the built-in choices.
[Theme authoring](theme-authoring.md) explains how to create a package and check
it against a specimen Deck. Use the [design contract](presentation-design.md)
for composition and review criteria.

[Science](science-theme-v1.md) shows how a production Theme uses the shared
renderer. [Debug](debug-theme.md) shows the layout regions and overflow checks.

## Understand the implementation

The [system overview](current-system.md) maps the source code and processing
steps. [CONTEXT.md](../CONTEXT.md) defines the vocabulary used in the code and
documentation. The [architectural decisions](adr/README.md) record why the
system has its current boundaries.

The [HTML publication](html-publication.md) and
[raster publication](raster-publication.md) guides describe output ownership,
rebuilds, and failure behavior. Read them when integrating zpres with another
tool or managing repeated exports.
