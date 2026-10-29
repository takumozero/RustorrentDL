//! Command-line options and their corresponding qBittorrent form fields.
use crate::AppResult;
use std::time::Duration;

pub const HELP: &str = "Usage: RustorrentDL [OPTIONS] <TORRENT> [SAVE_PATH]

TORRENT can be a local .torrent file, magnet link, or torrent URL.
SAVE_PATH is the legacy positional alternative to --save-path.

Options:
  --save-path PATH          Download folder on the qBittorrent server
  --category NAME           Assign a qBittorrent category
  --tags TAGS               Comma-separated tags (e.g. linux,iso)
  --rename NAME             Set the torrent's display name
  --download-limit KIB_S    Download limit in KiB/s (0 = unlimited)
  --upload-limit KIB_S      Upload limit in KiB/s (0 = unlimited)
  --poll-interval SECONDS   Progress update interval, 1..3600 (default: 2)
  --no-wait                Submit and exit without monitoring
  --login                  Use QBITTORRENT_USERNAME and QBITTORRENT_PASSWORD
  -h, --help               Show this help
  -V, --version            Show program version
  --                       Treat remaining arguments as positional values

Omitted torrent options use qBittorrent defaults. An explicit save path
disables Automatic Torrent Management for this torrent so the path is honored.
Authentication: QBITTORRENT_API_KEY or QBITTORRENT_SID, also supported in .env.
With --login, username/password authentication overrides API key and SID.
";

pub struct DownloadRequest {
    pub source: String,
    pub save_path: Option<String>,
    pub category: Option<String>,
    pub tags: Option<String>,
    pub rename: Option<String>,
    pub download_limit: Option<u64>,
    pub upload_limit: Option<u64>,
}

pub struct Options {
    pub download: DownloadRequest,
    pub poll_interval: Duration,
    pub wait: bool,
    pub login: bool,
}

/// Parse separately from environment loading so --help never needs credentials.
pub fn parse(args: impl IntoIterator<Item = String>) -> AppResult<Option<Options>> {
    let mut args = args.into_iter();
    let mut download = DownloadRequest {
        source: String::new(),
        save_path: None,
        category: None,
        tags: None,
        rename: None,
        download_limit: None,
        upload_limit: None,
    };
    let mut poll_interval = Duration::from_secs(2);
    let mut wait = true;
    let mut login = false;
    let mut positional = Vec::new();
    let mut positional_only = false;
    while let Some(arg) = args.next() {
        if positional_only {
            positional.push(arg);
            continue;
        }
        if arg == "--" {
            positional_only = true;
            continue;
        }
        if matches!(arg.as_str(), "--help" | "-h") {
            return Ok(None);
        }
        if arg == "--login" {
            login = true;
            continue;
        }
        if arg == "--no-wait" {
            wait = false;
            continue;
        }
        if !arg.starts_with('-') {
            positional.push(arg);
            continue;
        }
        let (option, inline) = arg
            .split_once('=')
            .map_or((arg.as_str(), None), |(k, v)| (k, Some(v)));
        if !matches!(
            option,
            "--save-path"
                | "--category"
                | "--tags"
                | "--rename"
                | "--download-limit"
                | "--upload-limit"
                | "--poll-interval"
        ) {
            return Err(format!("Unknown option {option}; use --help").into());
        }
        let value = match inline {
            Some(value) => value.to_owned(),
            None => args
                .next()
                .filter(|s| !s.starts_with("--"))
                .ok_or_else(|| format!("{option} requires a value"))?,
        };
        if value.is_empty() {
            return Err(format!("{option} requires a nonempty value").into());
        }
        match option {
            "--save-path" => download.save_path = Some(value),
            "--category" => download.category = Some(value),
            "--tags" => download.tags = Some(value),
            "--rename" => download.rename = Some(value),
            "--download-limit" => download.download_limit = Some(parse_speed(&value)?),
            "--upload-limit" => download.upload_limit = Some(parse_speed(&value)?),
            "--poll-interval" => {
                let seconds: u64 = value
                    .parse()
                    .map_err(|_| "--poll-interval must be an integer from 1 to 3600")?;
                if !(1..=3600).contains(&seconds) {
                    return Err("--poll-interval must be from 1 to 3600 seconds".into());
                }
                poll_interval = Duration::from_secs(seconds);
            }
            _ => unreachable!(),
        }
    }
    if positional.is_empty() || positional.len() > 2 {
        return Err("Provide a torrent and optionally a save path; use --help for options".into());
    }
    download.source = positional.remove(0);
    if let Some(path) = positional.pop() {
        if download.save_path.is_some() {
            return Err("Use either --save-path or a positional save path, not both".into());
        }
        download.save_path = Some(path);
    }
    Ok(Some(Options {
        download,
        poll_interval,
        wait,
        login,
    }))
}

