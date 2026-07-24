# Linux builder image for the full desktop app (Tauri v2) + native helpers.
# Used by make-distributables.sh to build a Linux distributable from macOS via Docker.
# Contains the Rust toolchain, Node, and every system -dev package the core + helpers link against.
FROM rust:1-bookworm

RUN apt-get update && apt-get install -y --no-install-recommends \
    # --- Tauri v2 (GTK / WebKitGTK / tray / svg) ---
    libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev \
    libssl-dev pkg-config build-essential file ca-certificates curl wget \
    # --- app crates: cpal(ALSA), arboard/global-shortcut(X11), enigo(xdo) ---
    libasound2-dev libxdo-dev \
    libxcb1-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev \
    # --- native/linux helpers: PipeWire/Pulse, AT-SPI, X11, GStreamer ---
    libpulse-dev libpipewire-0.3-dev libspa-0.2-dev clang \
    libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev \
    libatspi2.0-dev libdbus-1-dev libxcb-randr0-dev \
    # --- Node for the frontend (prepare_web_assets.sh + @tauri-apps/cli) ---
    nodejs npm \
    && rm -rf /var/lib/apt/lists/*

# Bake the Rust Tauri CLI (matches the project's tauri 2.11.x) so the build needs no npm/node_modules
# native binary (which fails to exec under a mapped non-root uid). Installed to /usr/local/cargo/bin,
# on PATH for all users, so `cargo tauri build` works even when CARGO_HOME is remapped at run time.
RUN cargo install tauri-cli --version '~2.11.4' --locked

# cmake — needed to build the bundled C++ model runtimes (transcribe-server / llama-server) for Linux.
# Separate layer so it doesn't invalidate the tauri-cli build cache above.
RUN apt-get update && apt-get install -y --no-install-recommends cmake git && rm -rf /var/lib/apt/lists/*

# Cargo/registry caches live under the mounted repo so they persist across runs (set at run time via
# CARGO_HOME).
WORKDIR /src
