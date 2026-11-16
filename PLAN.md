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
      IME commits). v2 ideas: caret positioning, selection, per-glyph editing.
- [x] Export path: E png / J jpg / W webp / G gif -> ~/Downloads/chonk-<ts>.<ext>.
      Encoding + file write run on the render worker; result shown in title.
      File picker dialog still planned.
- [x] Empty captions are pruned (backspacing a caption away deletes the layer).
- [ ] File open dialog (rfd or portal) once DnD verdict is in.
- [ ] Zoom/pan: preview-resolution compositing (downsample when zoomed out).
- [ ] Layer list panel (visibility, reorder, opacity).
- [ ] Spinner/progress overlay for multi-second ops (big GIF import,
      huge exports): compositing worker reports; UI shows indicator.
- [ ] Windows support check (gpui-pre is cross-platform; CI later).

## Tested-out / rejected
- (none yet)

## Open questions
- Selection outline clips at doc edge when layer sits at 0,0 (cosmetic).
- Text hitbox = layout box, not visible glyph bounds (outline shows the box;
  consistent, but worth a tighter bbox later).
