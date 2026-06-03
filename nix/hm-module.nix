# Home Manager module for GNOME Quick Web Apps.
#
# Lets you declare web apps "the Nix way" — in your Home Manager config —
# instead of (or alongside) clicking through the GUI manager. Each declared app
# is written as a `apps/<id>.json` config the runner reads, plus a `.desktop`
# launcher wired to the bundled CEF runner. Fully declarative: no portal calls,
# no activation scripts, reproducible across machines.
#
# `self` is the flake, used to default `package` to the matching system build.
self:
{ config, lib, pkgs, ... }:

let
  inherit (lib) mkOption mkEnableOption mkIf types literalExpression;

  cfg = config.programs.quick-web-apps;
  appId = "io.github.olafkfreund.QuickWebApps";

  # Freedesktop main categories the app understands (mirrors core Category).
  categories = [
    "Audio" "AudioVideo" "Video" "Development" "Education" "Game"
    "Graphics" "Network" "Office" "Science" "Settings" "System" "Utility"
  ];

  appModule = types.submodule ({ name, ... }: {
    options = {
      name = mkOption {
        type = types.str;
        default = name;
        description = "Display name shown in the app grid and window title.";
      };
      url = mkOption {
        type = types.str;
        example = "https://music.youtube.com";
        description = "The site this web app opens.";
      };
      category = mkOption {
        type = types.enum categories;
        default = "Utility";
        description = "Freedesktop application category.";
      };
      icon = mkOption {
        type = types.nullOr types.path;
        default = null;
        example = literalExpression "./youtube-music.png";
        description = ''
          Icon file (PNG or SVG) for the launcher. When null, the generic
          Quick Web Apps icon is used.
        '';
      };
      profile = mkOption {
        type = types.nullOr types.str;
        default = null;
        example = "google";
        description = ''
          Shared session profile name. Apps with the same profile share
          cookies/logins. Null means a private per-app profile keyed by the id.
        '';
      };
      scope = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "Origin/path prefix in-app navigation is confined to.";
      };
      userAgent = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "Override the User-Agent string.";
      };
      mobile = mkOption {
        type = types.bool;
        default = false;
        description = "Request the mobile version of the site.";
      };
      linkScope = mkOption {
        type = types.nullOr (types.enum [ "in_window" "same_site" "exact_host" ]);
        default = null;
        description = ''
          Link handling: keep everything in-window, send other registrable
          domains to the browser (same_site), or send any other host
          (exact_host). Null falls back to externalLinksInBrowser.
        '';
      };
      externalLinksInBrowser = mkOption {
        type = types.bool;
        default = false;
        description = "Open deliberate off-site links in the system browser.";
      };
      adblock = mkOption {
        type = types.bool;
        default = false;
        description = "Apply the bundled ad/tracker content filter.";
      };
      colorScheme = mkOption {
        type = types.enum [ "system" "light" "dark" ];
        default = "system";
        description = "Force light/dark appearance, or follow the system.";
      };
      runInBackground = mkOption {
        type = types.bool;
        default = false;
        description = "Keep running after the window is closed (for notifications).";
      };
      customCss = mkOption {
        type = types.nullOr types.lines;
        default = null;
        description = "User CSS injected into every page after load.";
      };
      allowCameraMic = mkOption {
        type = types.bool;
        default = true;
        description = "Allow the site to use the camera and microphone.";
      };
      allowLocation = mkOption {
        type = types.bool;
        default = false;
        description = "Allow the site to access geolocation.";
      };
      showBadge = mkOption {
        type = types.bool;
        default = false;
        description = "Show an unread-count badge on the dock, sourced from the title.";
      };
      autostart = mkOption {
        type = types.bool;
        default = false;
        description = "Start this app automatically on login (pairs with runInBackground).";
      };
      themeColor = mkOption {
        type = types.nullOr types.str;
        default = null;
        example = "#ff0000";
        description = "Theme colour (#rrggbb) used for the splash.";
      };
      width = mkOption {
        type = types.ints.positive;
        default = 960;
        description = "Initial window width.";
      };
      height = mkOption {
        type = types.ints.positive;
        default = 720;
        description = "Initial window height.";
      };
      handlers = mkOption {
        type = types.listOf (types.submodule {
          options = {
            mime = mkOption {
              type = types.str;
              example = "x-scheme-handler/mailto";
              description = "Freedesktop mime / scheme handler.";
            };
            template = mkOption {
              type = types.str;
              description = "Target URL template; {value} is filled from the scheme URL.";
            };
          };
        });
        default = [ ];
        description = "URL-scheme handlers this app registers as the default for.";
      };
      setDefaultHandlers = mkOption {
        type = types.bool;
        default = false;
        description = ''
          Make this app the system default for each of its `handlers` mime
          types, via `xdg.mimeApps.defaultApplications`. Off by default so the
          module never clobbers your existing defaults; opt in per app. On
          NixOS this is the only way to set the default (the runtime
          `xdg-mime` path can't write a read-only managed `mimeapps.list`).
        '';
      };
      extraConfig = mkOption {
        type = types.attrs;
        default = { };
        description = ''
          Extra keys merged verbatim into the generated app JSON, for fields
          not yet exposed as options. Uses the on-disk JSON key names.
        '';
      };
    };
  });

  # The full WebApp JSON for one app. Emits every field (the runner's schema
  # has required, non-defaulted keys) so the config always deserializes.
  mkAppJson = id: app: builtins.toJSON ({
    inherit id;
    inherit (app) name url category;
    scope = app.scope;
    icon_path = if app.icon == null then null else toString app.icon;
    theme_color = app.themeColor;
    user_agent = app.userAgent;
    profile = app.profile;
    mobile = app.mobile;
    external_links_in_browser = app.externalLinksInBrowser;
    link_scope = app.linkScope;
    handlers = map (h: { inherit (h) mime template; }) app.handlers;
    window = [ app.width app.height ];
    adblock = app.adblock;
    color_scheme = app.colorScheme;
    run_in_background = app.runInBackground;
    custom_css = app.customCss;
    allow_camera_mic = app.allowCameraMic;
    allow_location = app.allowLocation;
    show_badge = app.showBadge;
    autostart = app.autostart;
  } // app.extraConfig);

  runnerBin = "${cfg.package}/bin/gnome-quick-web-apps-runner";

  # One app/<id>.json file under the data dir the runner reads.
  appConfigFiles = lib.mapAttrs' (id: app:
    lib.nameValuePair "${appId}/apps/${id}.json" {
      text = mkAppJson id app;
    }) cfg.apps;

  # One .desktop launcher per app, matching the GUI's filename + StartupWMClass
  # so the window groups under its own dock icon.
  desktopEntries = lib.mapAttrs' (id: app:
    lib.nameValuePair "${appId}.${id}" {
      name = app.name;
      exec = "${runnerBin} ${id}" + (lib.optionalString (app.handlers != [ ]) " %u");
      icon = if app.icon == null then appId else toString app.icon;
      categories = [ app.category ];
      mimeType = map (h: h.mime) app.handlers;
      settings.StartupWMClass = "${appId}.${id}";
    }) cfg.apps;

  # Default-application map (mime -> .desktop) for apps that opted into
  # setDefaultHandlers. On NixOS this is the only way to set default handlers,
  # since the runtime xdg-mime path can't write a managed mimeapps.list. Only
  # apps that opt in appear here, so existing defaults are never clobbered.
  handlerDefaults = lib.listToAttrs (lib.flatten (lib.mapAttrsToList (id: app:
    if app.setDefaultHandlers then
      map (h: lib.nameValuePair h.mime "${appId}.${id}.desktop") app.handlers
    else
      [ ]) cfg.apps));

