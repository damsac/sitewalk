{ pkgs ? import <nixpkgs> {} }:
pkgs.mkShell {
  packages = with pkgs; [ cargo rustc cmake clang ];
  # bindgen needs libclang on its path:
  LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
}
