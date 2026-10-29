//! Poll only the torrents returned by this submission, leaving seeding settings alone.
use crate::AppResult;
use reqwest::{
    Url,
    blocking::Client,
    header::{HeaderMap, REFERER},
};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    thread,
    time::{Duration, Instant},
};

const REGISTRATION_TIMEOUT: Duration = Duration::from_secs(60);

pub fn added_torrent_ids(body: &str) -> AppResult<Vec<String>> {
    let value: Value = serde_json::from_str(body).map_err(
        |_| "Submission accepted, but this server did not return torrent IDs for progress tracking",
    )?;
    let ids = value["added_torrent_ids"]
        .as_array()
        .ok_or("Submission accepted, but no added_torrent_ids were returned for tracking")?;
    let mut result = Vec::new();
    for id in ids {
        let id = id
            .as_str()
            .ok_or("Invalid torrent ID in submission response")?;
        if !matches!(id.len(), 40 | 64) || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("Invalid torrent ID in submission response".into());
        }
        result.push(id.to_ascii_lowercase());
    }
    if result.is_empty() {
        return Err("Submission accepted but no torrent IDs are available yet (possibly pending). Check qBittorrent; progress cannot be tracked for this response".into());
    }
    if value["pending_count"].as_u64().unwrap_or(0) > 0 {
        return Err("Submission includes pending torrents without IDs; check qBittorrent for their progress".into());
    }
    Ok(result)
}

pub fn track_downloads(
    client: &Client,
    base_url: &Url,
    authentication: &HeaderMap,
    ids: &[String],
    poll_interval: Duration,
) -> AppResult<()> {
    let hashes = ids.join("|");
    let mut completed = HashSet::new();
    let mut seen = HashSet::new();
    let mut file_counts = HashMap::new();
    let started = Instant::now();
    println!(
        "Tracking downloads every {} seconds. Ctrl+C exits monitoring; qBittorrent keeps running.",
        poll_interval.as_secs()
    );

    loop {
        let response = client
            .get(base_url.join("api/v2/torrents/info")?)
            .headers(authentication.clone())
            .header(REFERER, base_url.as_str())
            .query(&[("hashes", hashes.as_str())])
            .send()?;
        let status = response.status();
        if !status.is_success() {
            return Err(format!("Progress request returned HTTP {status}; the download may still be running in qBittorrent").into());
        }
        let data: Value = serde_json::from_str(&response.text()?)?;
        let torrents = data
            .as_array()
            .ok_or("Expected a torrent array from progress API")?;
        let mut present = HashSet::new();
        for torrent in torrents {
            let hash = torrent["hash"]
                .as_str()
                .ok_or("Progress response missing torrent hash")?
                .to_ascii_lowercase();
            if !ids.contains(&hash) {
                continue;
            }
            present.insert(hash.clone());
            seen.insert(hash.clone());
            if completed.contains(&hash) {
                continue;
            }

            let name = torrent["name"].as_str().unwrap_or(&hash);
            let state = torrent["state"]
                .as_str()
                .ok_or("Progress response missing state")?;
            if matches!(state, "error" | "missingFiles" | "unknown") {
                return Err(format!(
                    "Torrent {name:?} cannot continue: {state}. Check qBittorrent"
                )
                .into());
            }
            let progress = torrent["progress"]
                .as_f64()
                .ok_or("Progress response missing percentage")?;
            let speed = torrent["dlspeed"].as_u64().unwrap_or(0) as f64 / 1_048_576.0;
            let eta = format_eta(torrent["eta"].as_i64());
            let metadata_pending = matches!(state, "metaDL" | "forcedMetaDL");
            if !metadata_pending && !file_counts.contains_key(&hash) {
                // Cache the file count once metadata is available.
                match fetch_file_count(client, base_url, authentication, &hash) {
                    Ok(Some(count)) => {
                        file_counts.insert(hash.clone(), count);
                    }
                    Ok(None) => {} // Metadata may not be ready yet; try next poll.
                    Err(error) => eprintln!("Could not read file count: {error}"),
                }
            }
            let files = file_counts
                .get(&hash)
                .map(|count| count.to_string())
                .unwrap_or_else(|| "unknown".into());
            println!(
                "{name:?}: {:6.2}% | {speed:.2} MiB/s | ETA {eta} | {state}",
                progress * 100.0
            );
            println!(
                "  {} | Files: {files}",
                format_sizes(torrent, metadata_pending)
            );
            if is_complete(torrent) {
                println!("Downloaded: {name:?}");
                if let Some(path) = torrent["content_path"].as_str() {
                    println!("Server path: {path}");
                }
                completed.insert(hash);
            }
        }
        for id in ids {
            if completed.contains(id) || present.contains(id) {
                continue;
            }
            if seen.contains(id) {
                return Err(
                    format!("Torrent {id} disappeared from qBittorrent before completion").into(),
                );
            }
            if started.elapsed() >= REGISTRATION_TIMEOUT {
                return Err(format!(
                    "Torrent {id} did not appear within 60 seconds; check qBittorrent"
                )
                .into());
            }
            println!("Waiting for torrent {id} to appear...");
        }
        if completed.len() == ids.len() {
            println!(
                "All downloads completed. qBittorrent continues with its configured seeding settings."
            );
            return Ok(());
        }
        thread::sleep(poll_interval);
    }
}

fn fetch_file_count(
    client: &Client,
    base_url: &Url,
    authentication: &HeaderMap,
    hash: &str,
) -> AppResult<Option<usize>> {
    let response = client
        .get(base_url.join("api/v2/torrents/files")?)
        .headers(authentication.clone())
        .header(REFERER, base_url.as_str())
        .query(&[("hash", hash)])
        .send()?;
    if !response.status().is_success() {
        return Err(format!("File list returned HTTP {}", response.status()).into());
    }
    let files: Value = serde_json::from_str(&response.text()?)?;
    let files = files
        .as_array()
        .ok_or("Expected a file array from the API")?;
    Ok((!files.is_empty()).then_some(files.len()))
}

