# Unigif server with the web UI embedded.
#   web/  -> buildNpmPackage (Vite build)
#   Rust  -> rustPlatform.buildRustPackage from Cargo.lock, web/dist copied in
#            before compilation so rust-embed can embed it
# ffmpeg-headless is wrapped onto PATH: the server drives it as a subprocess.
{
  lib,
  rustPlatform,
  buildNpmPackage,
  makeWrapper,
  ffmpeg-headless,
}:

let
  version = (lib.importTOML ../Cargo.toml).package.version;

  web = buildNpmPackage {
    pname = "unigif-web";
    inherit version;
    src = lib.fileset.toSource {
      root = ../web;
      fileset = lib.fileset.unions [
        ../web/package.json
        ../web/package-lock.json
        ../web/index.html
        ../web/tsconfig.json
        ../web/vite.config.ts
        ../web/svelte.config.js
        ../web/src
      ];
    };
    npmDepsHash = "sha256-1u7DmhOcj59LjOuPKvlJZxXI+vigSSn3FwiPEJqDGKU=";
    installPhase = ''
      runHook preInstall
      cp -r dist $out
      runHook postInstall
    '';
  };
in
rustPlatform.buildRustPackage {
  pname = "unigif";
  inherit version;

  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../Cargo.toml
      ../Cargo.lock
      ../build.rs
      ../rustfmt.toml
      ../LICENSE
      ../src
      ../api
      ../migrations
      ../tests
    ];
  };

  cargoLock.lockFile = ../Cargo.lock;

  nativeBuildInputs = [ makeWrapper ];

  preBuild = ''
    rm -rf web/dist
    mkdir -p web
    cp -r ${web} web/dist
    chmod -R u+w web/dist
  '';

  # The integration tests run real ffmpeg.
  nativeCheckInputs = [ ffmpeg-headless ];

  postInstall = ''
    wrapProgram $out/bin/unigif --prefix PATH : ${lib.makeBinPath [ ffmpeg-headless ]}
  '';

  passthru = { inherit web; };

  meta = {
    description = "Self-hosted, hashtag-first GIF library with a scoped-key API and MP4 storage";
    homepage = "https://github.com/TimurSl/unigif";
    license = lib.licenses.mit;
    mainProgram = "unigif";
    platforms = [
      "x86_64-linux"
      "aarch64-linux"
    ];
  };
}
