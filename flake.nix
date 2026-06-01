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

        # System libraries the prebuilt CEF (libcef.so) links against. The
        # nix cc-wrapper adds -L/-rpath for each, satisfying both link and
        # runtime resolution from the nix store.
        cefRuntimeLibs = with pkgs; [
          nss
          nspr
          atk
          at-spi2-atk
          at-spi2-core
          dbus
          cups
          expat
          libgbm
          libxkbcommon
          alsa-lib
          udev
          libdrm
          libx11
          libxcomposite
          libxdamage
          libxext
          libxfixes
          libxrandr
          libxcb
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
        ] ++ cefRuntimeLibs;

        devTools = with pkgs; [
          rustc
          cargo
          rust-analyzer
          clippy
          rustfmt
          just
          xvfb-run # headless run-verification of the CEF runner
        ];
      in
      {
        devShells.default = pkgs.mkShell {
          inherit nativeBuildInputs buildInputs;
          packages = devTools;

          # GNU ld resolves libcef.so's NEEDED transitive deps at link time via
          # LD_LIBRARY_PATH (native fallback); it also satisfies them at runtime.
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath (buildInputs ++ cefRuntimeLibs);

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
