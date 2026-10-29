# RustorrentDL

A small Rust CLI that submits a torrent to qBittorrent's Web API and tracks its download until completion. It accepts local `.torrent` files, magnet links, and HTTP(S) torrent URLs.

qBittorrent downloads the content on its own machine. This CLI uploads torrent metadata or submits a link; it does not download the content itself.

## Table of contents

- [Requirements](#requirements)
- [Using a precompiled release executable](#using-a-precompiled-release-executable)
- [Quick start from source](#quick-start-from-source)
- [Build and run](#build-and-run)
- [Configuration and authentication](#configuration-and-authentication)
  - [Username/password authentication](#usernamepassword-authentication)
  - [SID authentication](#sid-authentication)
  - [Server URLs and credentials](#server-urls-and-credentials)
- [Usage](#usage)
- [Examples](#examples)
- [Progress display](#progress-display)
- [Response handling and limitations](#response-handling-and-limitations)
- [Diagnostics and troubleshooting](#diagnostics-and-troubleshooting)
- [Source layout and verification](#source-layout-and-verification)
  - [CLI smoke tests](#cli-smoke-tests)
- [API references](#api-references)

## Requirements

- To build from source: Rust and Cargo with support for Rust edition 2024. Install a current stable toolchain from [rustup](https://rustup.rs/). A precompiled release executable does not require Rust or Cargo.
- A running qBittorrent instance with its Web UI enabled and reachable.
- For API key authentication: qBittorrent **5.2.0+**, or WebAPI **2.14.1+**.
- Progress monitoring requires a submission response containing `added_torrent_ids`. Older servers can use SID authentication and `--no-wait` if they do not return those IDs.

Examples below use PowerShell. For source builds, run them from the project directory:

```powershell
Set-Location '.\RustorrentDL'
```

## Using a precompiled release executable

You do not need Rust, Cargo, the source code, or `Cargo.toml` to run a release executable. You still need a running qBittorrent instance with its Web UI enabled.

1. Open this repository's **Releases** page on GitHub.
2. Select a release and expand **Assets**. Download the Windows executable or archive matching your system, rather than the automatically generated source-code archives.
3. If the asset is an archive, extract it. Place `RustorrentDL.exe` in a folder such as `C:\Tools\RustorrentDL`. If the downloaded executable has a different name, use that name in the commands below.
4. If not already included in the extracted archive, create a file named `.env` in that folder. If `.env.example` is included with the release, copy it to `.env` and fill in your own values.

For API key authentication, use:

```dotenv
QBITTORRENT_URL=http://localhost:8080
QBITTORRENT_API_KEY=replace_with_your_generated_key
```

Generate the full key under **qBittorrent Preferences → WebUI → API Key**. API key authentication requires qBittorrent 5.2.0+ (WebAPI 2.14.1+).

Alternatively, configure username/password authentication:

```dotenv
QBITTORRENT_URL=http://localhost:8080
QBITTORRENT_USERNAME=your_webui_username
QBITTORRENT_PASSWORD="your_webui_password"
```

Your folder should look like this:

```text
C:\Tools\RustorrentDL\
├── RustorrentDL.exe
└── .env
```

Open PowerShell and change to that folder before running the executable:

```powershell
Set-Location 'C:\Tools\RustorrentDL'
.\RustorrentDL.exe --help
.\RustorrentDL.exe 'C:\Torrents\example.torrent' --save-path 'C:\Downloads'
```

When using username/password credentials, add `--login`:

```powershell
.\RustorrentDL.exe 'C:\Torrents\example.torrent' --login --save-path 'C:\Downloads'
```

All CLI options work in the release executable. For example:

```powershell
.\RustorrentDL.exe 'magnet:?xt=urn:btih:YOUR_INFO_HASH' --tags 'linux,iso' --download-limit 5120 --poll-interval 5
.\RustorrentDL.exe 'C:\Torrents\example.torrent' --no-wait
```

Use `--login` in these commands too if you configured username/password authentication. Do not add Cargo's `--` separator when launching the executable directly.

**The `.env` file is not embedded in the executable.** The program searches the current working directory and its parents, not automatically the executable's folder. Running the executable by its full path from another folder does not change the working directory. For shortcuts, set **Start in** to the folder containing `.env`. For scheduled jobs or scripts, set the working directory explicitly. Existing environment variables override `.env` values.

Use a terminal to see progress and errors; double-clicking the executable provides no torrent argument and its console may close immediately. Downloaded content is saved on the qBittorrent server, so `--save-path` must be valid there (or inside its container).

The program exits with code `0` when all tracked downloads finish, or when a `--no-wait` submission is accepted. Check the result in PowerShell immediately after it exits:

```powershell
$LASTEXITCODE
```

Errors return a nonzero code. Ctrl+C ends monitoring while qBittorrent continues running. Keep `.env` private when sharing or replacing the executable. To update, stop the CLI, replace the executable with the new release, and retain your local `.env`.

## Quick start from source

1. Enable the Web UI in qBittorrent's settings and note its address and port.
2. Generate an API key under **Preferences → WebUI → API Key**.
3. Create a file named `.env` beside `Cargo.toml`:

   ```dotenv
   QBITTORRENT_URL=http://localhost:8080
   QBITTORRENT_API_KEY=replace_with_your_generated_key
   ```

   Use the full generated key, including its `qbt_` prefix. The placeholder above is not a valid key.

4. Submit a local torrent:

   ```powershell
   cargo run -- 'C:\Torrents\example.torrent' --save-path 'C:\Downloads'
   ```

The CLI prints a submission summary, then progress updates until completion. Existing downloads continue if you close the CLI or press Ctrl+C.

## Build and run

```powershell
cargo build --release
Copy-Item '.env.example' '.\target\release\.env'
# Edit target\release\.env and fill in your server address and credentials.
Set-Location '.\target\release'
.\RustorrentDL.exe --help
.\RustorrentDL.exe 'C:\Torrents\example.torrent'
```

For a portable build, include `.env` in the root of the executable folder, beside `RustorrentDL.exe`, and launch from that folder. Copy the executable and `.env` together when moving the build. The configuration stays separate from the executable; credentials must be filled in locally. If credentials are provided entirely through environment variables, `.env` is optional. See [troubleshooting](#diagnostics-and-troubleshooting) if configuration is not found.

For development, use `cargo run -- ...`. The first `--` belongs to Cargo and forwards the remaining arguments to this program. When running the executable directly, omit that Cargo separator.

## Configuration and authentication

| Variable | Purpose | Default |
| --- | --- | --- |
| `QBITTORRENT_URL` | Base URL of qBittorrent's Web UI | `http://localhost:8080/` |
| `QBITTORRENT_API_KEY` | API key sent as `Authorization: Bearer ...` | Unset |
| `QBITTORRENT_SID` | Legacy session cookie value | Unset |
| `QBITTORRENT_USERNAME` | Web UI username, used only with `--login` | Unset |
| `QBITTORRENT_PASSWORD` | Web UI password, used only with `--login` | Unset |

The program loads `.env` from the current working directory or a parent directory, not relative to the executable. Run from the project directory to use its `.env`. A missing `.env` is allowed; a malformed or unreadable file produces an error.

Existing environment variables take precedence over `.env`. Without `--login`, if `QBITTORRENT_API_KEY` is set, it takes precedence over `QBITTORRENT_SID`. An empty or invalid API key produces an error rather than falling back to SID authentication.

You can also configure the current PowerShell session directly:

```powershell
$env:QBITTORRENT_URL = 'http://localhost:8080'
$env:QBITTORRENT_API_KEY = 'replace_with_your_generated_key'
```

To let `.env` supply a value after previously setting it in PowerShell:

```powershell
Remove-Item Env:QBITTORRENT_API_KEY -ErrorAction SilentlyContinue
```

API keys consist of `qbt_` followed by 28 alphanumeric characters. Rotating the key in qBittorrent invalidates the previous key.

### Username/password authentication

Pass `--login` to explicitly select username/password authentication, ignoring any configured API key or SID. Add these values to your local `.env`:

```dotenv
QBITTORRENT_URL=http://localhost:8080
QBITTORRENT_USERNAME=your_webui_username
QBITTORRENT_PASSWORD="your_webui_password"
```

```powershell
# From source
cargo run -- '.\example.torrent' --login

# Portable executable
.\RustorrentDL.exe 'C:\Torrents\example.torrent' --login
```

The CLI logs in once through `/api/v2/auth/login` and reuses the returned session cookie for submission, file information, and progress. Both credentials are required; login failure does not fall back to API key authentication. Credentials can also come from environment variables, which retain precedence over `.env`. The session is kept in memory and is not written back to `.env`; expired sessions are not automatically refreshed. Requests and credentials are not printed.

Login supports legacy `200 Ok.` and modern empty success responses, including `204`, with `SID` or port-specific `QBT_SID_<port>` cookies. Use HTTPS for remote connections. Quote values containing spaces or `#` according to dotenv syntax.

### SID authentication

For a server without API key support, remove the API key entry and set:

```dotenv
QBITTORRENT_URL=http://localhost:8080
QBITTORRENT_SID=your_session_cookie_value
```

Obtain the SID cookie after logging in to the Web UI, or from the `Set-Cookie` header returned by `/api/v2/auth/login`. Supply only the value, without `SID=`. SID sessions expire; use `--login` to obtain a new session from username/password credentials. Sessions are not automatically refreshed during monitoring.

### Server URLs and credentials

Reverse-proxy prefixes such as `https://example.com/qbit/` are supported. Do not append `/api/v2` to the configured base URL. URLs must use HTTP or HTTPS and cannot contain embedded credentials, query parameters, or fragments.

Keep `.env` out of Git by including `.env` in `.gitignore`. It stores credentials as plain text. Use HTTPS across untrusted networks. TLS certificates are verified, and HTTP redirects are not followed. A proxy with separate authentication requires additional support beyond this CLI's qBittorrent credentials.

## Usage

```text
RustorrentDL [OPTIONS] <TORRENT> [SAVE_PATH]
```

`TORRENT` is required and can be a local file, magnet link, or torrent URL. The optional positional `SAVE_PATH` is retained for compatibility; use either that or `--save-path`, not both.

| Option | Description | Default |
| --- | --- | --- |
| `--save-path PATH` | Download folder on the qBittorrent server | qBittorrent's configured path |
| `--category NAME` | Assign a category | qBittorrent default |
| `--tags TAGS` | Comma-separated tags, such as `linux,iso` | qBittorrent default |
| `--rename NAME` | Set the torrent's display name | Original name |
| `--download-limit KIB_S` | Per-torrent download limit in KiB/s; `0` means unlimited | qBittorrent default |
| `--upload-limit KIB_S` | Per-torrent upload limit in KiB/s; `0` means unlimited | qBittorrent default |
| `--poll-interval SECONDS` | Progress update interval, from `1` to `3600` | `2` |
| `--login` | Use username/password credentials instead of API key or SID | Disabled |
| `--no-wait` | Validate submission, then exit without tracking | Track until completion |
| `-h`, `--help` | Print usage without requiring authentication | — |
| `-V`, `--version` | Print the program version | — |
| `--` | Treat all subsequent arguments as positional values | — |

Value options accept `--option value` or `--option=value`. Speed limits must be nonnegative whole numbers. For example, `5120` KiB/s is `5` MiB/s. Global qBittorrent limits can still constrain the effective speed.

An explicit save path sets `autoTMM=false` for that torrent so Automatic Torrent Management does not override the destination. All server paths refer to the machine running qBittorrent. For Docker, use paths inside the container, such as `/downloads`, rather than host-only paths. A local torrent file path refers to the machine running this CLI.

## Examples

Each example shows both source execution and the portable executable. Run source commands from the project directory; run portable commands from the folder containing `RustorrentDL.exe` and `.env`. With username/password credentials, add `--login` as shown below.

Local file with default download settings:

```powershell
# From source
cargo run -- '.\example.torrent'

# Portable executable
.\RustorrentDL.exe '.\example.torrent'
```

Magnet link and a server destination:

```powershell
# From source
cargo run -- 'magnet:?xt=urn:btih:YOUR_INFO_HASH' --save-path 'C:\Downloads'

# Portable executable
.\RustorrentDL.exe 'magnet:?xt=urn:btih:YOUR_INFO_HASH' --save-path 'C:\Downloads'
```

Quote magnet links so the shell preserves characters such as `&`.

Torrent URL with category and tags:

```powershell
# From source
cargo run -- 'https://example.com/example.torrent' --category 'linux' --tags 'iso,test'

# Portable executable
.\RustorrentDL.exe 'https://example.com/example.torrent' --category 'linux' --tags 'iso,test'
```

Use a category configured on your qBittorrent server. The CLI does not create categories explicitly.

Rename, limit speeds, and update progress every five seconds:

```powershell
# From source
cargo run -- '.\example.torrent' --rename 'Example download' --download-limit 5120 --upload-limit 1024 --poll-interval 5

# Portable executable
.\RustorrentDL.exe '.\example.torrent' --rename 'Example download' --download-limit 5120 --upload-limit 1024 --poll-interval 5
```

Submit without waiting:

```powershell
# From source
cargo run -- '.\example.torrent' --no-wait

# Portable executable
.\RustorrentDL.exe '.\example.torrent' --no-wait
```

Legacy positional destination:

```powershell
# From source
cargo run -- '.\example.torrent' 'C:\Downloads'

# Portable executable
.\RustorrentDL.exe '.\example.torrent' 'C:\Downloads'
```

A filename beginning with a hyphen needs the program's own positional separator:

```powershell
# From source
cargo run -- -- '-example.torrent'

# Portable executable
.\RustorrentDL.exe -- '-example.torrent'
```

Username/password authentication:

```powershell
# From source
cargo run -- 'C:\Torrents\example.torrent' --login --save-path 'C:\Downloads'

# Portable executable
.\RustorrentDL.exe 'C:\Torrents\example.torrent' --login --save-path 'C:\Downloads'
```

## Progress display

```text
"example":  45.20% | 3.15 MiB/s | ETA 0h 02m 10s | downloading
  Total: 2.00 GiB | Selected: 2.00 GiB | Downloaded: 925.70 MiB | Remaining: 1.10 GiB | Files: 24
```

- **Total:** size of all files in the torrent, including unselected files.
- **Selected:** size of the files selected for download.
- **Downloaded:** selected size minus bytes remaining; this is not cumulative network traffic.
- **Remaining:** bytes still needed for the selected content.
- **Files:** number of file entries in the torrent, including unselected files; folders are not counted separately. The count is fetched after metadata becomes available and cached.
- **Speed / ETA / state:** values reported by qBittorrent. Unknown ETA is shown as `unknown`.

Sizes use binary units: 1 KiB = 1024 bytes, 1 MiB = 1024 KiB. Size and file count can be unknown while a magnet link is fetching metadata.

The monitor tracks only the IDs returned by this submission. It waits through metadata fetching, checking, allocation, and moving. Completion requires reported progress of 100% and zero bytes remaining outside those transient states. Completion refers to selected content, not necessarily every file in a selectively downloaded torrent.

Paused, queued, and stalled downloads remain monitored. Resume paused downloads in qBittorrent. Monitoring exits on reported error/missing files/unknown state, a torrent disappearing after being seen, or a torrent not appearing within 60 seconds. Network or API errors also end monitoring; the download may still be running on the server.

After completion, the CLI exits without changing qBittorrent's seeding settings. Ctrl+C stops monitoring without stopping the torrent.

## Response handling and limitations

Structured submission responses use `success_count`, `pending_count`, and `failure_count`. Any reported failure produces an error, including partial failures. A submission reporting neither success nor pending work is also treated as an error.

Legacy empty success bodies and `Ok.` are accepted; `Fails.` is rejected. However, monitoring additionally needs nonempty, valid `added_torrent_ids`. Pending results without IDs, or responses containing any pending count, cannot currently be monitored. Use the Web UI for those submissions. `--no-wait` skips the torrent-ID requirement.

Submitting an existing torrent may not create another download or return an ID that can be monitored. The program does not currently attach to arbitrary existing downloads or resume monitoring by a supplied hash. Before retrying after an error, check whether the torrent was already added.

## Diagnostics and troubleshooting

Submission summaries and progress are printed to stdout. Errors and file-count warnings are printed to stderr. HTTP requests, headers, multipart fields, and raw response bodies are not printed.

| Symptom | What to check |
| --- | --- |
| Usage error | Supply a torrent argument; use `cargo run -- --help` from source or `.\RustorrentDL.exe --help` for the portable build. |
| Missing/invalid API key | Use the complete generated key; check whether a shell variable overrides `.env`. |
| HTTP 401/403 | Check credentials, API key support, Web UI access restrictions, and proxy settings. |
| HTTP 415 | Check that the uploaded file is a valid `.torrent`. |
| Connection refused or timeout | Check server address/port, Web UI availability, firewall, and connectivity. HTTP requests time out after 30 seconds. |
| TLS certificate error | Configure a certificate trusted by the machine running the CLI. |
| `.env` not loaded | For the portable build, put `.env` in the root of the executable folder beside `RustorrentDL.exe` and change to that folder before launching it. For source execution, use the project directory. Confirm the filename is `.env`, not `.env.txt`. The search starts at the working directory, not the executable location. |
| EXE works in one folder but not from a shortcut or script | Set the shortcut's **Start in** or the script/job working directory to the folder containing the EXE and `.env`. When moving the EXE, include `.env` as well; it is not embedded in the binary. |
| Download starts but tracking reports no IDs | Inspect the response; use the Web UI or `--no-wait`. Do not blindly resubmit. |
| File count unknown | Wait for metadata; file-list failures are logged and retried without ending progress monitoring. |
| Wrong destination | Use a server/container path and confirm qBittorrent can write there. |
| No progress | Inspect the displayed state and Web UI for paused downloads, queueing, unavailable peers, disk issues, or limits. |

Successful completion or accepted `--no-wait` submission exits with code `0`. Other errors exit nonzero. An error does not roll back or stop an already submitted torrent.

## Source layout and verification

| File | Responsibility |
| --- | --- |
| `src/main.rs` | Startup, `.env`, URL/authentication setup, multipart submission, response validation |
| `src/login.rs` | Username/password login and session cookie extraction |
| `src/cli.rs` | Argument parsing, help, input validation, mapping options to API fields |
| `src/progress.rs` | Torrent ID extraction, progress polling, file count, byte/ETA formatting, completion detection |
| `Cargo.toml` / `Cargo.lock` | Dependencies and resolved versions |

Dependencies: `reqwest` for blocking HTTP/multipart/TLS, `dotenvy` for `.env`, and `serde_json` for API responses.

```powershell
cargo fmt --check
cargo test
```

Tests cover CLI defaults/options/validation, API field mapping, submission responses, ID validation, size formatting, completion-state handling, and polling a local mock API through completion. They do not contact your live qBittorrent instance or download torrent content.

### CLI smoke tests

With Python 3 installed (standard library only), build the debug executable and run:

```powershell
cargo build
python tests/cli_smoke.py
```

The smoke suite launches a local mock API and checks every CLI parameter, argument validation, file uploads, magnet/URL submission, login override, progress completion, exit codes, and absence of request/response debug dumps. Synthetic fixtures are created in a temporary directory and removed afterward. No payload downloads are started.

To additionally verify the credentials in your `.env` against your live qBittorrent instance:

```powershell
python tests/cli_smoke.py --live
```

The optional live checks perform username/password login and default authentication, then submit empty or invalid torrent files and expect HTTP 415 rejection. They do not test real payload transfer or successful live torrent completion. Credentials and session cookies are not printed. This Windows smoke harness runs `target/debug/RustorrentDL.exe`.

## API references

- [qBittorrent WebUI API](https://github.com/qbittorrent/qBittorrent/wiki/WebUI-API-%28qBittorrent-5.0%29)
- [API key authentication](https://github.com/qbittorrent/qBittorrent/wiki/API-Key-Authentication-%28%E2%89%A5v5.2.0%29)
- [dotenvy](https://docs.rs/dotenvy/latest/dotenvy/)
