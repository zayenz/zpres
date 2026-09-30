# Adopt SV as a supported Theme

SV has a self-contained package and a visual identity that remains useful
beside Science and Dark Splash. Theme API v1 therefore adopts SV as a supported
built-in Theme.
The package consists of:

- `themes/sv/theme.toml`
- `themes/sv/theme.css.tmpl`
- `themes/sv/print.css.tmpl`

The complete ignored `themes/sv/inspo/` subtree is excluded. This includes the
two local source directories and their ZIP archives. Those source materials are not product dependencies. The SV
manifest does not declare any file from that subtree as a font, asset, style
reference, or inspiration source. Agents must not force-add, copy from, or
publish those files as part of SV work.

SV v1 consumes the renderer-owned `scientific-data` module. Its Theme package
owns the Propagation Workbench identity: condensed sans typography, segmented
trace-bus accents, a sparse technical grid, squared evidence surfaces, and
violet/gold data treatment. The shared module supplies the common technical-content behavior.

SV and Dark Splash provide different treatments of technical content.