fn parse_speed(value: &str) -> AppResult<u64> {
    let kib: u64 = value
        .parse()
        .map_err(|_| "Speed limits must be nonnegative whole numbers in KiB/s")?;
    // The API uses a signed byte count; avoid overflow during conversion.
    kib.checked_mul(1024)
        .filter(|bytes| *bytes <= i64::MAX as u64)
        .ok_or_else(|| "Speed limit is too large".into())
}

impl DownloadRequest {
    /// Map download options to qBittorrent API form fields.
    pub fn form_fields(&self) -> Vec<(&'static str, String)> {
        let mut fields = Vec::new();
        for (name, value) in [
            ("savepath", &self.save_path),
            ("category", &self.category),
            ("tags", &self.tags),
            ("rename", &self.rename),
        ] {
            if let Some(value) = value {
                fields.push((name, value.clone()));
            }
        }
        if self.save_path.is_some() {
            fields.push(("autoTMM", "false".into()));
        }
        for (name, value) in [
            ("dlLimit", self.download_limit),
            ("upLimit", self.upload_limit),
        ] {
            if let Some(value) = value {
                fields.push((name, value.to_string()));
            }
        }
        fields
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn options(args: &[&str]) -> AppResult<Option<Options>> {
        parse(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn parses_options_and_maps_api_fields() {
        let parsed = options(&[
            "file.torrent",
            "--save-path",
            "C:\\Downloads",
            "--category",
            "linux",
            "--tags=iso,test",
            "--rename",
            "Example",
            "--download-limit",
            "1024",
            "--upload-limit",
            "0",
            "--poll-interval",
            "5",
            "--no-wait",
        ])
        .unwrap()
        .unwrap();
        assert!(!parsed.wait);
        assert_eq!(parsed.poll_interval.as_secs(), 5);
        let fields = parsed.download.form_fields();
        for pair in [
            ("dlLimit", "1048576"),
            ("upLimit", "0"),
            ("autoTMM", "false"),
            ("category", "linux"),
            ("tags", "iso,test"),
            ("rename", "Example"),
            ("savepath", "C:\\Downloads"),
        ] {
            assert!(fields.contains(&(pair.0, pair.1.into())));
        }
    }

    #[test]
    fn defaults_and_legacy_path_work() {
        let parsed = options(&["file.torrent"]).unwrap().unwrap();
        assert!(parsed.wait);
        assert!(!parsed.login);
        assert!(
            options(&["file.torrent", "--login"])
                .unwrap()
                .unwrap()
                .login
        );
        assert_eq!(parsed.poll_interval.as_secs(), 2);
        assert!(parsed.download.form_fields().is_empty());
        assert_eq!(
            options(&["file.torrent", "dest"])
                .unwrap()
                .unwrap()
                .download
                .save_path
                .as_deref(),
            Some("dest")
        );
        assert!(options(&["--help"]).unwrap().is_none());
        assert_eq!(
            options(&["--", "-file.torrent"])
                .unwrap()
                .unwrap()
                .download
                .source,
            "-file.torrent"
        );
    }

    #[test]
    fn rejects_bad_options_before_submission() {
        for args in [
            vec!["file.torrent", "--poll-interval", "0"],
            vec!["file.torrent", "--download-limit", "-1"],
            vec!["file.torrent", "--download-limit", "18446744073709551615"],
            vec!["file.torrent", "dest", "--save-path", "other"],
            vec!["file.torrent", "--tags"],
            vec!["file.torrent", "--unknown"],
            vec!["file.torrent", "--category="],
        ] {
            assert!(options(&args).is_err());
        }
    }
}
