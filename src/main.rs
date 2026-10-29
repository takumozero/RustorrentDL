use reqwest::{
    Url,
    blocking::{Client, Response, multipart::Form},
    header::{AUTHORIZATION, COOKIE, HeaderMap, HeaderValue, REFERER},
    redirect::Policy,
};
use std::{env, error::Error, time::Duration};

type AppResult<T> = Result<T, Box<dyn Error>>;

mod cli;
mod login;
mod progress;
use cli::DownloadRequest;

const DEFAULT_URL: &str = "http://localhost:8080/";
const ADD_TORRENT_ENDPOINT: &str = "api/v2/torrents/add";

fn main() -> AppResult<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.len() == 1 && matches!(args[0].as_str(), "--version" | "-V") {
        println!("RustorrentDL {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let Some(options) = cli::parse(args)? else {
        print!("{}", cli::HELP);
        return Ok(());
    };
    load_dotenv()?;
    let base_url = load_base_url()?;
    let client = build_http_client()?;
    let authentication = if options.login {
        login::from_environment(&client, &base_url)?
    } else {
        load_authentication()?
    };

    let response = submit_torrent(&client, &base_url, authentication.clone(), options.download)?;
    let body = check_submission(response)?;
    if options.wait {
        let torrent_ids = progress::added_torrent_ids(&body)?;
        progress::track_downloads(
            &client,
            &base_url,
            &authentication,
            &torrent_ids,
            options.poll_interval,
        )?;
    } else {
        println!("Submission accepted. qBittorrent will continue the download.");
    }
    Ok(())
}

/// Load optional .env settings before reading configuration.
fn load_dotenv() -> AppResult<()> {
    match dotenvy::dotenv() {
        Ok(_) => Ok(()),
        Err(dotenvy::Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        // Avoid displaying parse errors, which could contain a secret value.
        Err(_) => Err("Could not load .env; check its syntax and file permissions".into()),
    }
}

/// Validate the server URL and preserve any reverse-proxy path prefix.
fn load_base_url() -> AppResult<Url> {
    let value = env::var("QBITTORRENT_URL").unwrap_or_else(|_| DEFAULT_URL.into());
    let mut url = Url::parse(&value)?;
    let is_http = matches!(url.scheme(), "http" | "https");
    let has_credentials = !url.username().is_empty() || url.password().is_some();
    let has_query_or_fragment = url.query().is_some() || url.fragment().is_some();

    if !is_http || url.host_str().is_none() || has_credentials || has_query_or_fragment {
        return Err(
            "QBITTORRENT_URL must be an HTTP(S) base URL without credentials, query, or fragment"
                .into(),
        );
    }

    // A trailing slash makes URL joining append to the proxy path.
    if !url.path().ends_with('/') {
        url.set_path(&format!("{}/", url.path()));
    }
    Ok(url)
}

/// Prefer an API key (qBittorrent 5.2+), falling back to a session cookie.
fn load_authentication() -> AppResult<HeaderMap> {
    let mut headers = HeaderMap::new();
    match env::var("QBITTORRENT_API_KEY") {
        Ok(key) => {
            if key.len() != 32
                || !key.starts_with("qbt_")
                || !key[4..].bytes().all(|b| b.is_ascii_alphanumeric())
            {
                return Err(
                    "QBITTORRENT_API_KEY must be qbt_ followed by 28 alphanumeric characters"
                        .into(),
                );
            }
            let mut value = HeaderValue::from_str(&format!("Bearer {key}"))?;
            value.set_sensitive(true);
            headers.insert(AUTHORIZATION, value);
        }
        Err(env::VarError::NotPresent) => {
            headers.insert(COOKIE, load_session_cookie()?);
        }
        Err(_) => return Err("QBITTORRENT_API_KEY must contain valid Unicode".into()),
    }
    Ok(headers)
}

/// Legacy authentication for servers without API key support.
fn load_session_cookie() -> AppResult<HeaderValue> {
    let sid = env::var("QBITTORRENT_SID")
        .map_err(|_| "Set QBITTORRENT_API_KEY, or QBITTORRENT_SID for session authentication")?;
    if sid.is_empty() || !sid.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err("QBITTORRENT_SID must contain only the alphanumeric SID value".into());
    }

    let mut cookie = HeaderValue::from_str(&format!("SID={sid}"))?;
    cookie.set_sensitive(true);
    Ok(cookie)
}

