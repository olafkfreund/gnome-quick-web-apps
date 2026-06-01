{
  description = "GNOME Quick Web Apps — GTK4/libadwaita web-app manager";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    { nixpkgs, flake-utils, ... }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs { inherit system; };

        # C libraries needed at build (pkg-config) and run time.
        nativeBuildInputs = with pkgs; [
          pkg-config
          wrapGAppsHook4
        ];

        buildInputs = with pkgs; [
          gtk4
          libadwaita
          glib
          cairo
          pango
          gdk-pixbuf
          graphene
          openssl
        ];

        devTools = with pkgs; [
          rustc
          cargo
          rust-analyzer
          clippy
          rustfmt
        ];
      in
      {
        devShells.default = pkgs.mkShell {
          inherit nativeBuildInputs buildInputs;
          packages = devTools;

          shellHook = ''
            echo "GNOME Quick Web Apps dev shell"
            echo "  gtk4:       $(pkg-config --modversion gtk4 2>/dev/null || echo missing)"
            echo "  libadwaita: $(pkg-config --modversion libadwaita-1 2>/dev/null || echo missing)"
            echo "  rustc:      $(rustc --version 2>/dev/null)"
            echo "Build the buildable crates with:"
            echo "  cargo build -p qwa-core -p gnome-quick-web-apps"
          '';
        };
      }
    );
}
