# Chonk — plan & status

Living file. Implemented = verified working (tests or screenshot). In progress /
Planned / Tested-out come below. Perf numbers live in PERFORMANCE.md.

## Architecture principles (agreed)
- Engine (`chonk-core`) is headless: no GPUI types anywhere in it. Canvas
  document data is fully separate from the gpui views. ✅ (enforced by crate split)
- Never put expensive work on the UI thread: rasterization + compositing run
  on a worker thread; UI thread only uploads the finished frame.
- Cache aggressively: rendered text layers ✅, composited background ✅,
  glyph rasterization (inside text cache) ✅.
- Only redraw dirty regions: drag frames ✅ (render_region + overlay);
  extend to all ops.
- Downsample preview when zoomed out: don't rasterize pixels the window
  can't show.
- Bundle permissively-licensed fonts with the app (OFL), don't rely on
  system fonts.
- Show a spinner/progress for multi-second operations.

## Done
- Workspace: chonk-core (engine) + chonk-app (GPUI). tiny-skia rasterizer,
  swash/cosmic-text text, resvg SVG, image codecs (png/jpg/webp/gif).
- Quote-card generation (QuoteSpec + auto-fit binary search) + Q toggle.
- GIF: animated layers, per-frame filters, captioning, chonk-gif CLI.
- App: click select (hit-test), drag, arrows nudge, T add caption, Del delete,
  space GIF playback, file arg loads png/jpg/webp/gif/svg.
- Text render cache + background cache (PERFORMANCE.md has the table).
- Frame-coalesced compositing + dirty-region drag frames (fixes drag lag).
- Stress bench (chonk-stress): 1080p/10 layers 17ms full, playback 0.54ms,
  4K/10 layers 96ms full — see PERFORMANCE.md.

## In progress
- [x] Background-thread compositing: worker thread owns Renderer + canvas
      cache; UI sends RenderJobs (coalesced, seq'd), result pump applies
      finished frames. UI thread no longer rasterizes at all.
- [x] Bundled fonts: Anton, Archivo Black, Noto Sans (R/B), Noto Serif (R/B),
      Space Mono (R/B) — all OFL-1.1, embedded via include_bytes!, loaded
      before system fonts (fonts/LICENSES.md).

## Early experiments
- [x] File drag-and-drop on Wayland: gpui-pre-linux implements it
      (Entered/Pending/Submit -> internal drag carrying ExternalPaths);
      app receives via div.on_drop::<ExternalPaths>. Needs live-user test.

## Planned
- [x] In-app text editing (v1): Enter edits the selected caption live on the
      canvas (EntityInputHandler + chonk-edit key context; append/backspace,
      IME commits).
- [x] Caret v2: caret_pos byte offset; ←/→ move it, click-in-text positions it
      via cosmic-text Buffer::hit + LayoutRun::cursor_position (wrapped text
      and fit_box handled correctly — geometry comes from the real layout,
      RenderedText carries block padding offset). Insert/backspace honor it.
      T creates an EMPTY caption and enters edit mode immediately.
- [ ] Caret v3: text selection (shift+arrows, drag-select, replace selection).
- [x] Export path: E png / J jpg / W webp / G gif -> ~/Downloads/chonk-<ts>.<ext>.
      Encoding + file write run on the render worker; result shown in title.
      File picker dialog still planned.
- [x] Empty captions are pruned (backspacing a caption away deletes the layer).
- [x] Zoom/pan: scroll-wheel zoom anchored at cursor, drag empty canvas to pan,
      =/- keyboard zoom, 0 resets. Pan/zoom are GPU-only (no recomposite).
- [x] Downsampled preview: zoomed-out canvases composite at display
      resolution (Renderer::composite_scaled; bg cached at preview size,
      layer blits carry the global scale). Exports always force full res.
- [x] File dialogs: Ctrl+O open / Ctrl+S save-as via rfd (xdg-portal only,
      no GTK). Picker runs on a background thread; save-as picks format by
      extension and rides the worker export path.
- [x] Layer panel (dark): right sidebar, topmost first — click row selects,
      ◉ toggles visibility, ▲▼ reorder, ✕ delete.
- [x] Canvas frame: 1px theme-border quad outline around the canvas rect.
- [x] Theme palette: all UI colors flow through `Theme`/`DARK` consts.
- [x] Oversized imports stay movable: clamp range flips to [docw-w, 0] when
      the layer is larger than the canvas (canvas stays covered).
- [x] Light theme: LIGHT palette + Ctrl+T toggle (all chrome colors theme-sourced).
- [x] User-facing docs: README.md (controls, features, design notes) — keep in
      sync with every feature landing.
- [x] Layer opacity: `]`/`[` adjust selected layer (5% steps, clamp .05..1),
      bar per row in the panel.
- [ ] Layer panel v2: drag-to-reorder, slider-drag opacity.
- [x] Busy badge: accent square in canvas corner while the worker encodes an
      export (set on dispatch, cleared on worker Status; handlers notify).
- [ ] Windows support check (gpui-pre is cross-platform; CI later).

## Tested-out / rejected
- (none yet)

## Open questions
- Selection outline clips at doc edge when layer sits at 0,0 (cosmetic).
- Text hitbox = layout box, not visible glyph bounds (outline shows the box;
  consistent, but worth a tighter bbox later).
