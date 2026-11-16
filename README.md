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
| Add caption | `T` (starts editing immediately) |
| Edit selected caption | `Enter` (finish with `Enter`/`Esc`) |
| Move caret while editing | `←` / `→`, or click in the text |
| Delete selected layer | `Del` / `Backspace` |
| Nudge selected layer | arrow keys (10 px) |
| Selected layer opacity | `]` up · `[` down (5% steps) |
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
- `✕` delete.

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
