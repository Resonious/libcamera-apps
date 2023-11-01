use std::time::Duration;

use hyper::{body::HttpBody, client::HttpConnector, Body, Client, Method, Request};
use hyper_rustls::HttpsConnector;
use serde_json::json;
use webrtc::{
    ice_transport::ice_candidate::{RTCIceCandidate, RTCIceCandidateInit},
    peer_connection::sdp::session_description::RTCSessionDescription,
};

use tokio::{sync::mpsc::{self, Receiver, Sender}, time::{timeout, error::Elapsed}};

pub enum Outgoing {
    Session(RTCSessionDescription),
    Candidate(Option<RTCIceCandidate>),
}

pub enum Incoming {
    Session(RTCSessionDescription),
    Candidate(RTCIceCandidateInit),
}

/// must_read_stdin blocks until input is received from stdin
pub fn must_read_stdin() -> anyhow::Result<String> {
    let mut line = String::new();

    std::io::stdin().read_line(&mut line)?;
    line = line.trim().to_owned();
    println!();

    Ok(line)
}

pub struct Broker {
    client: Client<HttpsConnector<HttpConnector>, Body>,
    name: String,
}

impl Broker {
    pub fn new(name: &str) -> Self {
        let https = hyper_rustls::HttpsConnectorBuilder::new()
            .with_native_roots()
            .https_only()
            .enable_http2()
            .build();

        let client = Client::builder().build(https);

        Self {
            name: name.to_string(),
            client,
        }
    }

