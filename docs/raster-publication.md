# Raster page-set publication

`zpres export talk.zp.md --png dist/talk-png` and
`zpres export talk.zp.md --jpg dist/talk-jpg` publish complete page sets by
atomically replacing the requested directory. The directory is an
exclusive zpres ownership boundary. Do not put a contact sheet, notes, a PDF,
or any other file inside it.

## On-disk layout

A successful PNG export has this shape:

```text
dist/
  .zpres-raster-publish-<target>.lock  persistent target-bound writer lock
  talk-pages/
    .zpres-raster-page-set.json       ownership and generation marker
    page-001.png
    page-002.png
    ...
  talk-contact.png                    separate single-file artifact
```

JPEG uses the same shape with `page-NNN.jpg` in a different directory. The
marker records the format, canonical target identity, generation, page count,
ordered filenames, and SHA-256 page hashes. The lock and temporary sibling
names are implementation details under the reserved `.zpres-*` namespace.
Leave them in place; the next publisher validates and recovers abandoned
entries when it can prove ownership.

The marker is bound to its output path. A copied or moved page directory is
still useful as an ordinary collection of images, but zpres will not replace it
at the new location. Export to a missing or empty destination to establish a
new ownership boundary.

## Ownership and migration

zpres claims only these destinations:

- a missing directory;
- an existing, truly empty regular directory; or
- a complete directory with the canonical target-bound marker for the
  requested format.

Every unmarked non-empty directory is user-owned. This includes a directory
that contains only a perfectly contiguous older `page-001.png` through
`page-NNN.png` set. zpres preserves and refuses it rather than guessing how it
was created.

To migrate an older direct page directory, move the whole directory aside and
export again:

```sh
mv dist/talk-pages dist/talk-pages.previous
zpres export talk.zp.md --png dist/talk-pages
```

Inspect or remove the old directory later. Do not create or edit the ownership
marker by hand. A marker with the wrong target, format, generation, inventory,
or hashes is treated as invalid ownership evidence.

## Publication and failure behavior

Capture scratch lives in the target parent, outside the requested raster
directory, while the host-wide per-user namespace lock is held. The publisher
writes the complete next generation to a unique sibling, validates the marker
and every page hash, syncs the staged files and directory, then uses one native
filesystem operation:

- a no-replace rename when the requested directory is missing;
- a directory exchange when it already contains an owned generation or is
  being claimed from an empty directory.

An error before that operation leaves the requested directory unchanged.
After the operation, the canonical path names the new generation. If zpres
cannot confirm the parent-directory sync or remove the displaced generation,
the export succeeds with a warning and retains the owned sibling for a later
recovery pass. Reporting an error at that point would falsely imply that the
old generation had been restored.

A process crash leaves the canonical directory naming either the old complete
generation or the new complete generation. Recovery first takes the persistent
target lock and stabilizes the parent directory. It deletes only target-bound
stages whose marker and page inventory validate; lookalike user files and
directories remain untouched.

## Readers and contact sheets

Directory exchange makes the canonical name continuously resolve to an old or
new complete directory. It does not give an ordinary multi-open reader a
generation snapshot. For example, a script can open `page-001.png`, cross a
successful rebuild, and then open `page-002.png` from the next generation.

The CLI avoids that race for `--png-contact-sheet`. PNG export returns a private
generation identity, and the report-specific contact-sheet path reacquires the
same target lock, verifies the generation plus ordered hash inventory, and
holds the lock while Chromium reads every page. If another export won the gap,
contact-sheet generation fails instead of silently using the newer page set.

External tools that need the same guarantee must prevent every rebuild for the
full multi-open read or use an API that pins the reported generation. Waiting
for one export process to finish establishes only the starting state; another
export may begin while the reader is opening later pages. Do not infer a
coherent generation from several independent path opens.

## Output layout rules

PNG and JPEG destinations must be separate and disjoint. Neither may equal,
contain, or sit inside the other. The CLI also rejects overlap with the Source
file, Project or Global config, the Theme package and its dependencies, Deck
dependencies, PDF, print HTML, notes, and contact-sheet outputs. Symlink aliases
and portable case/Unicode aliases take part in this comparison.

All cooperating zpres mutating Output-target boundaries serialize through one
host-wide per-user namespace lock. They refuse to create a file, HTML
publication, or nested raster directory inside an owned raster root. Raster
publishers refuse paths inside the marked `zpres-html-generations/` tree. These
rules keep one ownership model from invalidating another, including commands
whose outputs cross filesystem boundaries.

The same boundary protects `zpres init`, `zpres theme init`, and `zpres config
init`. Every `.zpres-*` path component is reserved; do not use one as a Deck,
Theme, config, or Output-target name. zpres also protects the selected HTML root
`index.html` and its private generation metadata from peer exports. Chromium
profiles and browser temporary files use a fixed validated host scratch root,
not a caller-controlled `TMPDIR`, `TMP`, or `TEMP`.

The contact sheet remains a separate single-file artifact outside the PNG and
JPEG roots. PDF, print HTML, notes, and Theme specimen review files also stay
outside. Theme specimens are written as several sequential artifacts, so
the whole specimen directory does not have the same atomic guarantee.

## Platform boundary

Atomic raster publication currently supports macOS and Linux. Both platforms
provide the required OS-released lock, no-replace rename, directory exchange,
and directory sync behavior. Other platforms fail before claiming or mutating
the requested raster directory.

The ownership marker and lock coordinate trusted local zpres processes. They
detect corruption and accidental path reuse; they do not defend against a
hostile process that rewrites files between checks.

The architectural decision is recorded in
[ADR 0007](adr/0007-publish-raster-pages-through-exclusive-directory-exchange.md).
