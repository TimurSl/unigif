{
  description = "Unigif: a self-hosted, hashtag-first GIF library";

  # A channel tarball rather than `github:NixOS/nixpkgs/...`: it locks the same
  # way and also works where only the channel host is reachable.
  inputs.nixpkgs.url = "https://channels.nixos.org/nixos-26.05/nixexprs.tar.xz";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in
    {
      packages = forAllSystems (pkgs: rec {
        unigif = pkgs.callPackage ./nix/package.nix { };
        default = unigif;
      });

      nixosModules.default = import ./nix/module.nix self;

      overlays.default = final: _prev: {
        unigif = final.callPackage ./nix/package.nix { };
      };

      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          packages = with pkgs; [
            cargo
            rustc
            clippy
            rustfmt
            rust-analyzer
            cargo-nextest
            sqlx-cli
            nodejs
            ffmpeg-headless
            sqlite
          ];
          RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
          shellHook = ''
            echo "unigif dev shell: cargo run -- serve   |   (cd web && npm install && npm run dev)"
          '';
        };
      });

      # `nix run`: a throwaway local instance in a temporary data directory.
      apps = forAllSystems (pkgs: {
        default = {
          type = "app";
          program = "${pkgs.writeShellScript "unigif-run" ''
            dir=$(mktemp -d)
            trap 'rm -rf "$dir"' EXIT INT TERM
            export UNIGIF_DATA_DIR="$dir/data" UNIGIF_CACHE_DIR="$dir/cache"
            pw="''${UNIGIF_DEV_PASSWORD:-dev-password}"
            printf '%s\n' "$pw" > "$dir/owner-password"
            export UNIGIF_AUTH__INITIAL_OWNER__USERNAME=dev
            export UNIGIF_AUTH__INITIAL_OWNER__PASSWORD_FILE="$dir/owner-password"
            echo "unigif: temporary data in $dir; log in as dev / $pw"
            ${self.packages.${pkgs.stdenv.hostPlatform.system}.unigif}/bin/unigif serve
          ''}";
        };
      });

      formatter = forAllSystems (pkgs: pkgs.nixfmt);

      checks = forAllSystems (pkgs: {
        # Building the package runs the whole cargo test suite (real ffmpeg).
        package = self.packages.${pkgs.stdenv.hostPlatform.system}.unigif;
        # Boots a NixOS VM with the module and nginx and drives the real service.
        vm = import ./nix/vm-test.nix { inherit self pkgs; };
      });
    };
}
