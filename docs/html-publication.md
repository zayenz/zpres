# HTML publication

Build a presentation with `zpres build talk.zp.md --out dist/talk`, then open
`dist/talk/index.html`. To share or host it, copy that entrypoint and the whole
`zpres-html-generations/` subtree together. Use the root entrypoint for links.

Each build writes a complete version, called an HTML generation, before making
it current. Other files in the output directory are left alone. The sections
below explain that layout, how rebuilds affect open tabs, and which files
zpres owns.

HTML builds include speaker notes by default. For a copy without them, use
`--exclude-speaker-notes` and a fresh output directory. That flag omits notes
from the new generation; it does not remove them from older retained builds.

## On-disk layout

```text
dist/talk/
  index.html                         current entrypoint
  deck.pdf                          optional peer file, untouched by build
  speaker-notes.txt                 optional peer file, untouched by build
  zpres-html-generations/
    .zpres-output.json              ownership marker
    g-.../
      .zpres-generation.json        completion marker
      index.html                    live entrypoint for this generation
      presentation.html             immutable renderer HTML
      assets/
        theme.css
        reveal.js
        ...
```

A build first writes and validates a new generation in the marked zpres-owned
subtree. It then commits the root `index.html`, a dependency-free checksummed
pointer, in one atomic filesystem operation. A reader therefore selects either
the previous complete generation or the new complete generation. Theme and
Deck dependencies are generation-qualified, so a browser cannot combine new
HTML with stale CSS or media.

The checksum is ownership/corruption evidence, not authentication. One
host-wide per-user namespace lock serializes cooperating zpres output
mutations, including commands that touch different filesystems. The HTML
publisher also refuses an output root inside an owned raster directory. The
output root and Theme are trusted local inputs; this is not a defence against a
hostile process rewriting files between filesystem operations.

## Rebuild and refresh

After the new root pointer commits durably, older generation `index.html`
files become small redirects back through the current root entrypoint. Their
immutable `presentation.html` and assets remain available to a tab that has
already loaded them. Refreshing such a tab follows the old redirect and then
opens the new generation while preserving its query string and route hash.

zpres retains the current generation, the 15 previous generations, and any
generation younger than one hour. This bounds long-term accumulation while
giving an active reader a grace period for lazy images and media. Once an old
generation is outside both limits it may be removed; bookmarks and deployment
links should always use the root `index.html`, never a `g-*` path.

Failed population leaves the current pointer unchanged and removes owned
scratch data on ordinary error paths. The next successful build recovers
positively owned crash remnants and unreferenced completed generations. It
never scavenges a root sibling by a filename pattern. If directory durability
cannot be confirmed after the pointer operation, the build reports a warning
and retains the rollback entry instead of retiring or pruning generations.

## Output ownership

The `zpres-html-generations` subtree is reserved once its exact ownership
marker exists. Do not place user files in it. The surrounding output directory
remains shared: PDFs, notes, reports, arbitrary directories, and other assets
are preserved byte-for-byte.

zpres replaces an existing `index.html` only when it is a canonical checksummed
zpres pointer. An arbitrary site, symlink, directory, or unknown file at that
path is refused without replacement.

Tools should open the root entrypoint or use
`zpres::html::current_html_bundle_publication` to resolve the immutable current
renderer document. A deployment must copy the root `index.html` and the
`zpres-html-generations/` subtree together. The entrypoint has an ordinary meta
refresh fallback for environments that block its inline route-preserving
script.

## Platform boundary

The crash-safe pointer commit and OS-released writer lock are currently
implemented for macOS and Linux. On other operating systems HTML publication
fails before creating the output root. Supporting another platform requires
equivalent native filesystem operations and recovery tests.

PNG and JPEG page sets use a different ownership model. Each requested page
directory is exclusive and commits as one whole-directory generation; raster
publication refuses the marked HTML generation tree. The two models cannot
nest. See [the raster publication guide](raster-publication.md) and
[ADR 0007](adr/0007-publish-raster-pages-through-exclusive-directory-exchange.md).