fn build_http_client() -> AppResult<Client> {
    Ok(Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(Policy::none())
        .build()?)
}

/// Encode the download options as multipart form fields expected by the API.
fn build_download_form(download: DownloadRequest) -> AppResult<Form> {
    let fields = download.form_fields();
    let is_link = Url::parse(&download.source)
        .map(|url| matches!(url.scheme(), "http" | "https" | "magnet" | "bc"))
        .unwrap_or(false);

    // Links go in `urls`; local files are uploaded in `torrents`.
    let mut form = if is_link {
        Form::new().text("urls", download.source)
    } else {
        Form::new()
            .file("torrents", &download.source)
            .map_err(|error| {
                format!(
                    "Could not open local torrent file '{}': {error}",
                    download.source
                )
            })?
    };
    for (name, value) in fields {
        form = form.text(name, value);
    }
    Ok(form)
}

fn submit_torrent(
    client: &Client,
    base_url: &Url,
    authentication: HeaderMap,
    download: DownloadRequest,
) -> AppResult<Response> {
    Ok(client
        .post(base_url.join(ADD_TORRENT_ENDPOINT)?)
        .headers(authentication)
        .header(REFERER, base_url.as_str())
        .multipart(build_download_form(download)?)
        .send()?)
}

/// Handle structured results and legacy text responses.
fn check_submission(response: Response) -> AppResult<String> {
    let status = response.status();
    let body = response.text()?;
    if !status.is_success() {
        return Err(format!("qBittorrent returned HTTP {status}; check the URL, API key or SID, server version, and Web UI access settings").into());
    }
    validate_submission_body(&body)?;
    Ok(body)
}

fn validate_submission_body(body: &str) -> AppResult<()> {
    match body.trim() {
        "" | "Ok." => Ok(()),
        "Fails." => Err("qBittorrent rejected the torrent submission".into()),
        _ => check_json_submission(body),
    }
}

/// Counts describe submission results, not completed downloads.
fn check_json_submission(body: &str) -> AppResult<()> {
    let result: serde_json::Value = serde_json::from_str(body).map_err(
        |_| "Unexpected response body from qBittorrent; check the Web UI before retrying",
    )?;
    let count = |field: &str| -> AppResult<u64> {
        result.get(field).and_then(serde_json::Value::as_u64)
            .ok_or_else(|| format!("qBittorrent response is missing a valid {field}; check the Web UI before retrying").into())
    };
    let succeeded = count("success_count")?;
    let failed = count("failure_count")?;
    let pending = count("pending_count")?;

    println!("Torrent submissions: {succeeded} added, {pending} pending, {failed} failed.");
    if failed > 0 {
        return Err("One or more torrent submissions failed; check the Web UI before retrying any successful or pending submissions".into());
    }
    if succeeded == 0 && pending == 0 {
        return Err("qBittorrent reported no added or pending torrents; check the Web UI".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_submission_body;

    #[test]
    fn accepts_observed_success_response() {
        let body = r#"{"added_torrent_ids":["0123456789abcdef0123456789abcdef01234567"],"failure_count":0,"pending_count":0,"success_count":1}"#;
        assert!(validate_submission_body(body).is_ok());
    }

    #[test]
    fn accepts_pending_but_rejects_failed_empty_or_invalid_results() {
        assert!(
            validate_submission_body(r#"{"success_count":0,"pending_count":1,"failure_count":0}"#)
                .is_ok()
        );
        for body in [
            r#"{"success_count":1,"pending_count":0,"failure_count":1}"#,
            r#"{"success_count":0,"pending_count":0,"failure_count":0}"#,
            r#"{"success_count":"1","pending_count":0,"failure_count":0}"#,
            "{}",
            "<html>Login</html>",
            "Fails.",
        ] {
            assert!(validate_submission_body(body).is_err(), "accepted {body}");
        }
    }

    #[test]
    fn accepts_legacy_success_responses() {
        assert!(validate_submission_body("").is_ok());
        assert!(validate_submission_body("Ok.\n").is_ok());
    }
}