fn format_bytes(bytes: u64) -> String {
    let units = ["B", "KiB", "MiB", "GiB", "TiB", "PiB", "EiB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < units.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    format!("{size:.2} {}", units[unit])
}

fn format_sizes(torrent: &Value, metadata_pending: bool) -> String {
    if metadata_pending {
        return "Size: unknown (waiting for metadata)".into();
    }
    let selected = torrent["size"].as_u64();
    let remaining = torrent["amount_left"].as_u64();
    let downloaded = selected
        .zip(remaining)
        .map(|(size, left)| size.saturating_sub(left));
    let display = |bytes: Option<u64>| bytes.map(format_bytes).unwrap_or_else(|| "unknown".into());
    format!(
        "Total: {} | Selected: {} | Downloaded: {} | Remaining: {}",
        display(torrent["total_size"].as_u64()),
        display(selected),
        display(downloaded),
        display(remaining)
    )
}

fn is_complete(torrent: &Value) -> bool {
    let state = torrent["state"].as_str().unwrap_or("");
    // Metadata and checking/moving phases can temporarily report misleading totals.
    if matches!(
        state,
        "metaDL"
            | "forcedMetaDL"
            | "checkingDL"
            | "checkingUP"
            | "checkingResumeData"
            | "moving"
            | "allocating"
            | "error"
            | "missingFiles"
            | "unknown"
    ) {
        return false;
    }
    torrent["progress"].as_f64().is_some_and(|p| p >= 1.0)
        && torrent["amount_left"].as_u64() == Some(0)
}

fn format_eta(seconds: Option<i64>) -> String {
    match seconds {
        Some(s) if (0..8_640_000).contains(&s) => {
            format!("{}h {:02}m {:02}s", s / 3600, (s % 3600) / 60, s % 60)
        }
        _ => "unknown".into(),
    }
}

#[cfg(test)]
#[test]
fn polls_until_download_completes() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let id = "0123456789abcdef0123456789abcdef01234567";
    let server = thread::spawn(move || {
        for step in 0..3 {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 1024];
            while !request.windows(4).any(|part| part == b"\r\n\r\n") {
                let count = stream.read(&mut buffer).unwrap();
                assert!(count > 0);
                request.extend_from_slice(&buffer[..count]);
            }
            let request = String::from_utf8(request).unwrap();
            assert!(request.contains("authorization: Bearer test-key\r\n"));
            let body = if step == 1 {
                assert!(request.starts_with(&format!("GET /api/v2/torrents/files?hash={id} ")));
                serde_json::json!([{"name":"first.bin"}, {"name":"second.bin"}]).to_string()
            } else {
                assert!(request.starts_with(&format!("GET /api/v2/torrents/info?hashes={id} ")));
                let (progress, left, state) = if step == 0 {
                    (0.5, 100, "downloading")
                } else {
                    (1.0, 0, "stalledUP")
                };
                serde_json::json!([{"hash":id,"name":"test","progress":progress,
                    "size":200,"total_size":300,"amount_left":left,"state":state,"dlspeed":100,"eta":2}]).to_string()
            };
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
    });
    let client = Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let mut headers = HeaderMap::new();
    headers.insert(
        reqwest::header::AUTHORIZATION,
        "Bearer test-key".parse().unwrap(),
    );
    track_downloads(
        &client,
        &base,
        &headers,
        &[id.into()],
        Duration::from_millis(10),
    )
    .unwrap();
    server.join().unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sizes_distinguish_selected_content_and_unknown_metadata() {
        let torrent = json!({"size":2048,"total_size":4096,"amount_left":512});
        assert_eq!(
            format_sizes(&torrent, false),
            "Total: 4.00 KiB | Selected: 2.00 KiB | Downloaded: 1.50 KiB | Remaining: 512.00 B"
        );
        assert!(format_sizes(&torrent, true).contains("waiting for metadata"));
        assert!(format_sizes(&json!({}), false).contains("Total: unknown"));
        assert_eq!(format_bytes(1_073_741_824), "1.00 GiB");
    }

    #[test]
    fn metadata_checking_and_moving_are_not_finished() {
        for state in [
            "metaDL",
            "forcedMetaDL",
            "checkingDL",
            "checkingUP",
            "moving",
            "error",
        ] {
            assert!(!is_complete(
                &json!({"state":state,"progress":1.0,"amount_left":0})
            ));
        }
        assert!(!is_complete(
            &json!({"state":"downloading","progress":0.5,"amount_left":100})
        ));
        assert!(is_complete(
            &json!({"state":"stalledUP","progress":1.0,"amount_left":0})
        ));
        assert!(is_complete(
            &json!({"state":"stoppedUP","progress":1.0,"amount_left":0})
        ));
    }

    #[test]
    fn extracts_observed_id_and_rejects_untrackable_results() {
        let id = "0123456789abcdef0123456789abcdef01234567";
        assert_eq!(
            added_torrent_ids(&json!({"added_torrent_ids":[id]}).to_string()).unwrap(),
            vec![id]
        );
        for body in [
            "Ok.",
            "{}",
            r#"{"added_torrent_ids":[]}"#,
            r#"{"added_torrent_ids":["invalid"]}"#,
        ] {
            assert!(added_torrent_ids(body).is_err());
        }
        assert_eq!(format_eta(Some(3661)), "1h 01m 01s");
        assert_eq!(format_eta(Some(8_640_000)), "unknown");
    }
}
