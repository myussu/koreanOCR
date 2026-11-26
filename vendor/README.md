# Patched copies of dependencies

Code here is somebody else's crate with a small change Maru needs until upstream has it. Both workspaces use it
through `[patch.crates-io]` (the root `Cargo.toml` and `app/src-tauri/Cargo.toml`); `vendor` is excluded from the
workspace, so Maru's lints and tests do not run on it.

## ort-sys 2.0.0-rc.13

The ONNX Runtime bindings that `oar-ocr` (PP-OCR) pins. Source: the crate as published on crates.io, MIT OR Apache-2.0
(`LICENSE-MIT`, `LICENSE-APACHE` are kept). One change, in `build/dynamic_link.rs`:

* The build script copies the runtime's DLLs (on Windows, `DirectML.dll`) next to the executables. It found that folder
  by going a fixed three levels up from `OUT_DIR`. Cargo 1.100 added a level to the build folder
  (`target/<profile>/build/<pkg>/<hash>/out`), so the copy went to folders that do not exist and the build script
  panicked: every Windows build failed with Rust 1.100 and later. The patched script takes the parent of the `build`
  folder instead and skips a destination folder that does not exist.

Remove this copy and the two `[patch.crates-io]` entries once `oar-ocr` depends on an `ort-sys` that builds with the
current Cargo (check by deleting them and running `cargo build --workspace` on Windows).
