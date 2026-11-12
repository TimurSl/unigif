# NixOS module: services.unigif
#
# Takes the flake (`self`) so that `package` can default to the flake's own
# build for the host's system. See docs/deployment.md for a walk-through.
self:
{
  config,
  lib,
  pkgs,
  ...
}:

let
  inherit (lib)
    mkEnableOption
    mkIf
    mkMerge
    mkOption
    types
    ;

  cfg = config.services.unigif;
  tomlFormat = pkgs.formats.toml { };

  # ---- helpers ----------------------------------------------------------

  # "50MiB" / "2 GiB" / 1500 -> bytes, with the same units as the server's
  # own parser (src/config.rs); only whole numbers.
  sizeToBytes =
    what: v:
    if builtins.isInt v then
      v
    else
      let
        m = builtins.match "([0-9]+)[[:space:]]*([A-Za-z]*)" (toString v);
        units = {
          "" = 1;
          b = 1;
          k = 1000;
          kb = 1000;
          m = 1000000;
          mb = 1000000;
          g = 1000000000;
          gb = 1000000000;
          kib = 1024;
          mib = 1048576;
          gib = 1073741824;
        };
        unit = if m == null then null else lib.toLower (builtins.elemAt m 1);
      in
      if m == null || !(units ? ${unit}) then
        throw "services.unigif: ${what} = ${builtins.toJSON v} is not a whole number of bytes with an optional unit (B, KB, MB, GB, KiB, MiB, GiB)"
      else
        lib.toInt (builtins.elemAt m 0) * units.${unit};

  # The host part of publicUrl, for the nginx virtual host.
  publicHost =
    let
      m = builtins.match "[a-zA-Z][a-zA-Z0-9+.-]*://([^/:?#]+)(:[0-9]+)?([/?#].*)?" cfg.publicUrl;
    in
    if m == null then null else builtins.elemAt m 0;

  # Where nginx (or a health check) reaches the server on this machine.
  localAddress =
    {
      "0.0.0.0" = "127.0.0.1";
      "::" = "[::1]";
    }
    .${cfg.listen.address} or (
      if lib.hasInfix ":" cfg.listen.address then "[${cfg.listen.address}]" else cfg.listen.address
    );
  listenString =
    if lib.hasInfix ":" cfg.listen.address then
      "[${cfg.listen.address}]:${toString cfg.listen.port}"
    else
      "${cfg.listen.address}:${toString cfg.listen.port}";

  credentialsDir = "/run/credentials/unigif.service";

  # ---- the configuration file ------------------------------------------

  baseSettings = {
    publicUrl = cfg.publicUrl;
    listen = listenString;
    dataDir = cfg.dataDir;
    cacheDir = cfg.cacheDir;
    media = {
      ffmpeg = "${cfg.ffmpegPackage}/bin/ffmpeg";
      ffprobe = "${cfg.ffmpegPackage}/bin/ffprobe";
    };
    auth =
      lib.optionalAttrs cfg.nginx.enable {
        # The proxy's X-Forwarded-For is believed, so login throttling and
        # rate limits see the real client address.
        trustedProxies = [
          "127.0.0.1"
          "::1"
        ];
      }
      // lib.optionalAttrs (cfg.initialOwner.username != null) {
        initialOwner = {
          username = cfg.initialOwner.username;
          passwordFile = "${credentialsDir}/owner-password";
        };
      };
  };

  settings = lib.recursiveUpdate baseSettings cfg.settings;
  configFile = tomlFormat.generate "unigif.toml" settings;

  uploadMax = sizeToBytes "settings.upload.maxSize" (settings.upload.maxSize or "50MiB");

  usesDefaultState = cfg.dataDir == "/var/lib/unigif";
  usesDefaultCache = cfg.cacheDir == "/var/cache/unigif";
  extraWritable =
    lib.optional (!usesDefaultState) cfg.dataDir ++ lib.optional (!usesDefaultCache) cfg.cacheDir;

  # ---- systemd ------------------------------------------------------------

  hardening = {
    NoNewPrivileges = true;
    ProtectSystem = "strict";
    ProtectHome = true;
    PrivateTmp = true;
    PrivateDevices = true;
    PrivateUsers = true;
    ProtectKernelTunables = true;
    ProtectKernelModules = true;
    ProtectKernelLogs = true;
    ProtectControlGroups = true;
    ProtectClock = true;
    ProtectHostname = true;
    ProtectProc = "invisible";
    ProcSubset = "pid";
    RestrictAddressFamilies = [
      "AF_INET"
      "AF_INET6"
      "AF_UNIX"
    ];
    RestrictNamespaces = true;
    RestrictRealtime = true;
    RestrictSUIDSGID = true;
    LockPersonality = true;
    MemoryDenyWriteExecute = true;
    RemoveIPC = true;
    SystemCallArchitectures = "native";
    SystemCallFilter = [
      "@system-service"
      "~@privileged"
    ];
    CapabilityBoundingSet = "";
    AmbientCapabilities = "";
    DevicePolicy = "closed";
    UMask = "0077";
  };

  commonService = {
    User = cfg.user;
    Group = cfg.group;
    ReadWritePaths = extraWritable;
  };

  unigifCtl = pkgs.writeShellScriptBin "unigif-ctl" ''
    # Runs the unigif CLI as the service user with the service's configuration:
    #   unigif-ctl user passwd timur | key create --name laptop --preset full | doctor
    if [ "$(id -u)" -ne 0 ]; then
      echo "unigif-ctl: run it as root (for example with sudo)" >&2
      exit 1
    fi
    exec ${pkgs.util-linux}/bin/runuser -u ${cfg.user} -- \
      env UNIGIF_CONFIG=${configFile} ${cfg.package}/bin/unigif "$@"
  '';
