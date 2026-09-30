# zpres

zpres builds HTML presentations and PDF exports from Markdown-based `.zp.md`
Source files. It also exports PNG and JPEG page sets and speaker notes.

## Install and run

Requires Rust 1.88 or newer on macOS or Linux. Chrome or Chromium is needed
for checks and PDF or raster exports.

```sh
cargo install --path . --locked
zpres init talk.zp.md --title "My talk"
zpres serve talk.zp.md
zpres check talk.zp.md --strict
zpres build talk.zp.md --out dist/talk
zpres export talk.zp.md --pdf dist/talk.pdf
```

Run `zpres --help` for commands and `cargo test` for the regression suite.
Licensed under MIT OR Apache-2.0.
