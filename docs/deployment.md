# Deploying Unigif on NixOS

Unigif ships a NixOS module (`services.unigif`) that runs the server as a
hardened systemd service, optionally puts nginx with a Let's Encrypt
certificate in front of it, and can snapshot the database on a timer.

## 1. Add the flake and the module

```nix
# flake.nix of your host
{
  inputs.unigif.url = "github:timursl/unigif";
  inputs.unigif.inputs.nixpkgs.follows = "nixpkgs";   # optional: share your nixpkgs

  outputs = { nixpkgs, unigif, ... }: {
    nixosConfigurations.vps = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        unigif.nixosModules.default
        ./configuration.nix
      ];
    };
  };
}
```

`aarch64-linux` is supported too. The package builds the web UI and the server
and wraps `ffmpeg` onto its `PATH`; building it runs the whole test suite.

## 2. Configure the service

```nix
# configuration.nix
{ config, ... }:
{
  services.unigif = {
    enable = true;
    publicUrl = "https://gif.example.com";

    # Secrets stay out of the Nix store. These work with agenix and sops-nix:
    environmentFile = config.age.secrets.unigif-env.path;          # UNIGIF_* variables
    initialOwner = {
      username = "timur";
      passwordFile = config.age.secrets.unigif-owner-password.path; # first line = password
    };

    settings = {
      upload.maxSize = "50MiB";
      media.keepOriginals = "alpha";
      cache.maxSize = "2GiB";
    };

    nginx.enable = true;     # virtual host for publicUrl, TLS via ACME
    backup.enable = true;    # daily database snapshot, 7 kept
  };

  # Needed by nginx.enableACME (the default):
  security.acme = { acceptTerms = true; defaults.email = "you@example.com"; };
  networking.firewall.allowedTCPPorts = [ 80 443 ];
}
```

`environmentFile` holds anything secret, one `NAME=value` per line, for example

```
UNIGIF_SEARCH__GIPHY_API_KEY=...
UNIGIF_SEARCH__KLIPY_API_KEY=...
```

(`settings` is rendered into a world-readable file in the Nix store, so keep keys
out of it. GIF search keys can also be entered in the web UI under Settings.)

### Options

| Option | Default | Meaning |
|---|---|---|
| `enable` | `false` | |
| `package` | flake package | |
| `publicUrl` | required | Used for absolute media URLs, the `Secure` cookie flag and the nginx host name |
| `listen.address`, `listen.port` | `127.0.0.1`, `8787` | Keep loopback behind a proxy |
| `dataDir` | `/var/lib/unigif` | Database and media: **back this up** |
| `cacheDir` | `/var/cache/unigif` | On-demand conversions: never back up |
| `user`, `group` | `unigif` | Created by the module when left at the default |
| `settings` | `{}` | Merged into the TOML config; keys as in the README |
| `environmentFile` | `null` | Secrets as `UNIGIF_*` variables |
| `initialOwner.username`, `.passwordFile` | `null` | Creates the first user when there is none (set both) |
| `ffmpegPackage` | `pkgs.ffmpeg-headless` | Needs libx264 and libwebp |
| `resources.memoryMax`, `.tasksMax` | `2G`, `512` | Caps so a hostile file cannot take the host down |
| `nginx.enable`, `.enableACME`, `.forceSSL` | `false`, `true`, `true` | Virtual host with `client_max_body_size` from `upload.maxSize`, unbuffered uploads, and an unbuffered event stream |
| `openFirewall` | `false` | Only for use without the proxy |
| `backup.enable`, `.calendar`, `.keep` | `false`, `daily`, `7` | Snapshot timer |

With `nginx.enable` the module also lists `127.0.0.1` and `::1` as
`auth.trustedProxies`, so login throttling and rate limits see real client
addresses.

## 3. First start

```bash
sudo nixos-rebuild switch --flake .#vps
systemctl status unigif
curl https://gif.example.com/healthz        # ok
```

Sign in at `publicUrl` as the initial owner. Without `initialOwner`, create the
account from the shell. `unigif-ctl` runs the CLI as the service user with the
service's configuration:

```bash
sudo unigif-ctl user add timur
sudo unigif-ctl key create --name backend --preset read_only
sudo unigif-ctl doctor
```

## 4. Hardening and limits

The unit runs as the static `unigif` user with `NoNewPrivileges`,
`ProtectSystem=strict`, `ProtectHome`, `PrivateTmp`, `PrivateDevices`,
`PrivateUsers`, the `Protect*` kernel switches, `RestrictAddressFamilies`,
`RestrictNamespaces`, `LockPersonality`, `MemoryDenyWriteExecute`,
`SystemCallFilter=@system-service ~@privileged`, an empty capability set and
`UMask=0077`. It can write only to `dataDir` and `cacheDir`. The NixOS VM test
fails if `systemd-analyze security unigif.service` reports an exposure level
above 2.0; check it yourself with

```bash
systemd-analyze security unigif.service
```

## 5. Backups and restore

`backup.enable = true` starts `unigif-backup.timer`, which runs
`unigif backup --keep N` and writes `unigif-<timestamp>.db` into
`dataDir/backups`. A snapshot is made with SQLite's `VACUUM INTO`, so it is
consistent while the server keeps writing, and it is one self-contained file.
Run one by hand:

```bash
sudo systemctl start unigif-backup.service
ls /var/lib/unigif/backups
```

Stored media (`dataDir/media`) never changes after it is written, so the
snapshots plus a copy of `media/` are a complete backup. Point restic, borg or
whatever the host already uses at `dataDir` (it includes `backups/`), and
exclude `cacheDir`.

Before the server applies a database migration it writes
`backups/pre-migration-v<version>-<timestamp>.db` (the newest five are kept),
so an upgrade always has a way back. A failed snapshot stops the start.

**Restore** (the VM test exercises exactly this):

```bash
sudo systemctl stop unigif
sudo rm -f /var/lib/unigif/unigif.db /var/lib/unigif/unigif.db-wal /var/lib/unigif/unigif.db-shm
sudo install -o unigif -g unigif -m 600 /var/lib/unigif/backups/unigif-<timestamp>.db /var/lib/unigif/unigif.db
sudo systemctl start unigif
```

To restore onto a fresh machine, put the backed-up `dataDir` back first (media
and snapshots), then do the same.

## 6. Upgrading

Update the flake input and rebuild. The service restarts, takes a
pre-migration snapshot if the new version has migrations, migrates, and carries
on. `nix flake check` (CI runs it) builds the package, which runs the cargo
tests, and runs the VM test.

## 7. Notes

- **Reverse proxy of your own.** Without `nginx.enable`, proxy to
  `listen.address:listen.port`, pass `Host`, `X-Forwarded-For` and
  `X-Forwarded-Proto`, set `client_max_body_size` above `upload.maxSize` plus
  1 MiB, do not buffer `/api/v1/events`, and list the proxy's address in
  `settings.auth.trustedProxies`.
- **Media is public by unguessable URL**; hide sensitive hashtags from other
  people's keys with a key's `deny_tags`.
- **Running the VM test** needs a machine with KVM for a reasonable runtime:
  `nix build .#checks.x86_64-linux.vm -L`.