in
{
  options.programs.quick-web-apps = {
    enable = mkEnableOption "GNOME Quick Web Apps, with declaratively-defined web apps";

    package = mkOption {
      type = types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
      defaultText = literalExpression "quick-web-apps.packages.\${system}.default";
      description = "The Quick Web Apps package providing the manager and runner.";
    };

    apps = mkOption {
      type = types.attrsOf appModule;
      default = { };
      example = literalExpression ''
        {
          youtube-music = {
            name = "YouTube Music";
            url = "https://music.youtube.com";
            category = "Audio";
          };
          teams = {
            name = "Microsoft Teams";
            url = "https://teams.microsoft.com";
            category = "Network";
            runInBackground = true;
          };
        }
      '';
      description = ''
        Web apps to install declaratively, keyed by a stable id (used for the
        config filename, profile and window class). Use a simple slug like
        `youtube-music`.
      '';
    };
  };

  config = mkIf cfg.enable {
    home.packages = [ cfg.package ];
    xdg.dataFile = appConfigFiles;
    xdg.desktopEntries = desktopEntries;
    # Only manage mimeApps when at least one app opted into setDefaultHandlers,
    # so we never take over a user's mimeapps.list otherwise.
    xdg.mimeApps = lib.mkIf (handlerDefaults != { }) {
      enable = lib.mkDefault true;
      defaultApplications = handlerDefaults;
    };
  };
}
