# Publish raster pages through exclusive directory exchange

PNG and JPEG export writes several files, but a filesystem cannot atomically
replace a selected `page-NNN` subset while preserving arbitrary siblings. A
file-by-file publisher can recover from an ordinary error, yet a process crash
or concurrent lookup can still find a missing or mixed set.

Each requested raster directory is therefore one exclusive zpres ownership
boundary and contains one complete raster generation. zpres stages the marker
and every page in a unique sibling directory, validates and syncs the complete
set, then commits with a native no-replace rename or directory exchange. PNG
and JPEG use separate, disjoint directories. A contact sheet is a separate
file outside both ownership boundaries.

The canonical marker records the raster format, target identity, generation,
page count, ordered filenames, and SHA-256 hash of every page. The target
identity binds the marker to its canonical destination. Moving or copying a
marked directory does not transfer ownership to the new path.

zpres claims a missing directory or a truly empty regular directory. It
refuses every unmarked non-empty directory, including a contiguous page-only
directory written by an older zpres version. It also refuses corrupt markers,
unexpected files, missing pages, hash mismatches, symlinks, nested directories,
and a marker for the other raster format. Migration requires moving or removing
the old directory before exporting again; filename patterns never establish
ownership.

Cooperating zpres mutations serialize through one host-wide per-user namespace
lock before inspecting or changing output ownership. This includes Output
targets and Deck, Theme, and Global-config initialization. It avoids lock-order
problems when one command writes targets on different filesystems. Mutators
refuse either ownership model's private tree, protect the HTML root entrypoint,
and reserve every `.zpres-*` path component for publication metadata. Raster
publication likewise refuses an HTML-managed generation tree. Recovery removes
only entries that carry valid target-bound evidence.

Chromium profiles and the child browser's temporary-file environment use a
validated fixed host scratch root. Process-controlled `TMPDIR`, `TMP`, or
`TEMP` values therefore cannot redirect browser scratch into an owned page set.

The native directory operation is the commit point. Before it succeeds, a
returned error leaves the requested raster directory unchanged and ordinary
error cleanup removes the private stage. After it succeeds, zpres reports the
new generation even if a parent-directory durability sync fails; it warns and
retains the displaced owned directory for later recovery instead of claiming
that the old generation was restored.

Directory exchange makes each lookup of the canonical directory select the
old or new complete directory. It does not pin several pathname opens to one
generation. A reader that opens `page-001.png`, crosses a rebuild, and then
opens `page-002.png` may straddle generations. The report-specific contact
sheet path holds the raster lock and verifies the marker generation and page
hash inventory while Chromium reads the pages.

The first implementation uses `flock` plus native no-replace and exchange
operations on macOS and Linux. Other operating systems fail before claiming or
mutating the requested directory. The locks and checksums coordinate and
identify trusted local zpres output; they are not a security boundary against
a hostile local process.

This decision replaces the provisional unrelated-file preservation behavior
from the release slice. The simpler on-disk page paths remain, but their parent
directory is no longer a shared output folder. Theme specimen roots remain a
separate multi-artifact transaction.