    pub fn open_incoming_channel(self: &Self) -> Receiver<Incoming> {
        let (incoming_message_sender, incoming_message_receiver) = mpsc::channel(64);
        let listen_name = self.name.to_string();
        let listen_client = self.client.clone();
        tokio::spawn(async move {
            'listen_loop: loop {
                let snd_receive_url =
                    format!("https://hook.snd.one/resonious/teleport/{listen_name}/eye");
                let get = Request::builder()
                    .method(Method::GET)
                    .uri(snd_receive_url)
                    .header("Accept", "text/event-stream")
                    .body(Body::empty())
                    .expect("Failed to build GET request");
                let mut resp = match listen_client.request(get).await {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::error!("snd.one GET request failed: {e:?}");
                        continue;
                    }
                };

                let mut line = Vec::<u8>::with_capacity(4096);
                let mut event_name: Option<Vec<u8>> = None;

                loop {
                    let next = match timeout(Duration::from_secs(3600), resp.data()).await {
                        Ok(Some(x)) => x,
                        Err(Elapsed { .. }) | Ok(None) => {
                            break 'listen_loop;
                        }
                    };
                    let chunk = match next {
                        Ok(c) => c,
                        Err(e) => {
                            tracing::error!("snd.one GET request body broke: {e:?}");
                            continue 'listen_loop;
                        }
                    };

                    for byte in chunk {
                        if byte != b"\n"[0] {
                            line.push(byte);
                            continue;
                        }

                        // process line
                        if line.len() == 0 {
                            event_name = None;
                        } else if line.starts_with(b"event: ") {
                            event_name = Some(line[7..].to_vec());
                        } else if line.starts_with(b"data: ") && event_name.is_none() {
                            let data = &line[6..];
                            let utf8 = String::from_utf8_lossy(&line);

                            if let Ok(json) = serde_json::from_slice::<serde_json::Value>(data) {
                                if json.get("sdp").is_some() {
                                    match serde_json::from_value::<RTCSessionDescription>(json) {
                                        Ok(session_desc) => {
                                            match incoming_message_sender
                                                .send(Incoming::Session(session_desc))
                                                .await
                                            {
                                                Ok(_) => {
                                                    line.clear();
                                                    continue;
                                                }
                                                Err(e) => {
                                                    tracing::debug!("Listener shutting down {e:?}");
                                                    break 'listen_loop;
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            tracing::warn!("Invalid SDP signal: {e:?} {utf8}");
                                        }
                                    }
                                } else if let Some(candidate) = json.get("candidate") {
                                    match serde_json::from_value::<RTCIceCandidateInit>(
                                        candidate.clone(),
                                    ) {
                                        Ok(ice_candidate) => {
                                            match incoming_message_sender
                                                .send(Incoming::Candidate(ice_candidate))
                                                .await
                                            {
                                                Ok(_) => {
                                                    line.clear();
                                                    continue;
                                                }
                                                Err(e) => {
                                                    tracing::debug!("Listener shutting down {e:?}");
                                                    break 'listen_loop;
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            tracing::warn!(
                                                "Invalid candidate signal: {e:?} {utf8}"
                                            );
                                        }
                                    }
                                } else {
                                    tracing::warn!("Unknown signal format: {utf8}");
                                }
                            } else {
                                tracing::warn!("Invalid JSON signal: {utf8}");
                            }
                        }
                        line.clear();
                    }
                }
            }
        });

        incoming_message_receiver
    }

    // Open an outgoing channel for sending answers to a potential peer
    pub fn open_outgoing_channel(self: &Self) -> Sender<Outgoing> {
        let (outgoing_message_sender, mut outgoing_message_receiver) =
            mpsc::channel::<Outgoing>(64);
        let send_name = self.name.clone();
        let send_client = self.client.clone();
        tokio::spawn(async move {
            while let Some(message) = outgoing_message_receiver.recv().await {
                let body_json = match message {
                    Outgoing::Session(session_desc) => {
                        serde_json::to_string(&session_desc).unwrap()
                    }
                    Outgoing::Candidate(Some(ice_candidate)) => {
                        let formatted = match ice_candidate.to_json() {
                            Ok(x) => x,
                            Err(e) => {
                                tracing::error!("Invalid outgoing ice candidate? {e:?}");
                                continue;
                            }
                        };
                        let wrapped = json!({ "type": "candidate", "candidate": formatted });
                        serde_json::to_string(&wrapped).unwrap()
                    }
                    Outgoing::Candidate(None) => {
                        "{\"type\":\"candidate\",\"candidate\":{\"candidate\":\"\",\"sdpMLineIndex\":0,\"sdpMid\":\"0\"}}"
                            .to_string()
                    }
                };

                let snd_send_url =
                    format!("https://hook.snd.one/resonious/teleport/{send_name}/head");
                let post = Request::builder()
                    .method(Method::POST)
                    .uri(snd_send_url)
                    .header("Content-Type", "application/json")
                    .body(Body::from(body_json))
                    .expect("Failed to build POST request");
                let mut resp = match send_client.request(post).await {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::error!("snd.one GET request failed: {e:?}");
                        continue;
                    }
                };
                if !resp.status().is_success() {
                    let status = resp.status();
                    let mut body = Vec::<u8>::with_capacity(4096);
                    while let Some(next) = resp.data().await {
                        let Ok(chunk) = next else { break };
                        for byte in chunk {
                            body.push(byte);
                        }
                    }
                    tracing::error!(
                        "snd.one GET request failed ({status}): {}",
                        String::from_utf8_lossy(&body)
                    );
                    continue;
                }
            }
        });

        outgoing_message_sender
    }
}

/// Not even a real unit test, just me playing around with eventstream...
#[tokio::test]
async fn test_eventstream() {
    use hyper::{body::HttpBody, Body, Client, Method, Request};
    use tokio::io::AsyncWriteExt;

    let https = hyper_rustls::HttpsConnectorBuilder::new()
        .with_native_roots()
        .https_only()
        .enable_http2()
        .build();

    let client: Client<_, Body> = Client::builder().build(https);
    let req = Request::builder()
        .method(Method::GET)
        .uri("https://hook.snd.one/resonious/teleport/eye2")
        .header("Accept", "text/event-stream")
        .body(Body::empty())
        .expect("Failed to build GET request");
    let mut res = client.request(req).await.unwrap();

    println!("Response: {}", res.status());
    println!("Headers: {:#?}\n", res.headers());

    tokio::spawn(async {
        let https = hyper_rustls::HttpsConnectorBuilder::new()
            .with_native_roots()
            .https_only()
            .enable_http2()
            .build();
        let client: Client<_, Body> = Client::builder().build(https);
        let req = Request::builder()
            .method(Method::POST)
            .uri("https://hook.snd.one/resonious/teleport/eye2")
            .body(Body::from("hoy"))
            .expect("Failed to build POST request");

        let _ = client.request(req).await.unwrap();
    });

    // Stream the body, writing each chunk to stdout as we get it
    // (instead of buffering and printing at the end).
    while let Some(next) = res.data().await {
        let chunk = next.unwrap();
        tokio::io::stdout().write_all(&chunk).await.unwrap();
    }

    println!("\n\nDone!");
}
