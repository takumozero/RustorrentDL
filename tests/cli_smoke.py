"""CLI smoke tests using synthetic metadata and a local mock API; no payload downloads.

Run: python tests/cli_smoke.py [--live]
--live also checks configured authentication against invalid local uploads.
"""
import argparse
import email.parser
import email.policy
import http.server
import json
import os
from pathlib import Path
import subprocess
import tempfile
import threading

ROOT = Path(__file__).resolve().parents[1]
EXE = ROOT / "target/debug/RustorrentDL.exe"
HASH = "0123456789abcdef0123456789abcdef01234567"
EVENTS = []


class Mock(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def reply(self, value, status=200, cookie=None):
        body = value if isinstance(value, bytes) else json.dumps(value).encode()
        self.send_response(status)
        self.send_header("Content-Length", str(len(body)))
        if cookie:
            self.send_header("Set-Cookie", cookie)
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self):
        body = self.rfile.read(int(self.headers.get("Content-Length", "0")))
        if self.path == "/api/v2/auth/login":
            assert b"username=smoke-user" in body
            assert b"password=smoke-password" in body
            assert self.headers.get("Authorization") is None
            EVENTS.append(("login", {}))
            self.reply(b"", 204, "QBT_SID_8080=smoke123; path=/; HttpOnly")
            return
        assert self.path == "/api/v2/torrents/add"
        assert (self.headers.get("Authorization") == "Bearer qbt_" + "A" * 28
                or self.headers.get("Cookie") in ("QBT_SID_8080=smoke123", "SID=smoke123"))
        message = email.parser.BytesParser(policy=email.policy.default).parsebytes(
            b"Content-Type: " + self.headers["Content-Type"].encode() + b"\r\n\r\n" + body)
        fields = {}
        file_data = None
        for part in message.iter_parts():
            name = part.get_param("name", header="content-disposition")
            value = part.get_payload(decode=True)
            if name == "torrents":
                file_data = value
            else:
                fields[name] = value.decode()
        EVENTS.append(("add", fields))
        if file_data is not None and not file_data.startswith(b"d4:info"):
            self.reply(b"", 415)
        else:
            self.reply(dict(added_torrent_ids=[HASH], failure_count=0, pending_count=0, success_count=1))

    def do_GET(self):
        EVENTS.append(("get", self.path))
        if self.path.startswith("/api/v2/torrents/files?"):
            self.reply([dict(name="empty.txt", size=0)])
        else:
            assert self.path == "/api/v2/torrents/info?hashes=" + HASH
            self.reply([dict(hash=HASH, name="empty-fixture", progress=1.0, amount_left=0,
                             size=0, total_size=0, state="stalledUP", eta=0, dlspeed=0)])


def run(label, args, environment, success=True, text=None):
    result = subprocess.run([str(EXE), *map(str, args)], cwd=ROOT, env=environment,
                            capture_output=True, text=True, timeout=40)
    output = result.stdout + result.stderr
    assert (result.returncode == 0) == success, label + ": unexpected exit code " + str(result.returncode)
    if text:
        assert text in output, label + ": expected output missing"
    assert "--- Request" not in output and "--- Response" not in output, label + ": debug dump found"
    print("PASS:", label, "(exit", str(result.returncode) + ")")
    return output


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--live", action="store_true")
    args = parser.parse_args()
    assert EXE.exists(), "Run cargo build first"
    with tempfile.TemporaryDirectory(prefix="qbit-smoke-") as temporary:
        directory = Path(temporary)
        empty = directory / "empty.torrent"
        empty.write_bytes(b"")
        invalid = directory / "invalid.torrent"
        invalid.write_text("This is not torrent metadata.")
        # A bencoded zero-byte file. Used only by the mock; no payload exists.
        fixture = directory / "zero-byte.torrent"
        fixture.write_bytes(b"d4:infod6:lengthi0e4:name9:empty.txt12:piece lengthi16384e6:pieces0:ee")
        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Mock)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        environment = os.environ.copy()
        environment.update(QBITTORRENT_URL="http://127.0.0.1:" + str(server.server_port),
                           QBITTORRENT_API_KEY="qbt_" + "A" * 28,
                           QBITTORRENT_USERNAME="smoke-user", QBITTORRENT_PASSWORD="smoke-password")
        try:
            run("long help", ["--help"], environment, text="--login")
            run("short help", ["-h"], environment, text="--save-path")
            run("local file and full completion", [fixture], environment, text="All downloads completed")
            output = run("all value options and login override", [fixture, "--login", "--save-path", "C:\\Test Downloads",
                "--category", "test", "--tags", "one,two", "--rename", "Smoke test", "--download-limit", "5120",
                "--upload-limit", "0", "--poll-interval", "1", "--no-wait"], environment, text="Submission accepted")
            assert "smoke-password" not in output
            fields = [value for kind, value in EVENTS if kind == "add"][-1]
            assert fields == dict(savepath="C:\\Test Downloads", category="test", tags="one,two",
                                  rename="Smoke test", dlLimit="5242880", upLimit="0", autoTMM="false")
            assert any(kind == "login" for kind, _ in EVENTS)
            run("legacy positional destination", [fixture, "C:\\Downloads", "--no-wait"], environment)
            run("magnet link", ["magnet:?xt=urn:btih:" + HASH, "--no-wait"], environment)
            run("torrent URL", ["https://example.invalid/test.torrent", "--no-wait"], environment)
            run("equals syntax", [fixture, "--tags=one,two", "--poll-interval=3", "--no-wait"], environment)
            run("positional separator", ["--no-wait", "--", fixture], environment)
            run("empty upload rejected", [empty, "--no-wait"], environment, False, "415")
            run("text upload rejected", [invalid, "--no-wait"], environment, False, "415")
            run("missing local file", [directory / "missing.torrent"], environment, False, "Could not open")
            for label, options in [
                ("missing torrent", []), ("unknown option", [fixture, "--bad"]),
                ("zero polling interval", [fixture, "--poll-interval", "0"]),
                ("negative speed", [fixture, "--upload-limit", "-1"]),
                ("missing option value", [fixture, "--tags"]),
                ("conflicting paths", [fixture, "dest", "--save-path", "other"]),
            ]:
                before = len(EVENTS)
                run(label, options, environment, False)
                assert len(EVENTS) == before, label + ": unexpectedly contacted API"
        finally:
            server.shutdown()
            server.server_close()
            thread.join()
        if args.live:
            # The actual CLI loads the user's .env itself. No secrets are read or printed here.
            live = os.environ.copy()
            run("LIVE username/password + empty upload rejected", [empty, "--login", "--no-wait"], live, False, "qBittorrent returned HTTP 415")
            run("LIVE username/password + invalid text rejected", [invalid, "--login", "--no-wait"], live, False, "qBittorrent returned HTTP 415")
            run("LIVE default API authentication + empty upload rejected", [empty, "--no-wait"], live, False, "qBittorrent returned HTTP 415")
    print("All smoke checks passed; temporary fixtures removed. No payload downloads were started.")


if __name__ == "__main__":
    main()

