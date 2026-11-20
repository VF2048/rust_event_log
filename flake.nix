{
  description = "Rust development environment";

  inputs = {
    nixpkgs.url = "github:Nixos/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, rust-overlay, flake-utils, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };
        
        # Выбираем версию Rust. Можно заменить на .nightly."latest".default
        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rust-src" "rust-analyzer" ];
        };
      in
      {
        devShells.default = pkgs.mkShell {
          nativeBuildInputs = with pkgs; [
            rustup
            rustToolchain
            pkg-config
          ];

          buildInputs = with pkgs; [
            # Добавьте сюда системные библиотеки, если cargo ругается на их отсутствие
            # openssl
          ];
        };
      }
    );
}
