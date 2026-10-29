//! Exchange Web UI credentials for an SID used by submission and progress requests.
use crate::AppResult;
use reqwest::{
    Url,
    blocking::Client,
    header::{COOKIE, HeaderMap, HeaderValue, REFERER, SET_COOKIE},
};
use std::env;

pub fn from_environment(client: &Client, base_url: &Url) -> AppResult<HeaderMap> {
    let username = env::var("QBITTORRENT_USERNAME")
        .ok()
        .filter(|value| !value.is_empty())
        .ok_or("--login requires QBITTORRENT_USERNAME in .env or the environment")?;
    let password = env::var("QBITTORRENT_PASSWORD")
        .ok()
        .filter(|value| !value.is_empty())
        .ok_or("--login requires QBITTORRENT_PASSWORD in .env or the environment")?;
    authenticate(client, base_url, &username, &password)
}

fn authenticate(
    client: &Client,
    base_url: &Url,
    username: &str,
    password: &str,
) -> AppResult<HeaderMap> {
    let response = client
        .post(base_url.join("api/v2/auth/login")?)
        .header(REFERER, base_url.as_str())
        .form(&[("username", username), ("password", password)])
        .send()?;
    if !response.status().is_success() {
        return Err(format!(
            "Login returned HTTP {}; check credentials and Web UI access settings",
            response.status()
        )
        .into());
    }
    let cookies = response.headers().clone();
    let body = response.text()?;
    if !matches!(body.trim(), "" | "Ok.") {
        return Err("Login failed; check QBITTORRENT_USERNAME and QBITTORRENT_PASSWORD".into());
    }
    for value in cookies.get_all(SET_COOKIE) {
        let Ok(value) = value.to_str() else {
            continue;
        };
        let cookie = value.split(';').next().unwrap_or("").trim();
        if let Some((name, sid)) = cookie.split_once('=') {
            let session_name = name == "SID"
                || name.strip_prefix("QBT_SID_").is_some_and(|port| {
                    !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit())
                });
            // Session IDs are opaque cookie values and may contain punctuation.
            // RFC 6265 cookie-octet excludes whitespace, controls, quotes,
            // commas, semicolons, and backslashes.
            let valid_value = !sid.is_empty()
                && sid.bytes().all(
                    |b| matches!(b, 0x21 | 0x23..=0x2b | 0x2d..=0x3a | 0x3c..=0x5b | 0x5d..=0x7e),
                );
            if session_name && valid_value {
                let mut header = HeaderValue::from_str(cookie)?;
                header.set_sensitive(true);
                let mut authentication = HeaderMap::new();
                authentication.insert(COOKIE, header);
                return Ok(authentication);
            }
        }
    }
    Err("Login succeeded but no valid SID cookie was returned".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
        time::Duration,
    };

    #[test]
    fn login_encodes_credentials_and_handles_failure_or_missing_cookie() {
        for (status, body, cookie, expected) in [
            (
                "200 OK",
                "Ok.",
                "Set-Cookie: SID=abc123; HttpOnly; path=/\r\n",
                Some("SID=abc123"),
            ),
            (
                "204 No Content",
                "",
                "Set-Cookie: QBT_SID_8080=abc-123_+=; HttpOnly; path=/\r\n",
                Some("QBT_SID_8080=abc-123_+="),
            ),
            ("200 OK", "Fails.", "", None),
            ("200 OK", "Ok.", "", None),
            ("401 Unauthorized", "", "", None),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let base =
                Url::parse(&format!("http://{}/qbit/", listener.local_addr().unwrap())).unwrap();
            let server = thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0; 1024];
                loop {
                    let n = stream.read(&mut buffer).unwrap();
                    assert!(n > 0);
                    request.extend_from_slice(&buffer[..n]);
                    if let Some(end) = request.windows(4).position(|p| p == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&request[..end]).to_lowercase();
                        let length: usize = headers
                            .lines()
                            .find_map(|line| line.strip_prefix("content-length: "))
                            .unwrap()
                            .parse()
                            .unwrap();
                        if request.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                let request = String::from_utf8(request).unwrap();
                assert!(request.starts_with("POST /qbit/api/v2/auth/login "));
                assert!(request.contains("username=test+user&password=p%26%3D%2B"));
                assert!(!request.to_lowercase().contains("authorization:"));
                write!(stream, "HTTP/1.1 {status}\r\n{cookie}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            });
            let client = Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap();
            let result = authenticate(&client, &base, "test user", "p&=+");
            assert_eq!(result.is_ok(), expected.is_some());
            if let Ok(headers) = result {
                assert_eq!(headers[COOKIE], expected.unwrap());
                assert!(headers[COOKIE].is_sensitive());
            }
            server.join().unwrap();
        }
    }
}
