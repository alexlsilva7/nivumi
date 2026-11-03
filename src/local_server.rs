//! Local HTTP bridge server allowing the Nivumi Chrome Extension to communicate
//! with the running Nivumi desktop notch application.
use crate::services::{ExternalLyrics, MediaSnapshot, ServiceEvent};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{mpsc::Sender, Arc, Mutex},
    thread,
    time::Duration,
};

pub const DEFAULT_PORT: u16 = 48123;

pub fn start(
    current_media: Arc<Mutex<MediaSnapshot>>,
    events: Sender<ServiceEvent>,
) -> Option<u16> {
    let listener = TcpListener::bind(("127.0.0.1", DEFAULT_PORT))
        .or_else(|_| TcpListener::bind(("127.0.0.1", 0)))
        .ok()?;

    let port = listener.local_addr().ok()?.port();

    thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let media = current_media.clone();
            let events = events.clone();
            let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
            let _ = stream.set_write_timeout(Some(Duration::from_secs(3)));
            thread::spawn(move || {
                handle_client(&mut stream, &media, &events);
            });
        }
    });

    Some(port)
}

fn handle_client(
    stream: &mut TcpStream,
    media: &Arc<Mutex<MediaSnapshot>>,
    events: &Sender<ServiceEvent>,
) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
    let Some((method, path, body)) = read_request(stream) else {
        send_json_response(
            stream,
            400,
            r#"{"status":"error","message":"Incomplete or oversized request"}"#,
        );
        return;
    };

    // Handle CORS preflight OPTIONS request
    if method == "OPTIONS" {
        let response = "HTTP/1.1 204 No Content\r\n\
Access-Control-Allow-Origin: *\r\n\
Access-Control-Allow-Methods: GET, POST, OPTIONS\r\n\
Access-Control-Allow-Headers: Content-Type, Authorization\r\n\
Access-Control-Max-Age: 86400\r\n\
Connection: close\r\n\r\n";
        let _ = stream.write_all(response.as_bytes());
        return;
    }

    if method == "GET" && (path == "/api/status" || path == "/") {
        let json = r#"{"status":"ok","app":"nusa-notch","version":"0.2.0"}"#;
        send_json_response(stream, 200, json);
        return;
    }

    if method == "GET" && path == "/api/current" {
        let snapshot = media.lock().map(|m| m.clone()).unwrap_or_default();
        let json = serde_json::json!({
            "title": snapshot.title,
            "artist": snapshot.artist,
            "album": snapshot.album,
            "duration": snapshot.duration,
            "position": snapshot.position,
            "playing": snapshot.playing,
            "player": snapshot.player,
        });
        send_json_response(stream, 200, &json.to_string());
        return;
    }

    if method == "POST" && path == "/api/lyrics" {
        if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&body) {
            let title = val["title"].as_str().unwrap_or("").to_string();
            let artist = val["artist"].as_str().unwrap_or("").to_string();
            let original = val["original"].as_str().unwrap_or("").to_string();
            let translation = val["translation"].as_str().map(|s| s.to_string());
            let source = val["source"]
                .as_str()
                .unwrap_or("letras.mus.br")
                .to_string();

            if !original.is_empty() {
                let _ = events.send(ServiceEvent::ExternalLyrics(ExternalLyrics {
                    translation_origin: Default::default(),
                    title,
                    artist,
                    original,
                    translation,
                    source,
                }));

                let json = r#"{"status":"success","message":"Lyrics received by Nivumi app"}"#;
                send_json_response(stream, 200, json);
                return;
            }
        }

        let json = r#"{"status":"error","message":"Invalid payload. 'original' lyrics required."}"#;
        send_json_response(stream, 400, json);
        return;
    }

    // 404 for unknown endpoints
    let json = r#"{"status":"not_found"}"#;
    send_json_response(stream, 404, json);
}

/// HTTP/TCP chunks do not align with headers, UTF-8 characters or complete LRC files.
fn read_request(stream: &mut TcpStream) -> Option<(String, String, Vec<u8>)> {
    const MAX_HEADERS: usize = 16 * 1024;
    const MAX_BODY: usize = 1024 * 1024;
    let mut request = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        if let Some(end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
            break end + 4;
        }
        if request.len() >= MAX_HEADERS {
            return None;
        }
        let count = stream.read(&mut chunk).ok()?;
        if count == 0 {
            return None;
        }
        request.extend_from_slice(&chunk[..count]);
    };
    if header_end > MAX_HEADERS {
        return None;
    }
    let headers = std::str::from_utf8(&request[..header_end]).ok()?;
    let mut lines = headers.lines();
    let mut first = lines.next()?.split_whitespace();
    let method = first.next()?.to_owned();
    let path = first.next()?.to_owned();
    let mut length = None;
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                if length.is_some() {
                    return None;
                }
                length = Some(value.trim().parse::<usize>().ok()?);
            }
            if name.eq_ignore_ascii_case("transfer-encoding") {
                return None;
            }
        }
    }
    let length = length.unwrap_or(0);
    if length > MAX_BODY {
        return None;
    }
    while request.len() < header_end + length {
        let count = stream.read(&mut chunk).ok()?;
        if count == 0 {
            return None;
        }
        request.extend_from_slice(&chunk[..count]);
    }
    Some((
        method,
        path,
        request[header_end..header_end + length].to_vec(),
    ))
}

