# Architectural decisions

These records explain the shipped architecture and its tradeoffs. Use the
[system overview](../current-system.md) and [authoring guides](../README.md)
for current behavior.

| Decision | Subject |
| --- | --- |
| [0001](0001-deck-model-with-peer-output-targets.md) | The Deck model is canonical; HTML and PDF are peer Output targets |
| [0002](0002-start-with-chromium-pdf-rendering.md) | Start with a replaceable Chromium PDF renderer |
| [0003](0003-keep-chromium-for-first-conference-decks.md) | Keep Chromium while requiring static readiness checks |
| [0004](0004-require-browser-rendered-release-gates-for-production-themes.md) | Combine browser checks with human visual review |
| [0005](0005-version-theme-contracts-and-compile-v1-css-offline.md) | Version Theme APIs and compile the v1 CSS foundation offline |
| [0006](0006-publish-html-through-retained-generations.md) | Publish complete HTML generations without replacing sibling files |
| [0007](0007-publish-raster-pages-through-exclusive-directory-exchange.md) | Give each raster page set its own output directory |
| [0008](0008-compile-theme-api-v1-output-from-one-presentation-plan.md) | Share one Presentation plan across live and static output |
| [0009](0009-require-explicit-background-semantics-in-theme-api-v1.md) | Declare whether a background is decoration, context, or evidence |
| [0010](0010-adopt-sv-as-a-supported-theme.md) | Adopt the self-contained SV package |
| [0012](0012-retain-the-distinct-v1-theme-portfolio-and-16-9-contract.md) | Retain five production Themes and the 16:9 contract |
