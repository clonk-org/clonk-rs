# Bundled FreeType notice

Clonk Rust builds `freetype-rs 0.38.0` with its `bundled` feature. The
corresponding `freetype-sys 0.23.0` crate statically compiles the FreeType
sources found at `freetype2/` and selects the FreeType License for this
distribution. `FTL.TXT` is copied verbatim from
`freetype2/docs/FTL.TXT`.

The bundled build also statically compiles libpng; its separate notice is in
`../libpng/LICENSE`. FreeType's build uses the platform zlib on the current
macOS and Linux release paths. Separately, the Rust graph contains
`libz-sys`, which may compile vendored zlib on Windows or fallback/static
builds; that notice is in `../zlib/LICENSE`.
