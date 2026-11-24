# Document Viewer

A document viewer for PDF, DjVu, DVI, TIFF, images and comic book archives, written entirely
in Rust. There is no GTK, Qt, glib, cairo or poppler dependency: PDFs are rendered with
[hayro](https://github.com/LaurenzV/hayro), DjVu with djvu-rs and the interface is
[egui](https://github.com/emilk/egui).

## Install

Arch Linux:

```bash
cd packaging && makepkg -si
```

From source:

```bash
make
sudo make install          # PREFIX=/usr by default
```

## Programs

| Program | Does |
|---|---|
| `document-viewer` | The viewer |
| `document-viewer-thumbnailer` | PNG thumbnails for file managers |

## Formats

PDF (including encrypted PDFs and `.pdf.gz/.bz2/.xz`), DjVu, DVI (PK fonts found with
`kpsewhich`), single and multi-page TIFF, PNG/JPEG/GIF/WebP/BMP and other images, and CBZ/CBT
comic books.

## Configuration

Everything lives in `~/.config/document-viewer/`:

- `theme.toml` — colors for the light and dark palettes, font, font size, corner radius,
  spacing, page gap and more. It is written with the defaults on first start and re-read as
  soon as you save it. *Main menu → Appearance → Edit Theme File…* opens it.
- `config.toml` — default layout (continuous, dual pages, zoom), inverted colors, auto reload,
  render threads and window size.

Per-document state (last page, zoom, rotation, bookmarks) is kept in
`~/.local/state/document-viewer/documents.toml`.

## Keyboard

Press F1 in the viewer for the full list. Highlights: Ctrl+F find, Ctrl+L go to page, F9 side
panel, F5 presentation, Ctrl+wheel zoom, C/D continuous/dual, F/W fit page/width,
Alt+Left/Right history, Ctrl+D bookmark.

## Tests

```bash
make check
```

The UI tests drive the real window headlessly through its accessibility tree.

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).
