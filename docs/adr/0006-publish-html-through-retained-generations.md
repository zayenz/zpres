# Publish HTML through retained generations

The requested HTML output directory is a shared Output-target root, not an
owned bundle. It may also contain a PDF export, speaker notes, visual reports,
and user files. zpres will therefore not stage a copy of that directory and
swap it wholesale: copying arbitrary siblings can lose concurrent edits,
permissions, extended attributes, hard-link identity, or sparse-file state.

Each HTML build instead creates a complete generation under the marked
`zpres-html-generations/` subtree. The generation contains immutable renderer
HTML, runtime CSS and JavaScript, Theme dependencies, and Deck dependencies.
After validation and durability sync, zpres atomically replaces only the root
`index.html` with a canonical checksummed pointer to the new generation. Old
generations remain coherent for readers that selected them before the commit.

When a later generation commits durably, the superseded generation's live
entrypoint redirects through the current root while its immutable renderer HTML
and assets remain. This makes browser refresh select the current build without
mixing generations. Retention keeps the current generation, 15 predecessors,
and every generation younger than one hour. A path outside both protections may
be pruned, so only the root entrypoint is a stable bookmark.

The surrounding output root remains unowned. An existing root index must be a
canonical checksummed zpres pointer; an arbitrary index is refused. Scratch
recovery acts only inside marked ownership boundaries or on exact complete
marker evidence. A filename pattern is never sufficient evidence for deletion.

The root index is an entrypoint. Renderer HTML and assets live in immutable
generations. Tools follow the entrypoint or use the resolver API. A deployment
copies the root entrypoint and generation subtree together.

The first implementation uses native atomic replacement and OS-released locks
on macOS and Linux. Other operating systems fail before publication until
equivalent native behavior is available. The lock coordinates zpres writers;
the checksum is ownership/corruption evidence rather than authentication, and
the design does not claim safety against a hostile local writer.

Raster page directories use the separate exclusive-ownership decision in
[ADR 0007](0007-publish-raster-pages-through-exclusive-directory-exchange.md).
HTML output cannot sit inside an owned raster directory, and raster publication
cannot claim the marked HTML generation tree. One host-wide per-user namespace
lock serializes these cooperating output mutations before either ownership
model changes the filesystem.
