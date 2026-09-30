# zpres

zpres is a source-driven presentation builder. It grew out of Zayenz's
scientific talks, but supports presentations on any subject. This vocabulary
keeps the code, documentation, and design discussions consistent; start with the
[getting started guide](docs/getting-started.md) for the authoring workflow.

## Language

**Deck**:
A complete presentation described by one source file and its referenced local assets.
_Avoid_: Slide deck, presentation file

**Section**:
A top-level unit of a deck containing one main slide and optional detail slides.
_Avoid_: Horizontal slide group

**Main slide**:
The first slide in a section, used for the primary talk path.
_Avoid_: Horizontal slide

**Detail slide**:
A slide attached to a section that expands, supports, or backs up the section's main slide.
_Avoid_: Vertical slide, backup slide

**Source file**:
The author-written `.zp.md` file that zpres compiles into presentation outputs.
_Avoid_: Markdown file, input document

**Deck root**:
The directory used to resolve a deck's local assets and data dependencies.
_Avoid_: Project root, working directory

**Theme search path**:
A directory where zpres looks for themes, including shared theme directories outside the deck root.
_Avoid_: Theme root

**Project config**:
The `zpres.toml` file that defines shared defaults for one deck directory or talk project.
_Avoid_: App config, workspace config

**Global config**:
The per-user `zpres.toml`, located through `XDG_CONFIG_HOME` when set or the operating system config directory otherwise. It stores machine-local defaults such as shared Theme search paths.
_Avoid_: User project config

**Section separator**:
The `---` marker in a source file that starts a new section and its main slide.
_Avoid_: Horizontal separator

**Detail separator**:
The `--` marker in a source file that starts a detail slide within the current section.
_Avoid_: Vertical separator

**HTML presentation**:
The preferred live-presenting output target, with browser-native navigation, interaction, and animation.
_Avoid_: Web export, HTML export

**HTML generation**:
A complete retained publication of one HTML presentation, including its renderer document, runtime files, Theme dependencies, and Deck dependencies.
_Avoid_: Cache directory, partial bundle

**Raster page set**:
A complete ordered set of PNG or JPEG pages derived from one executed static Deck and published in an exclusive directory.
_Avoid_: Image folder, mixed export directory

**Raster generation**:
One complete Raster page set with an ownership marker tied to its output path. Publication replaces the whole directory in one atomic operation.
_Avoid_: Batch, cache entry

The ownership, migration, and reader contract for these terms is documented in [docs/raster-publication.md](docs/raster-publication.md).

**PDF export**:
The paginated output target used for submission, sharing, archival use, and presenting in environments where PDF is the only acceptable format.
_Avoid_: Print version

**Theme**:
A local package that controls the visual and layout treatment of a deck without changing the deck's source content.
_Avoid_: Template, skin, style

**Theme parameter**:
A declared input accepted by a theme to adjust its behavior without editing the theme package.
_Avoid_: Theme variable, option

**Theme override**:
An explicit theme-provided replacement for a default rendering hook, used when styling controls are not enough.
_Avoid_: Monkey patch, custom renderer

**Debug theme**:
A non-presentational theme that exposes deck structure, layout boundaries, diagnostics, and output-target behavior.
_Avoid_: Plain theme

**Slide variant**:
A semantic label for the intent of a slide, such as claim, figure, comparison, derivation, or dense.
_Avoid_: Slide type, template

**Layout directive**:
A constrained source-file directive that describes slide structure, such as columns, grid, stack, overlay, or aside.
_Avoid_: CSS, manual positioning

**Layout vocabulary**:
The names and values used by layout directives to describe spacing, sizing, alignment, and structure.
_Avoid_: CSS framework

**Layout value**:
A parsed value used by a layout directive, either from the standard zpres scale or as an explicit arbitrary value.
_Avoid_: Raw CSS string

**Step**:
An ordered semantic progression within a slide, such as revealing a point, changing emphasis, or advancing an explanation.
_Avoid_: Animation, fragment

**Output target**:
A concrete representation produced from the deck model, such as an HTML presentation or PDF export.
_Avoid_: Backend, renderer

**Content block**:
A typed semantic unit in the deck model, such as a paragraph, list, equation, figure, table, code block, chart, layout container, or HTML-only block.
_Avoid_: HTML snippet, Markdown chunk

**Chart block**:
A typed content block containing a declarative Vega-Lite JSON chart specification plus its data dependencies.
_Avoid_: Plot image

**Data dependency**:
A local structured data file referenced by a content block, such as CSV or JSON used by a chart.
_Avoid_: Generated data source

**Math block**:
A typed content block containing LaTeX-style math that can be rendered by KaTeX.
_Avoid_: Formula snippet

**Diagnostic**:
A message tied to source content that reports an error, warning, or authoring note.
_Avoid_: Log message

**Last good deck**:
The most recent successfully built deck served by the live server while the current source file has errors.
_Avoid_: Cached build

**HTML-only content**:
An explicit source-file block whose contents are interpreted as HTML rather than as portable zpres deck content.
_Avoid_: Raw HTML

## Relationships