fn send_json_response(stream: &mut TcpStream, status_code: u16, json: &str) {
    let status_text = match status_code {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        404 => "Not Found",
        _ => "OK",
    };
    let response = format!(
        "HTTP/1.1 {} {}\r\n\
Content-Type: application/json; charset=utf-8\r\n\
Access-Control-Allow-Origin: *\r\n\
Access-Control-Allow-Methods: GET, POST, OPTIONS\r\n\
Access-Control-Allow-Headers: Content-Type, Authorization\r\n\
Content-Length: {}\r\n\
Connection: close\r\n\r\n{}",
        status_code,
        status_text,
        json.len(),
        json
    );
    let _ = stream.write_all(response.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn receives_large_utf8_lyrics_across_fragmented_tcp_writes() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (events, received) = std::sync::mpsc::channel();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            handle_client(
                &mut stream,
                &Arc::new(Mutex::new(MediaSnapshot::default())),
                &events,
            );
        });
        let original = "[00:01.00]Canção de teste com acentuação\n".repeat(1000);
        let body = serde_json::json!({"title":"Teste", "artist":"Artista", "original":original, "source":"lyricsify.com"}).to_string();
        let request = format!(
            "POST /api/lyrics HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let mut client = TcpStream::connect(address).unwrap();
        for chunk in request.as_bytes().chunks(127) {
            client.write_all(chunk).unwrap();
        }
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        assert!(response.contains("200 OK"));
        let ServiceEvent::ExternalLyrics(result) = received.recv().unwrap() else {
            panic!("Expected imported lyrics");
        };
        assert_eq!(result.original, original);
        server.join().unwrap();
    }

    #[test]
    fn test_local_server_routes() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        let media = Arc::new(Mutex::new(MediaSnapshot {
            title: "Mr. FEAR".into(),
            artist: "SIAMÉS".into(),
            playing: true,
            ..Default::default()
        }));
        let (tx, rx) = std::sync::mpsc::channel();

        let media_clone = media.clone();
        thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                handle_client(&mut stream, &media_clone, &tx);
            }
        });

        // Test GET /api/status
        {
            let mut client = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
            client
                .write_all(b"GET /api/status HTTP/1.1\r\nHost: localhost\r\n\r\n")
                .unwrap();
            let mut res = String::new();
            client.read_to_string(&mut res).unwrap();
            assert!(res.contains("200 OK"));
            assert!(res.contains("\"app\":\"nusa-notch\""));
        }

        // Test GET /api/current
        {
            let mut client = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
            client
                .write_all(b"GET /api/current HTTP/1.1\r\nHost: localhost\r\n\r\n")
                .unwrap();
            let mut res = String::new();
            client.read_to_string(&mut res).unwrap();
            assert!(res.contains("200 OK"));
            assert!(res.contains("Mr. FEAR"));
            assert!(res.contains("SIAMÉS"));
        }

        // Test POST /api/lyrics
        {
            let payload = r#"{"title":"Mr. FEAR","artist":"SIAMÉS","original":"Listen...","translation":"Ouça...","source":"letras.mus.br"}"#;
            let mut client = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
            let req = format!(
                "POST /api/lyrics HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n{}",
                payload.len(),
                payload
            );
            client.write_all(req.as_bytes()).unwrap();
            let mut res = String::new();
            client.read_to_string(&mut res).unwrap();
            assert!(res.contains("200 OK"));
            assert!(res.contains("Lyrics received by Nivumi app"));

            let event = rx.try_recv().unwrap();
            match event {
                ServiceEvent::ExternalLyrics(ext) => {
                    assert_eq!(ext.title, "Mr. FEAR");
                    assert_eq!(ext.artist, "SIAMÉS");
                    assert_eq!(ext.original, "Listen...");
                    assert_eq!(ext.translation.as_deref(), Some("Ouça..."));
                }
                _ => panic!("Expected ExternalLyrics event"),
            }
        }
    }
}
