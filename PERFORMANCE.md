# Chonk performance notes

## Compositing breakdown (chonk-time bin, 1080×1080 doc)

Measured 2026-11-15 after fixing the >1s quote-toggle report. The report was
a **debug-build artifact**: tiny-skia/imageproc/cosmic-text are 10-50x slower
unoptimized. Use `cargo run --release` for real use.

| path | debug before | debug after | release before | release after |
|---|---|---|---|---|
| toggle to quote (build doc + composite, warm) | ~1.5-2.7 s | ~190 ms | ~25 ms | ~13 ms |
| composite meme doc, warm | 2.30 s | 77 ms | 42 ms | 1.9 ms |
| composite quote doc, warm | 359 ms | 137 ms | 13 ms | 4.9 ms |
| text render, cache hit | — | 10 µs | — | 16 µs |
| composite bg-only 1080² | 5 ms | 6 ms | 2.3 ms | 2.3 ms |

## What was fixed
1. **Text render cache** (`TextEngine::render`, keyed on text+style debug
   format): text layers used to re-rasterize on EVERY composite — drags and
   animation frames paid full shaping + binary-search fit + blur each time.
   Hits are ~10 µs.
2. **Background cache** (`Renderer::bg_cache`, keyed on size+background):
   the 1080² gradient shader fill is ~250 ms in debug, ~10 ms in release,
   and only changes when the background is edited.
3. **Blit quality**: translation-only transforms (drags) now use Nearest,
   not Bilinear — Bilinear in debug costs ~10x for sub-pixel smoothness
   invisible at meme scale. Bilinear only for scale/rotation.

## Remaining costs (debug) are the rasterizers themselves
Warm composite in release is 2-12 ms — well under a 16 ms frame budget.
Further optimization is noise; don't chase it. If debug ever needs to be
usable, opt tiny-skia/imageproc/cosmic-text to `-C opt-level=2` in the dev
profile rather than optimizing engine code.
