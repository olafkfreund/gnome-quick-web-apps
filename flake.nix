{
  description = "GNOME Quick Web Apps — GTK4/libadwaita web-app manager (CEF runner)";

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
        lib = pkgs.lib;

        # System libraries the prebuilt libcef.so links against.
        cefRuntimeLibs = with pkgs; [
          glib
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
          cairo
          pango
          gdk-pixbuf
          fontconfig
          freetype
          libGL
          libglvnd
          zlib
          libx11
          libxcomposite
          libxdamage
          libxext
          libxfixes
          libxrandr
          libxcb
          libxrender
          libxtst
          libxi
          libxkbfile
          stdenv.cc.cc.lib
        ];

        # CEF 145 binary distribution, pinned to exactly what cef-dll-sys 145
        # expects. Flattened (Release/ + Resources/) into one dir with the
        # archive.json that CEF_PATH validation reads, and autoPatchelf'd so
        # libcef.so resolves its deps on NixOS.
        cef = pkgs.stdenv.mkDerivation {
          pname = "cef-binary";
          version = "145.0.28";
          src = pkgs.fetchurl {
            url = "https://cef-builds.spotifycdn.com/cef_binary_145.0.28%2Bg51162e8%2Bchromium-145.0.7632.160_linux64_minimal.tar.bz2";
            sha256 = "1x1rxi92xc95hd68cqp9ghpiwzmq5h2yizf3jmx93wpl89s7yp06";
          };
          nativeBuildInputs = [ pkgs.autoPatchelfHook ];
          buildInputs = cefRuntimeLibs;
          # libcef.so has undefined refs satisfied at runtime by the host app.
          autoPatchelfIgnoreMissingDeps = true;
          dontConfigure = true;
          dontBuild = true;
          installPhase = ''
            runHook preInstall
            mkdir -p $out
            cp -r Release/* $out/
            cp -r Resources/* $out/
            cat > $out/archive.json <<'EOF'
            {"type":"minimal","name":"cef_binary_145.0.28+g51162e8+chromium-145.0.7632.160_linux64_minimal.tar.bz2","sha1":"b95bd667f5e8baa096dad616a396ed28e06a43d8"}
            EOF
            runHook postInstall
          '';
        };

        package = pkgs.rustPlatform.buildRustPackage {
          pname = "gnome-quick-web-apps";
          version = "0.1.0";
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;

          nativeBuildInputs = with pkgs; [
            pkg-config
            wrapGAppsHook4
            autoPatchelfHook
          ];
          buildInputs = (with pkgs; [ gtk4 libadwaita glib openssl ]) ++ cefRuntimeLibs;

          # Use the pinned CEF instead of downloading it during the build.
          CEF_PATH = cef;
          autoPatchelfIgnoreMissingDeps = true;

          postInstall = ''
            # Deploy the CEF runtime next to the binaries (cef_dir() finds it).
            mkdir -p $out/share/cef
            cp -r ${cef}/* $out/share/cef/

            install -Dm644 data/io.github.olafkfreund.QuickWebApps.desktop \
              $out/share/applications/io.github.olafkfreund.QuickWebApps.desktop
            install -Dm644 data/io.github.olafkfreund.QuickWebApps.metainfo.xml \
              $out/share/metainfo/io.github.olafkfreund.QuickWebApps.metainfo.xml
            install -Dm644 data/icons/hicolor/scalable/apps/io.github.olafkfreund.QuickWebApps.svg \
              $out/share/icons/hicolor/scalable/apps/io.github.olafkfreund.QuickWebApps.svg
          '';

          # Let the runner/helper find libcef.so directly (the .desktop also
          # sets LD_LIBRARY_PATH, but this makes direct launches work too).
          postFixup = ''
            for bin in gnome-quick-web-apps-runner gnome-quick-web-apps-runner-helper; do
              if [ -e "$out/bin/$bin" ]; then
                patchelf --add-rpath "$out/share/cef" "$out/bin/$bin"
              fi
            done
          '';

          meta = with lib; {
            description = "Turn any website into a first-class GNOME desktop app";
            homepage = "https://github.com/olafkfreund/gnome-quick-web-apps";
            license = licenses.gpl3Only;
            platforms = platforms.linux;
            mainProgram = "gnome-quick-web-apps";
          };
        };

        libPath = lib.makeLibraryPath (
          (with pkgs; [ gtk4 libadwaita glib cairo pango gdk-pixbuf graphene openssl ])
          ++ cefRuntimeLibs
        );
      in
      {
        packages.default = package;
        packages.gnome-quick-web-apps = package;

        devShells.default = pkgs.mkShell {
          nativeBuildInputs = with pkgs; [ pkg-config wrapGAppsHook4 ];
          buildInputs =
            (with pkgs; [ gtk4 libadwaita glib cairo pango gdk-pixbuf graphene openssl ])
            ++ cefRuntimeLibs;
          packages = with pkgs; [ rustc cargo rust-analyzer clippy rustfmt just patchelf xvfb-run ];

          LD_LIBRARY_PATH = libPath;
          RUSTFLAGS = "-C link-arg=-Wl,--disable-new-dtags -C link-arg=-Wl,-rpath,${libPath}";

          shellHook = ''
            echo "GNOME Quick Web Apps dev shell"
            echo "  gtk4: $(pkg-config --modversion gtk4 2>/dev/null || echo missing)  rustc: $(rustc --version 2>/dev/null)"
          '';
        };
      }
    );
}
