# The NixOS VM test from section 10.4 of PLAN.md: the module, nginx and the
# real binary, driven over HTTP the way a client would.
{ self, pkgs }:

pkgs.testers.runNixOSTest {
  name = "unigif";

  nodes.machine =
    { pkgs, ... }:
    {
      imports = [ self.nixosModules.default ];

      services.unigif = {
        enable = true;
        publicUrl = "http://unigif.test";
        initialOwner = {
          username = "owner";
          passwordFile = pkgs.writeText "unigif-owner-password" "vm-test-password\n";
        };
        nginx = {
          enable = true;
          enableACME = false;
          forceSSL = false;
        };
        backup.enable = true;
        settings = {
          media.preset = "ultrafast";
          upload.maxSize = "20MiB";
        };
      };

      networking.hosts."127.0.0.1" = [ "unigif.test" ];
      environment.systemPackages = with pkgs; [
        curl
        jq
        ffmpeg-headless
      ];
      virtualisation.memorySize = 2048;
    };

  testScript = ''
    import json

    base = "http://unigif.test"
    jar = "/tmp/cookies"

    def api(method="GET", path="/", token=None, body=None, ctype="application/json", expect=None, data_file=None):
        """One curl call through nginx; returns (status, parsed JSON or raw text)."""
        cmd = ["curl", "-sS", "-o", "/tmp/out", "-w", "%{http_code}", "-X", method]
        if token:
            cmd += ["-H", f"Authorization: Bearer {token}"]
        else:
            cmd += ["-b", jar, "-c", jar]
        cmd += ["-H", f"Origin: {base}"]
        if body is not None:
            cmd += ["-H", f"Content-Type: {ctype}", "--data", json.dumps(body)]
        if data_file is not None:
            cmd += ["-H", f"Content-Type: {ctype}", "--data-binary", f"@{data_file}"]
        cmd += [base + path]
        status = int(machine.succeed(" ".join(f"'{c}'" for c in cmd)).strip())
        raw = machine.succeed("cat /tmp/out")
        try:
            parsed = json.loads(raw)
        except ValueError:
            parsed = raw
        if expect is not None:
            assert status == expect, f"{method} {path}: expected {expect}, got {status}: {raw}"
        return status, parsed

    def probe(path, entries):
        return json.loads(machine.succeed(
            f"ffprobe -v error -select_streams v:0 -show_entries stream={entries} -of json {path}"
        ))["streams"][0]

    machine.start()
    machine.wait_for_unit("unigif.service")
    machine.wait_for_unit("nginx.service")
    machine.wait_for_open_port(8787)
    machine.wait_for_open_port(80)

    with subtest("the service answers through nginx"):
        machine.succeed(f"curl -fsS {base}/healthz | grep -qx ok")
        machine.succeed(f"curl -fsS {base}/readyz | grep -qx ready")
        machine.succeed(f"curl -fsS {base}/ | grep -qi '<html'")

    with subtest("hardening reaches the exposure target"):
        out = machine.succeed("systemd-analyze security unigif.service --no-pager")
        print(out)
        score = float(machine.succeed(
            "systemd-analyze security unigif.service --no-pager | grep 'Overall exposure level' | grep -o '[0-9]\\+\\.[0-9]\\+'"
        ).strip())
        assert score <= 2.0, f"exposure level {score} is above 2.0"

    with subtest("1. the initial owner can log in"):
        machine.fail(f"curl -fsS -X POST {base}/api/v1/auth/login -H 'Content-Type: application/json' "
                     "--data '{\"username\":\"owner\",\"password\":\"wrong password\"}'")
        api("POST", "/api/v1/auth/login", body={"username": "owner", "password": "vm-test-password"}, expect=200)
        _, me = api("GET", "/api/v1/me", expect=200)
        assert me["kind"] == "user" and me["name"] == "owner", me

    with subtest("2. keys: upload, read and a key that hides #secret"):
        def mint(name, **kw):
            _, k = api("POST", "/api/v1/keys", body={"name": name, **kw}, expect=201)
            return k["key"]
        upload_key = mint("uploader", scopes=["upload", "read", "tag"])
        read_key = mint("reader", preset="read_only")
        hidden_key = mint("friend", preset="read_only", deny_tags=["secret"])

    with subtest("3. quick add: H.264 and smaller than the source"):
        machine.succeed(
            "ffmpeg -v error -f lavfi -i 'testsrc=size=320x240:rate=10' -t 3 -y /tmp/fixture.gif"
        )
        src_size = int(machine.succeed("stat -c %s /tmp/fixture.gif").strip())
        _, gif = api("POST", "/api/v1/gifs?wait=60", token=upload_key, ctype="image/gif",
                     data_file="/tmp/fixture.gif", expect=201)
        assert gif["status"] == "ready", gif
        gid = gif["id"]
        machine.succeed(f"curl -fsS -o /tmp/full.mp4 {base}/m/{gid}/full.mp4")
        assert probe("/tmp/full.mp4", "codec_name,pix_fmt")["codec_name"] == "h264"
        mp4_size = int(machine.succeed("stat -c %s /tmp/full.mp4").strip())
        assert mp4_size < src_size, f"{mp4_size} is not smaller than {src_size}"
        machine.succeed(f"curl -fsS -r 0-99 -o /tmp/range -w '%{{http_code}}' {base}/m/{gid}/full.mp4 | grep -qx 206")

    with subtest("4. it is Unsorted until tagged"):
        _, listing = api("GET", "/api/v1/gifs?unsorted=true", token=upload_key, expect=200)
        assert [g["id"] for g in listing["items"]] == [gid], listing
        api("POST", f"/api/v1/gifs/{gid}/tags", token=upload_key, body={"add": ["test", "secret"]}, expect=200)
        _, listing = api("GET", "/api/v1/gifs?unsorted=true", token=upload_key, expect=200)
        assert listing["items"] == [], listing
        _, listing = api("GET", "/api/v1/gifs?q=test", token=upload_key, expect=200)
        assert [g["id"] for g in listing["items"]] == [gid], listing

    with subtest("5. full.gif is converted and decodes"):
        machine.succeed(f"curl -fsS -o /tmp/full.gif {base}/m/{gid}/full.gif")
        assert probe("/tmp/full.gif", "codec_name")["codec_name"] == "gif"
        machine.succeed("ffmpeg -v error -i /tmp/full.gif -f null -")
        machine.succeed("test -n \"$(ls -A /var/cache/unigif)\"")

    with subtest("6. scopes and deny_tags"):
        api("POST", "/api/v1/gifs?wait=1", token=read_key, ctype="image/gif",
            data_file="/tmp/fixture.gif", expect=403)
        _, listing = api("GET", "/api/v1/gifs?q=test", token=read_key, expect=200)
        assert [g["id"] for g in listing["items"]] == [gid], listing
        _, listing = api("GET", "/api/v1/gifs", token=hidden_key, expect=200)
        assert listing["items"] == [], listing
        api("GET", f"/api/v1/gifs/{gid}", token=hidden_key, expect=404)

    with subtest("restarting the service keeps everything"):
        machine.systemctl("restart unigif.service")
        machine.wait_for_unit("unigif.service")
        machine.wait_for_open_port(8787)
        # Sessions and keys live in the database, so the same cookie and key still work.
        api("GET", "/api/v1/me", expect=200)
        _, listing = api("GET", "/api/v1/gifs?q=test", token=read_key, expect=200)
        assert [g["id"] for g in listing["items"]] == [gid], listing
        machine.succeed(f"curl -fsS -o /dev/null {base}/m/{gid}/full.mp4")

    with subtest("the backup timer's job writes a consistent snapshot"):
        machine.succeed("systemctl list-timers unigif-backup.timer --no-pager | grep -q unigif-backup")
        machine.succeed("systemctl start unigif-backup.service")
        snap = machine.succeed("ls /var/lib/unigif/backups/unigif-*.db").split()[-1]
        check = machine.succeed(f"${pkgs.sqlite}/bin/sqlite3 {snap} 'PRAGMA integrity_check'").strip()
        assert check == "ok", check
        n = machine.succeed(f"${pkgs.sqlite}/bin/sqlite3 {snap} 'SELECT count(*) FROM gifs'").strip()
        assert n == "1", n

    with subtest("restore: stop, replace the database with the snapshot, start"):
        machine.systemctl("stop unigif.service")
        machine.succeed("rm -f /var/lib/unigif/unigif.db /var/lib/unigif/unigif.db-wal /var/lib/unigif/unigif.db-shm")
        machine.succeed(f"install -o unigif -g unigif -m 600 {snap} /var/lib/unigif/unigif.db")
        machine.systemctl("start unigif.service")
        machine.wait_for_unit("unigif.service")
        machine.wait_for_open_port(8787)
        _, listing = api("GET", "/api/v1/gifs?q=test", token=read_key, expect=200)
        assert [g["id"] for g in listing["items"]] == [gid], listing
        machine.succeed(f"curl -fsS -o /dev/null {base}/m/{gid}/full.mp4")

    with subtest("unigif-ctl runs the CLI as the service user"):
        machine.succeed("unigif-ctl doctor")
        out = machine.succeed("unigif-ctl key list")
        assert "uploader" in out, out
  '';
}
