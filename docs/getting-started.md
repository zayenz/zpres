# Getting started

A zpres Deck starts as one `.zp.md` Source file. You write slide content in
Markdown, add directives for presentation features, and let a Theme arrange it
for the browser and PDF export. This guide takes you from that Source file to
a presentation you can rehearse and share.

Install zpres using the [README instructions](../README.md#install). Chrome or
Chromium is needed for `check`, PDF export, and page images. Writing a Source
file, building HTML, and serving a live preview do not require it.

## Create and preview a Deck

```sh
zpres init talk.zp.md --title "Search order" --author "Ada"
zpres serve talk.zp.md
```

Open the local URL printed by `serve` and edit `talk.zp.md` in your editor.
The preview reloads successful builds and preserves your presentation position
where possible. If an edit prevents a rebuild, it keeps the Last good deck
visible and reports the problem.

`init` creates an example you can edit. Without a configured Theme, zpres uses
Debug. Set `theme: science` in front matter for the serif-led treatment used
below. The following smaller Source shows the
basic structure; you can use it as the contents of `talk.zp.md`:

```markdown
---
title: "Search order"
author: "Ada"
theme: "science"
aspect: "16:9"
---

# Search order changes the work

We compare two ways to choose the next variable.

^ Introduce the example before showing the result.

---

# Choose the most constrained variable first

::: steps
1. Inspect the remaining domains.
2. Choose the smallest domain.
3. Branch on one value.
:::

--

## Detail: ties need a stable rule

When domains have equal size, use the declared variable order.
```

The first `---` pair encloses YAML front matter: Deck metadata and defaults.
After front matter, `---` starts a new Section and its Main slide. `--` starts
a Detail slide in the same Section. A heading supplies the slide title; a
heading alone does not start a new slide.

This Source has two Sections and three slides. Its Main path introduces the
question and explains the rule. The Detail slide supplies a qualification that
the speaker can visit when needed.

Keep assets beside the Source file, for example `figures/runtime.svg` or
`data/runtime.csv`. Local asset paths resolve from the Source file's directory,
called the Deck root.

## Navigate and rehearse

| Key | Action |
| --- | --- |
| Right arrow, Page Down, or Space | Reveal the next Step, then move to the next Section |
| Left arrow or Page Up | Return to the previous Step, then the previous Section |
| Down / Up arrow | Move through a Section's Detail slides and back to its Main slide |
| N | Show or hide speaker notes |

Speaker notes are hidden from the slide, but included in live HTML by default.
They are available to anyone who receives that HTML. Build with
`--exclude-speaker-notes` when the shared copy should omit them.

PDF export puts each Main slide before its Detail slides. Steps normally
collapse into one final state. Use `::: steps pdf="pages"` when each stage
should become a separate page. See [Steps](text-authoring.md#steps) for an
example.

## Choose a Theme

Science is a restrained Theme with serif headings and light and dark palettes.
Try another built-in Theme without editing the Source:

```sh
zpres serve talk.zp.md --theme paper-chalk
zpres serve talk.zp.md --theme science --theme-param mode=dark
```

Stop the first server before starting the next, or use the live switcher:

```sh
zpres serve talk.zp.md --theme-switcher
```

To save a choice, set `theme` and `theme_params` in front matter:

```yaml
theme: science
theme_params:
  mode: dark
```

Theme parameters belong to the selected Theme. For example, Science uses
`mode` for its palette, while Wedding uses `variant`. See the
[Theme inventory](theme-inventory.md) for the available Themes. Built-in Themes
use a 16:9 canvas; other aspect ratios are not supported by Theme API v1.

## Check and export

```sh
zpres check talk.zp.md --strict
zpres build talk.zp.md --out dist/talk
zpres export talk.zp.md --pdf dist/talk.pdf
```

`check` parses the Source, resolves dependencies, and uses Chromium to check
static rendering without publishing output. `--strict` also makes Source
warnings fatal. A passing check does not judge whether the argument is clear
or the type is readable in your room; inspect and rehearse the result.

Open `dist/talk/index.html` for the HTML presentation. Copy the whole
`dist/talk/` directory to share or host it. The root entrypoint selects a
complete version under `zpres-html-generations/`, so `index.html` alone is
not a portable presentation. Local assets are bundled; remote images and
embeds still need a network connection for live use.

For a public copy without speaker notes, use a fresh output directory.
Reusing an existing directory retains older builds, which may contain notes:

```sh
zpres build talk.zp.md --out dist/public-talk --exclude-speaker-notes
```

PDF export omits speaker notes by default. Other useful exports are:

```sh
zpres export talk.zp.md --png dist/talk-png \
  --png-contact-sheet dist/talk-contact.png
zpres export talk.zp.md --jpg dist/talk-jpg --image-size 1920x1080
zpres export talk.zp.md --print-html dist/talk-print.html
zpres export talk.zp.md --pdf dist/rehearsal.pdf --notes
zpres export talk.zp.md --notes-txt dist/speaker-notes.txt
```

PNG and JPEG destinations each belong exclusively to that page set. Keep
PDFs, notes, and contact sheets outside those directories. zpres refuses to
replace a non-empty directory it does not own. The
[publication guides](README.md#understand-the-implementation) explain rebuilds
and output ownership in more detail.

## Share defaults between talks

Configuration is optional. Put a `zpres.toml` in the Deck directory or an
ancestor directory when several Sources should share defaults:

```toml
schema_version = 1

[deck]
theme = "science"

[theme.params]
mode = "light"

[paths]
theme_dirs = ["themes"]
```

For scalar settings and individual Theme parameters, CLI flags override Source
front matter, which overrides Project config, which overrides Global config.
Theme search paths accumulate across these layers. Paths in a config file are
relative to that file; Source paths and CLI output paths are relative to the
Deck root. Selecting a different Theme with `--theme` clears inherited Theme
parameters before applying explicit `--theme-param` values.

Inspect the effective settings with:

```sh
zpres config show talk.zp.md
zpres config path
```

`config path` prints the machine's Global config location. `zpres config init`
creates that file if you want user-wide defaults. When set, `XDG_CONFIG_HOME`
selects its base directory; otherwise zpres uses the operating system's config
directory.

## Add charts, images, and Layouts

Use [charts and math](scientific-content.md) for equations, data, and code;
[figures](figure-authoring.md) for plots produced by other tools; and
[text and Layouts](text-authoring.md) for comparisons, derivations, and other
slide structures. The [complete examples](../examples/README.md) show how
these pieces fit into a talk.
