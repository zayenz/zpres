# Version Theme contracts and compile the foundation offline

`theme.api_version = 1` selects the renderer and cascade contract shipped in
the first release. Unsupported values fail at their manifest location.

The renderer owns the 1280 by 720 logical field, safe area, semantic regions,
reading order, fit and letterboxing, and Output-target geometry. Themes own
typography, color, surfaces, and ornament within those regions. Screen and
print templates remain in the `zpres-theme` cascade layer; the renderer
reserves reset, foundation, module, and target layers.

Tailwind is a developer-only compiler for the foundation. Its version is
locked and the generated CSS is committed. Building or presenting a Deck
requires no Tailwind runtime, Node process, CDN, or network service.

Every runtime font and asset must be a declared local dependency. Remote,
absolute, and traversal URLs, `@import`, and unsupported URL-bearing functions
fail validation. This makes the resource graph available to HTML bundling and
static export before publication.
