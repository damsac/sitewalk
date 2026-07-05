{
  description = "Murmur rebuild — Rust core workspace";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };
        # Pinned stable toolchain (matches the host rustc this repo was built
        # against: 1.95, edition 2021) with iOS targets added so `cargo build
        # -p ffi --target aarch64-apple-ios{,-sim}` links against a real
        # core/std instead of failing E0463. This replaces pkgs.cargo/rustc —
        # rustup is NOT used; the overlay toolchain is the single source of
        # cargo/rustc/clippy/rustfmt for both host and iOS targets.
        rustToolchain = pkgs.rust-bin.stable."1.95.0".default.override {
          extensions = [ "rust-src" "clippy" "rustfmt" ];
          targets = [
            "aarch64-apple-ios"
            "aarch64-apple-ios-sim"
            "x86_64-apple-ios"
          ];
        };
      in {
        devShells.default = pkgs.mkShell {
          packages = [ rustToolchain pkgs.rust-analyzer pkgs.cmake pkgs.clang ];
          # bindgen (whisper-rs-sys) needs libclang on its path for `--features whisper`:
          LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
        };
      });
}