- A **Deck** is defined by exactly one `.zp.md` **Source file**.
- A **Deck root** is the directory containing the **Source file**.
- A **Project config** may define defaults such as theme search paths, output directories, and default export settings.
- A **Global config** may define machine-local defaults such as shared theme search paths.
- Source front matter overrides **Project config**, which overrides **Global config**.
- A **Deck** contains a linear sequence of **Sections**.
- A **Section** contains exactly one **Main slide** and may contain many **Detail slides**.
- A **Section separator** starts a new **Section** and its **Main slide**.
- A **Detail separator** starts a **Detail slide** in the current **Section**.
- A **Deck** may reference many local assets.
- A **Deck** can produce multiple peer **Output targets**.
- An **HTML presentation** and a **PDF export** are peer **Output targets**.
- A published **HTML presentation** selects exactly one complete **HTML generation** through its root entrypoint.
- An **HTML generation** does not own peer files in the surrounding Output-target directory.
- A **Raster page set** is a derived artifact, not a peer **Output target** beside the **HTML presentation** and **PDF export**.
- A **Raster generation** exclusively owns its requested directory and contains only its marker plus one contiguous page inventory.
- PNG and JPEG **Raster page sets** use separate disjoint directories; a contact sheet is a separate file outside both.
- A missing or truly empty directory may become a **Raster generation**. A non-empty unmarked directory remains user-owned even when its files look like an older zpres page set.
- Cooperating zpres output mutations use one host-wide per-user namespace lock and never nest another output boundary inside an owned **Raster generation**.
- An **HTML presentation** is the preferred output target for live presenting when the environment permits it.
- A **PDF export** must be good enough to present from, not only good enough to archive.
- A **PDF export** linearizes each **Section** as the **Main slide** followed by its **Detail slides**.
- A **PDF export** may collapse interaction and animation into static states, but the resulting deck must remain coherent.
- A **Step** becomes an interactive progression state in an **HTML presentation**.
- A **PDF export** defaults to the final coherent state of each slide, with an option to export one page per **Step** when the progression is pedagogically important.
- A **Theme** changes how a **Deck** is presented, not what the **Deck** says.
- A **Theme** should normally style containers, captions, colors, typography, and layout treatment.
- A **Theme override** may make deeper rendering changes, but the default path should not require one.
- The built-in **Debug theme** exposes the renderer contract independently of production Theme styling.
- Themes may be loaded from the **Deck root** or from configured **Theme search paths**.
- **Theme search paths** may come from **Global config**, **Project config**, or CLI flags.
- A **Theme** declares the **Theme parameters** it accepts.
- A **Source file**, **Project config**, or CLI flag may provide values for declared **Theme parameters**.
- zpres diagnoses unknown, missing, or invalid **Theme parameters**.
- CLI flags override source front matter, including **Theme parameters**.
- **HTML-only content** must be explicit in the **Source file**.
- **HTML-only content** may be rendered into the **PDF export** through an HTML-capable renderer such as Chromium, rather than being silently dropped.
- A **Source file** is parsed into typed **Content blocks** before rendering to any **Output target**.
- A **Content block** preserves semantic meaning rather than pre-rendered HTML.
- A **Chart block** is a supported first-class content type.
- A **Chart block** may use inline data or local **Data dependencies**.
- **Chart block** specifications use Vega-Lite JSON.
- A **Math block** uses KaTeX-compatible LaTeX.
- A **Math block** must be resolved before PDF capture.
- zpres does not execute Python, R, Julia, or other authoring scripts during a build.
- A **Diagnostic** may be non-fatal during live authoring but fatal for `build` or `export`.
- The live server keeps serving the **Last good deck** when the current **Source file** cannot be built.
- A **PDF export** should fail when a **Content block** has no reliable PDF representation, unless the author explicitly marks an acceptable omission or degradation.
- A **Slide variant** communicates slide intent to a **Theme**.
- A **Layout directive** communicates slide structure to a **Theme**.
- A **Layout directive** may describe spatial relationships, but normal **Source file** content should not use arbitrary CSS positioning.
- A **Theme** must preserve the structural intent of a **Layout directive**, while controlling visual treatment.
- A **Layout vocabulary** may take naming inspiration from Tailwind CSS when there is an obvious equivalent, but zpres does not use Tailwind as a runtime dependency.
- A **Layout value** should use the standard zpres scale when possible, with arbitrary values available for precise slide layouts.

## Design boundaries

The Deck model is canonical. The HTML presentation is preferred for live
presenting, but PDF export must also be presentable when a venue requires it.
Steps default to a final coherent static state; authors can request one page
per Step when the progression is part of the explanation.

A Slide variant describes intent, such as a claim or comparison. A Layout
directive describes a spatial relationship, such as two columns. Themes must
preserve that relationship while choosing its visual treatment.

Themes declare their parameters and local dependencies. Machine-specific
search paths belong in Global config; talk-specific defaults belong in
Project config. Source front matter and CLI flags supply more specific choices.

Chart blocks, external figures, and Math blocks serve different authoring
needs. zpres validates their supported representations before static export;
it does not run authoring scripts. The live server retains the Last good deck
while the current Source file has errors.

A Raster page set owns its whole output directory. Contact sheets, PDFs, and
notes stay outside it. HTML publication owns only its marked generations and
root entrypoint, leaving sibling files alone.

Feature selection follows concrete presentation needs. The project is intended
for public use across subjects, with documented contracts and checks appropriate
to its supported features.
