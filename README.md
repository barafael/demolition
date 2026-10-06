# Demolition Pong

A 2D physics sandbox and level editor where everything — walls, paddles, the ball — is built from
small rigid cells held together by bonds that bend, yield and snap. Pong is one preset level.

Built with Bevy 0.19, Avian 2D 0.7 and egui.

**Play in the browser:** [editor](https://barafael.github.io/demolition/) ·
[straight into Pong](https://barafael.github.io/demolition/?level=pong&play)

## How destruction works

Each destructible element is a grid of cells joined by compliant fixed joints; elements are fixed
to the world (or to a player-driven carrier) by pins of the same kind. Every physics step each
bond's strain is measured: elastic strain springs back, strain past the material's yield is
absorbed into the bond's rest pose (it stays bent) and accumulates damage, and the bond breaks
when the damage exceeds the material's ductility or the strain its break limit.

## Run natively

```sh
cargo run --release                          # editor with the Lab preset
cargo run --release -- --level pong --play   # straight into Pong
```

**Editor:** drag to move, RMB/MMB pan, wheel zoom, Q/E rotate, Ctrl+D duplicate, Del delete,
Ctrl+Z / Ctrl+Shift+Z undo/redo, Tab to play. **Play:** R restart, C clear debris, B stress
overlay, M slow-mo, Space pause, Tab back to the editor. Pong: WASD vs. arrow keys.

Levels are saved as RON in `levels/`; "Share" in the editor copies a level as text.

Headless tuning tools: `--probe` (materials × ammo matrix), `--pong`, `--diag [substeps]`, `--bench`.

## Web build

Needs the `wasm32-unknown-unknown` target and [trunk](https://trunkrs.dev):

```sh
rustup target add wasm32-unknown-unknown
trunk serve                                              # dev server on http://localhost:8080
trunk build --release --cargo-profile wasm-release       # optimized bundle in dist/
```

`dist/` is static files; host it anywhere. For a sub-path (e.g. GitHub Pages) add
`--public-url /repo-name/`. Serving the `.wasm` with gzip or brotli cuts the download from about
25 MB to about 7 MB.

In the browser, levels and highscores are kept in `localStorage`. Link straight into a level
with `?level=pong&play` (any preset or saved level name).