in
{
  options.services.unigif = {
    enable = mkEnableOption "Unigif, a self-hosted hashtag-first GIF library";

    package = mkOption {
      type = types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.unigif;
      defaultText = lib.literalExpression "unigif.packages.\${system}.unigif";
      description = "The Unigif server, with the web UI embedded.";
    };

    ffmpegPackage = mkOption {
      type = types.package;
      default = pkgs.ffmpeg-headless;
      defaultText = lib.literalExpression "pkgs.ffmpeg-headless";
      description = "ffmpeg used for conversions. It needs libx264 and libwebp.";
    };

    publicUrl = mkOption {
      type = types.str;
      example = "https://gif.example.com";
      description = ''
        The address people reach the service at. It is used to build absolute
        media URLs, to decide whether the session cookie is `Secure` and (with
        `nginx.enable`) for the virtual host name.
      '';
    };

    listen = {
      address = mkOption {
        type = types.str;
        default = "127.0.0.1";
        description = "Address the server binds to. Keep it on loopback when a reverse proxy is in front.";
      };
      port = mkOption {
        type = types.port;
        default = 8787;
        description = "TCP port the server binds to.";
      };
    };

    dataDir = mkOption {
      type = types.path;
      default = "/var/lib/unigif";
      description = "The database and the stored media. This is what to back up.";
    };

    cacheDir = mkOption {
      type = types.path;
      default = "/var/cache/unigif";
      description = "On-demand GIF, WebM and WebP conversions. Safe to delete at any time; never back it up.";
    };

    user = mkOption {
      type = types.str;
      default = "unigif";
      description = "Service user. The module creates it only when it is left at the default.";
    };

    group = mkOption {
      type = types.str;
      default = "unigif";
      description = "Service group. The module creates it only when it is left at the default.";
    };

    settings = mkOption {
      type = tomlFormat.type;
      default = { };
      example = lib.literalExpression ''
        {
          upload.maxSize = "50MiB";
          media.keepOriginals = "alpha";
          cache.maxSize = "2GiB";
        }
      '';
      description = ''
        Free-form configuration, written to the TOML config file; the keys are
        the ones documented in the README and in `src/config.rs`. It is merged
        over the values this module derives from its other options.

        The file is world-readable in the Nix store: put secrets (for example
        `search.giphyApiKey`) in `environmentFile` instead.
      '';
    };

    environmentFile = mkOption {
      type = types.nullOr types.path;
      default = null;
      example = "/run/agenix/unigif-env";
      description = ''
        A systemd `EnvironmentFile` with secrets as `UNIGIF_*` variables, for
        example `UNIGIF_SEARCH__GIPHY_API_KEY=...`. It is read by systemd as
        root, so it works with agenix and sops-nix.
      '';
    };

    initialOwner = {
      username = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "Creates this owner account on start when the database has no user yet.";
      };
      passwordFile = mkOption {
        type = types.nullOr types.path;
        default = null;
        example = "/run/agenix/unigif-owner-password";
        description = "File whose first line is the owner's password. Read by systemd as root.";
      };
    };

    resources = {
      memoryMax = mkOption {
        type = types.str;
        default = "2G";
        description = "systemd `MemoryMax`, so that a hostile file cannot make ffmpeg exhaust the host.";
      };
      tasksMax = mkOption {
        type = types.int;
        default = 512;
        description = "systemd `TasksMax`.";
      };
    };

    nginx = {
      enable = mkEnableOption "an nginx virtual host for `publicUrl` that proxies to Unigif";
      enableACME = mkOption {
        type = types.bool;
        default = true;
        description = "Get a Let's Encrypt certificate (the host must set `security.acme.acceptTerms` and an email).";
      };
      forceSSL = mkOption {
        type = types.bool;
        default = true;
        description = "Redirect HTTP to HTTPS.";
      };
    };

    openFirewall = mkOption {
      type = types.bool;
      default = false;
      description = "Open `listen.port`. Only useful without the reverse proxy.";
    };

    backup = {
      enable = mkEnableOption "a timer that writes a consistent snapshot of the database into `dataDir/backups`";
      calendar = mkOption {
        type = types.str;
        default = "daily";
        description = "systemd `OnCalendar` expression.";
      };
      keep = mkOption {
        type = types.ints.positive;
        default = 7;
        description = "How many snapshots to keep.";
      };
    };
  };

  config = mkIf cfg.enable (mkMerge [
    {
      assertions = [
        {
          assertion = cfg.publicUrl != "";
          message = "services.unigif.publicUrl must be set.";
        }
        {
          assertion = (cfg.initialOwner.username == null) == (cfg.initialOwner.passwordFile == null);
          message = "services.unigif.initialOwner.username and .passwordFile must be set together.";
        }
        {
          assertion = !(cfg.nginx.enable && publicHost == null);
          message = "services.unigif.publicUrl must look like https://host[:port] when nginx.enable is set.";
        }
        {
          assertion = lib.all (k: !(cfg.settings ? ${k})) [
            "publicUrl"
            "listen"
            "dataDir"
            "cacheDir"
          ];
          message = "services.unigif.settings must not set publicUrl, listen, dataDir or cacheDir: use the matching module options.";
        }
        {
          assertion = !(cfg.settings ? auth && cfg.settings.auth ? initialOwner);
          message = "services.unigif.settings.auth.initialOwner is managed by services.unigif.initialOwner.";
        }
      ];

      environment.etc."unigif/config.toml".source = configFile;
      environment.systemPackages = [ unigifCtl ];

      users.users = mkIf (cfg.user == "unigif") {
        unigif = {
          isSystemUser = true;
          group = cfg.group;
          description = "Unigif service";
        };
      };
      users.groups = mkIf (cfg.group == "unigif") { unigif = { }; };

      systemd.tmpfiles.rules = map (d: "d ${d} 0700 ${cfg.user} ${cfg.group} -") extraWritable;

      systemd.services.unigif = {
        description = "Unigif GIF library";
        wantedBy = [ "multi-user.target" ];
        after = [ "network.target" ];
        environment = {
          UNIGIF_CONFIG = "${configFile}";
          RUST_LOG = "info";
        };
        serviceConfig = hardening // commonService // {
          ExecStart = "${cfg.package}/bin/unigif serve";
          Restart = "on-failure";
          RestartSec = "5s";
          StateDirectory = mkIf usesDefaultState "unigif";
          StateDirectoryMode = "0700";
          CacheDirectory = mkIf usesDefaultCache "unigif";
          CacheDirectoryMode = "0700";
          RuntimeDirectory = "unigif";
          RuntimeDirectoryMode = "0700";
          EnvironmentFile = mkIf (cfg.environmentFile != null) cfg.environmentFile;
          LoadCredential = mkIf (cfg.initialOwner.passwordFile != null) [
            "owner-password:${toString cfg.initialOwner.passwordFile}"
          ];
          MemoryMax = cfg.resources.memoryMax;
          TasksMax = cfg.resources.tasksMax;
          LimitNOFILE = 65536;
        };
      };

      networking.firewall.allowedTCPPorts = mkIf cfg.openFirewall [ cfg.listen.port ];
    }

    (mkIf cfg.nginx.enable {
      services.nginx = {
        enable = true;
        virtualHosts.${publicHost} = {
          enableACME = cfg.nginx.enableACME;
          forceSSL = cfg.nginx.forceSSL;
          # The server enforces upload.maxSize exactly; this only has to let
          # it through (the API allows 1 MiB of multipart framing on top).
          extraConfig = ''
            client_max_body_size ${toString (uploadMax + 1048576)};
          '';
          locations."/" = {
            proxyPass = "http://${localAddress}:${toString cfg.listen.port}";
            extraConfig = ''
              proxy_http_version 1.1;
              proxy_set_header Host $host;
              proxy_set_header X-Real-IP $remote_addr;
              proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
              proxy_set_header X-Forwarded-Proto $scheme;
              # Uploads stream straight to the server instead of being spooled to disk first.
              proxy_request_buffering off;
              proxy_read_timeout 300s;
              proxy_send_timeout 300s;
            '';
          };
          # The live-update stream must not be buffered and stays open for long.
          locations."= /api/v1/events" = {
            proxyPass = "http://${localAddress}:${toString cfg.listen.port}";
            extraConfig = ''
              proxy_http_version 1.1;
              proxy_set_header Host $host;
              proxy_set_header X-Real-IP $remote_addr;
              proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
              proxy_set_header X-Forwarded-Proto $scheme;
              proxy_set_header Connection "";
              proxy_buffering off;
              proxy_cache off;
              proxy_read_timeout 1h;
            '';
          };
        };
      };
    })

    (mkIf cfg.backup.enable {
      systemd.services.unigif-backup = {
        description = "Unigif database snapshot";
        environment.UNIGIF_CONFIG = "${configFile}";
        serviceConfig = hardening // commonService // {
          Type = "oneshot";
          ExecStart = "${cfg.package}/bin/unigif backup --keep ${toString cfg.backup.keep}";
          StateDirectory = mkIf usesDefaultState "unigif";
          StateDirectoryMode = "0700";
          MemoryMax = "512M";
        };
      };
      systemd.timers.unigif-backup = {
        description = "Unigif database snapshot";
        wantedBy = [ "timers.target" ];
        timerConfig = {
          OnCalendar = cfg.backup.calendar;
          Persistent = true;
          RandomizedDelaySec = "10m";
        };
      };
    })
  ]);
}
