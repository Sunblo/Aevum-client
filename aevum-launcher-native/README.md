# Aevum Launcher

A native, space-themed Minecraft client launcher with a Lunar-style UI, written in Rust with [eframe / egui 0.29](https://github.com/emilk/egui). It downloads real Mojang version metadata, assets, and libraries, then launches an actual Minecraft client via a locally installed Java runtime.

## Features

- Real Mojang version manifest: all release/snapshot versions with online metadata
- Full asset pipeline: version JSON, library + client jar downloads with SHA1 verification, asset objects, and platform-native extraction
- Offline-style profile (username → md5 UUID), 1.16.5 validated end-to-end on Java 17
- Native OS rule handling for libraries (e.g. macOS-only JVM flags are not leaked onto Linux)
- Game output capture: the client's stdout/stderr are written to `game/logs/latest.log` under the launcher data dir
- Custom boot animation with phase-by-phase progress sequence, toasts, and launch overlay
- Frameless window with a custom title bar (minimize / maximize / close)
- Space ambience: deterministic LCG starfield + soft glow accents
- Sidebar navigation between Play, Instances, and Settings views
- Quick account switcher modal (editable offline username)
- Settings: allocated memory slider, toggles (animations, reduce motion, ambient sound), volume slider
- Custom fonts bundled as TTFs: Orbitron (display) + Space Grotesk (body)

## Requirements

- Java 17+ on `PATH` (the client needs it; the launcher locates `java` automatically). Tested with OpenJDK 17.0.20.
- On Linux, the game window needs an X display; run under Xvfb when headless (see Run).
- Windows note: the launcher builds a `;`-separated classpath on Windows and `:` on Unix/macOS.

## Data & cache

Everything is stored under `~/.aevum-launcher/`:

- `versions/<id>/` — version JSON + client jar
- `libraries/` — verified downloaded libraries
- `assets/` — indexed asset objects (textures, sounds)
- `natives/<id>/` — extracted platform natives
- `game/logs/latest.log` — captured game output
- `game/crash-reports/` — Minecraft crash reports

## Build

Requires a Rust toolchain (stable) plus the usual C system libraries that `egui` needs on Linux:

```bash
# Debian / Ubuntu build dependencies
sudo apt-get install -y libx11-dev libxkbcommon-dev libwayland-dev \
  libxrandr-dev libxinerama-dev libxcursor-dev libxi-dev \
  libgl1-mesa-dev libasound2-dev pkg-config
```

```bash
cargo build --release
```

The binary is written to `target/release/aevum-launcher`.

## Run

```bash
./target/release/aevum-launcher
```

On a headless Linux box you can run it under Xvfb with software GL:

```bash
xvfb-run -a -s "-screen 0 1400x900x24" ./target/release/aevum-launcher
```

## Tests

```bash
cargo test
```

`end_to_end_launch_1_16_5` is an integration test that drives the real launch pipeline (downloads, natives extraction, Java spawn) and is `#[ignore]`d by default because it needs network and ~400 MB of assets:

```bash
cargo test end_to_end_launch_1_16_5 -- --ignored --nocapture
```

## Project Layout

- `src/main.rs` — eframe entry point, frameless viewport (1280x800), ignored integration test harness
- `src/theme.rs` — palette, bundled font installation, egui visual style
- `src/starfield.rs` — ambient starfield painted through the CentralPanel painter
- `src/paint.rs` — painting helpers (rounded rects, glow, arcs, cube, chevrons)
- `src/launcher.rs` — real launch engine: manifest, downloads, rules, command build, process monitor, game log capture
- `src/app.rs` — main app: boot sequence, title bar, sidebar, views, modal, launch overlay, toast
- `assets/fonts/` — Orbitron + Space Grotesk TTFs

## Notes

- egui 0.29 specifics: the emoji fallback font key is `NotoEmoji-Regular`; `Rounding` is used instead of `CornerRadius`; `allocate_new_ui` replaces the deprecated `allocate_ui_at_rect`.
- The classpath separator must be the OS value (`:` / `;`), not `std::path::MAIN_SEPARATOR` (`/` / `\`).
- egui colors are premultiplied: a translucent white must be built as `Color32::from_white_alpha(n)` (i.e. `rgba(n,n,n,n)`), never `from_rgba_premultiplied(255,255,255,n)`, which renders as opaque white. Same rule applies to tinted glass fills (RGB ≤ alpha).
