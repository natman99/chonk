# Chonk

A fast, keyboard-first meme & quote-card editor. Rust + GPUI for the shell,
tiny-skia for rasterizing the canvas itself. Built to make captioned images
and quote cards without leaving the home row.

## Getting started

```sh
cargo run --release -p chonk-app            # empty meme to start
cargo run --release -p chonk-app photo.jpg  # open something specific
```

Release mode matters: debug builds are ~100× slower at rasterizing.

There is also a headless engine (`chonk-core`) with CLI tools:
`chonk-quote` (quote cards), `chonk-gif` (caption/effect a GIF), and
`chonk-sample` (composite a document to PNG).

## Opening & saving

| Action | How |
|---|---|
| Open file | drag onto the window, `Ctrl+O`, or command-line argument |
| Quick export | `E` PNG · `J` JPEG · `W` WebP · `G` GIF → `~/Downloads/chonk-<time>.<ext>` |
| Save as… | `Ctrl+S` — choose path & format (by extension) |

Supported inputs: PNG, JPEG, WebP, GIF (animated), SVG, BMP.
GIF exports animate when the document contains an animated layer.

## Editing

| Action | Key |
|---|---|
| Toggle quote-card mode | `Q` |
| Add caption | `T` then **click where you want it** (`Esc` cancels) |
| Copy / paste layer | `Ctrl+C` / `Ctrl+V` (in-app clipboard) |
| Paste files/image/text | `Ctrl+V` with empty in-app clipboard |
| Edit selected caption | `Enter` (finish with `Enter`/`Esc`) |
| Move caret while editing | `←` / `→`, or click in the text |
| Delete selected layer | `Del` / `Backspace` |
| Nudge selected layer | arrow keys (10 px) |
| Selected layer opacity | `]` up · `[` down (5% steps) |
| Selected layer blend mode | `,` prev · `.` next (16 modes) |
| Mark layer as template slot | `S` (or the `[ ]` tag in the panel) |
| Deep-fry selected image layer | `F` (walks mild → medium → heavy → ultra) |
| Undo / redo | `Ctrl+Z` · `Ctrl+Y` / `Ctrl+Shift+Z` |
| Toggle light/dark theme | `Ctrl+T` |
| Play / pause GIF | `Space` |
| Export quick | `E` `J` `W` `G` |

Click a layer to select it (cyan outline). Drag to move it — layers can't
escape the canvas; oversized ones slide but always keep the canvas covered.
An empty caption you backspaced away stays alive while you're editing it
(shown as a placeholder box) and is removed when you finish or click away.

### Selecting stacked layers

- Plain click selects the **topmost** layer under the cursor.
- `Alt+click` steps **down** through the layers at that point (wraps).
- `Tab` / `Shift+Tab` cycle selection through the whole stack.
- The layer panel (right side) always shows the full stack.

### Zoom & pan

- Scroll wheel zooms toward the cursor (0.1×–8×); zoom speed eases off at
  high magnification.
- Drag empty canvas to pan. `=` / `-` zoom at center, `0` resets.
- Zoomed-out canvases composite at display resolution automatically, so
  panning a 4K import at 20% stays smooth.

## Layer panel

Right side, topmost layer first:

- Click a row to select the layer.
- `◉` / `◎` toggle visibility.
- `▲` / `▼` reorder.
- A small bar shows each layer's opacity (`]`/`[` adjust the selected one).
- Blend mode short name (`.`/`,` cycle the selected one).
- `[slotname]` tag marks fillable template slots (`S` toggles).
- **EXTRACT PALETTE** pulls up to 8 dominant colors from the composite;
  click a swatch to recolor the selected caption to match the image.
- `✕` delete.

## Templates & batch filling

A `.chonk` file with slots is a template. Mark text/image slots with `S`,
save with `Ctrl+S`, share it like any file (a git repo of templates works
great — text-only templates are plain JSON and diff cleanly).

Fill from the CLI:

```
chonk-fill template.chonk --list                       # show slots
chonk-fill template.chonk --set top="CHONK MODE" \
    --set avatar=face.png --out meme.png
```

Values that name an existing file fill image slots (scaled to the slot's
box); everything else fills text slots. Loop it over a folder or a CSV
for batch memes.

Fry and repost the whole composite from the CLI:

```
chonk-fill template.chonk --set top="CHONK MODE" \
    --fry heavy --watermark tiktok=@chonk_enjoyer --out fried.png
```

`--fry` runs genuine JPEG round-trips (downscale → compress → blow back
up) plus contrast/saturation blowout and noise — the real artifacts, not
an impression of them. `--watermark` styles a fake platform overlay:
tiktok, instagram, youtube, snapchat, reddit.

## Status bar

Feedback (exports, deep-fries, blend changes, errors) shows in a status
bar along the bottom of the window — never in the window title. With
nothing to report it shows a shortcut cheat sheet.

## Themes

Dark is the default; `Ctrl+T` toggles the light theme.

## Design notes

- The canvas engine (`chonk-core`) never touches GPUI — the same code runs
  headless in the CLI tools and tests.
- All rasterization, encoding, and file dialogs run off the UI thread; the
  window never blocks while a GIF encodes or a save dialog is open.
- Fonts: Anton, Archivo Black, Noto Sans/Serif, and Space Mono are bundled
  (OFL-1.1), so quote cards look the same on every machine.

See `PLAN.md` for the roadmap and `PERFORMANCE.md` for benchmarks.
