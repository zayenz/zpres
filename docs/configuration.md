# Configuration

zpres reads the per-user `zpres.toml`, a project `zpres.toml` found from the Deck
root, and Source-file front matter. Front matter overrides project defaults;
CLI options override front matter. Theme search paths accumulate, with the
highest-precedence paths searched first.

Locate and initialize the per-user file with:

```sh
zpres config path
zpres config init
zpres config show talk.zp.md
```

A project config can set the Theme, output directory, search paths, Theme
parameters, and default PDF path:

```toml
schema_version = 1

[deck]
theme = "science"
output_dir = "dist/talk"

[paths]
theme_dirs = ["themes"]

[pdf]
renderer = "chromium"
path = "dist/talk.pdf"

[theme.params]
mode = "light"
```

Paths in TOML are relative to the file containing them. Source-file paths and
CLI export paths are relative to the Deck root. `renderer` is optional;
Chromium is the supported PDF renderer.

Room profiles apply to browser-rendered Theme review:

```sh
zpres theme check themes/science --visual \
  --write-specimen dist/science-review --room-profile auditorium.toml
```

The profile path is relative to the current directory. Provisional profiles
report measurements; approved profiles can enforce their declared v1 floors.
Ordinary `check`, `build`, `serve`, `export`, and `config show` do not accept
`--room-profile`.

Before the public release, some settings were accepted without affecting
output. They now fail with a diagnostic: remove `paths.output_root` and
`paths.cache_root`, use `deck.output_dir` for output defaults, and use the
Theme review command for `room_profile`. Only `pdf.renderer = "chromium"` is
accepted. The generated per-user config contains only implemented settings.
