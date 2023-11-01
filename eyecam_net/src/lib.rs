use std::{ffi::c_char, ffi::c_int, ffi::c_void, io::Cursor, mem, time::Duration};

use tracing::Level;
use webrtc::media::{io::h264_reader::H264Reader, Sample};

pub mod peer;
pub mod servos;
pub mod signal;

// use tokio::sync::mpsc;

struct State {
    runtime: tokio::runtime::Runtime,
    connection: Option<peer::Connection>,
}

#[no_mangle]
pub extern "C" fn eyecam_net_init() -> *const c_void {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(1)
        .build()
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("Failed to create tokio runtime {e:?}");
            return std::ptr::null();
        }
    };

    let _runtime_enter = runtime.enter();

    let log_level = match std::env::var("EYE_LOG") {
        Ok(s) => {
            if s == "DEBUG" {
                Level::DEBUG
            } else {
                Level::INFO
            }
        },
        Err(_) => Level::INFO,
    };
    let _ = tracing_subscriber::fmt()
        .with_max_level(log_level)
        .with_ansi(true)
        .try_init();

    let state = Box::into_raw(Box::new(State {
        runtime,
        connection: None,
    }));

    unsafe { mem::transmute(state) }
}

#[no_mangle]
pub extern "C" fn eyecam_net_deinit(state: *const c_void) {
    if state.is_null() {
        return;
    }

    unsafe {
        let state = Box::<State>::from_raw(mem::transmute(state));
        state.runtime.shutdown_background();
    }
}

#[no_mangle]
pub extern "C" fn eyecam_net_wait_for_connection(state: *mut c_void, name: *const c_char) -> c_int {
    let state = unsafe { Box::leak(Box::<State>::from_raw(mem::transmute(state))) };
    let name = unsafe {
        let cstr = std::ffi::CStr::from_ptr(name);
        cstr.to_str().unwrap_or("invalid")
    };

    let connection = state.runtime.block_on(peer::Connection::wait_for_new(name));

    match connection {
        Ok(c) => state.connection = Some(c),
        Err(_e) => {
            // TODO: set error on handle I guess
            return 0;
        }
    }

    1
}

#[no_mangle]
pub extern "C" fn eyecam_net_write_video(
    state: *mut c_void,
    len: usize,
    data: *const u8,
    microseconds: u64,
) -> c_int {
    let state = unsafe { Box::leak(Box::<State>::from_raw(mem::transmute(state))) };

    let connection = match &state.connection {
        Some(c) => c,
        None => {
            // TODO: error message on handle!!
            return 0;
        }
    };

    let slice = unsafe { std::slice::from_raw_parts(data, len) };
    let cursor = Cursor::new(slice);
    let mut h264 = H264Reader::new(cursor, len * 2);

    loop {
        let nal = match h264.next_nal() {
            Ok(n) => n,
            Err(e) => {
                let msg = e.to_string();
                if msg.ends_with("EOF") {
                    return 1;
                } else {
                    return 0;
                }
            }
        };

        match state
            .runtime
            .block_on(connection.video_track.write_sample(&Sample {
                data: nal.data.freeze(),
                duration: Duration::from_micros(microseconds),
                ..Default::default()
            })) {
            Ok(()) => {}
            Err(e) => {
                tracing::error!("Failed to write sample {e}");
                return 0;
            }
        }
    }
}

#[tokio::test]
async fn webrtc_example_test() {
    use std::fs::File;
    use std::io::BufReader;
    use std::io::Read;
    use std::path::Path;
    use std::sync::Arc;

    use anyhow::Result;

    use tokio::sync::Notify;
    use tokio::time::Duration;
    use webrtc::api::interceptor_registry::register_default_interceptors;
    use webrtc::api::media_engine::{MediaEngine, MIME_TYPE_H264, MIME_TYPE_OPUS};
    use webrtc::api::APIBuilder;
    use webrtc::ice_transport::ice_connection_state::RTCIceConnectionState;
    use webrtc::ice_transport::ice_server::RTCIceServer;
    use webrtc::interceptor::registry::Registry;
    use webrtc::media::io::h264_reader::H264Reader;
    use webrtc::media::io::ogg_reader::OggReader;
    use webrtc::media::Sample;
    use webrtc::peer_connection::configuration::RTCConfiguration;
    use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
    use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
    use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability;
    use webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample;
    use webrtc::track::track_local::TrackLocal;

    const OGG_PAGE_DURATION: Duration = Duration::from_millis(20);

    let video_file = Some("vid.h264".to_string());
    let audio_file: Option<String> = None;

    if let Some(video_path) = &video_file {
        if !Path::new(video_path).exists() {
            assert!(false);
            // return Err(Error::new(format!("video file: '{video_path}' not exist")).into());
        }
    }
    if let Some(audio_path) = &audio_file {
        if !Path::new(audio_path).exists() {
            assert!(false);
            // return Err(Error::new(format!("audio file: '{audio_path}' not exist")).into());
        }
    }

    // Everything below is the WebRTC-rs API! Thanks for using it ❤️.

    // Create a MediaEngine object to configure the supported codec
    let mut m = MediaEngine::default();

    m.register_default_codecs().unwrap();

    // Create a InterceptorRegistry. This is the user configurable RTP/RTCP Pipeline.
    // This provides NACKs, RTCP Reports and other features. If you use `webrtc.NewPeerConnection`
    // this is enabled by default. If you are manually managing You MUST create a InterceptorRegistry
    // for each PeerConnection.
    let mut registry = Registry::new();

    // Use the default set of Interceptors
    registry = register_default_interceptors(registry, &mut m).unwrap();

    // Create the API object with the MediaEngine
    let api = APIBuilder::new()
        .with_media_engine(m)
        .with_interceptor_registry(registry)
        .build();

    // Prepare the configuration
    let config = RTCConfiguration {
        ice_servers: vec![RTCIceServer {
            urls: vec!["stun:stun.l.google.com:19302".to_owned()],
            ..Default::default()
        }],
        ..Default::default()
    };

    // Create a new RTCPeerConnection
    let peer_connection = Arc::new(api.new_peer_connection(config).await.unwrap());

    let notify_tx = Arc::new(Notify::new());
    let notify_video = notify_tx.clone();
    let notify_audio = notify_tx.clone();

    let (done_tx, mut done_rx) = tokio::sync::mpsc::channel::<()>(1);
    let video_done_tx = done_tx.clone();
    let audio_done_tx = done_tx.clone();

    if let Some(video_file) = video_file {
        // Create a video track
        let video_track = Arc::new(TrackLocalStaticSample::new(
            RTCRtpCodecCapability {
                mime_type: MIME_TYPE_H264.to_owned(),
                ..Default::default()
            },
            "video".to_owned(),
            "webrtc-rs".to_owned(),
        ));

        // Add this newly created track to the PeerConnection
        let rtp_sender = peer_connection
            .add_track(Arc::clone(&video_track) as Arc<dyn TrackLocal + Send + Sync>)
            .await
            .unwrap();

        // Read incoming RTCP packets
        // Before these packets are returned they are processed by interceptors. For things
        // like NACK this needs to be called.
        tokio::spawn(async move {
            let mut rtcp_buf = vec![0u8; 1500];
            while let Ok((_, _)) = rtp_sender.read(&mut rtcp_buf).await {}
            Result::<()>::Ok(())
        });

        let video_file_name = video_file.to_owned();
        tokio::spawn(async move {
            // Open a H264 file and start reading using our H264Reader
            let file = File::open(&video_file_name).unwrap();
            let reader = BufReader::new(file);
            let mut h264 = H264Reader::new(reader, 1_048_576);

            // Wait for connection established
            notify_video.notified().await;

            println!("play video from disk file {video_file_name}");

            // It is important to use a time.Ticker instead of time.Sleep because
            // * avoids accumulating skew, just calling time.Sleep didn't compensate for the time spent parsing the data
            // * works around latency issues with Sleep
            let mut ticker = tokio::time::interval(Duration::from_millis(33));
            loop {
                let nal = match h264.next_nal() {
                    Ok(nal) => nal,
                    Err(err) => {
                        println!("All video frames parsed and sent: {err}");
                        break;
                    }
                };

                /*println!(
                    "PictureOrderCount={}, ForbiddenZeroBit={}, RefIdc={}, UnitType={}, data={}",
                    nal.picture_order_count,
                    nal.forbidden_zero_bit,
                    nal.ref_idc,
                    nal.unit_type,
                    nal.data.len()
                );*/

                video_track
                    .write_sample(&Sample {
                        data: nal.data.freeze(),
                        duration: Duration::from_secs(1),
                        ..Default::default()
                    })
                    .await
                    .unwrap();

                let _ = ticker.tick().await;
            }

            let _ = video_done_tx.try_send(());

            Result::<()>::Ok(())
        });
    }

    if let Some(audio_file) = audio_file {
        // Create a audio track
        let audio_track = Arc::new(TrackLocalStaticSample::new(
            RTCRtpCodecCapability {
                mime_type: MIME_TYPE_OPUS.to_owned(),
                ..Default::default()
            },
            "audio".to_owned(),
            "webrtc-rs".to_owned(),
        ));

        // Add this newly created track to the PeerConnection
        let rtp_sender = peer_connection
            .add_track(Arc::clone(&audio_track) as Arc<dyn TrackLocal + Send + Sync>)
            .await
            .unwrap();

        // Read incoming RTCP packets
        // Before these packets are returned they are processed by interceptors. For things
        // like NACK this needs to be called.
        tokio::spawn(async move {
            let mut rtcp_buf = vec![0u8; 1500];
            while let Ok((_, _)) = rtp_sender.read(&mut rtcp_buf).await {}
            Result::<()>::Ok(())
        });

        let audio_file_name = audio_file.to_owned();
        tokio::spawn(async move {
            // Open a IVF file and start reading using our IVFReader
            let file = File::open(audio_file_name).unwrap();
            let reader = BufReader::new(file);
            // Open on oggfile in non-checksum mode.
            let (mut ogg, _) = OggReader::new(reader, true).unwrap();

            // Wait for connection established
            notify_audio.notified().await;

            println!("play audio from disk file output.ogg");

            // It is important to use a time.Ticker instead of time.Sleep because
            // * avoids accumulating skew, just calling time.Sleep didn't compensate for the time spent parsing the data
            // * works around latency issues with Sleep
            let mut ticker = tokio::time::interval(OGG_PAGE_DURATION);

            // Keep track of last granule, the difference is the amount of samples in the buffer
            let mut last_granule: u64 = 0;
            while let Ok((page_data, page_header)) = ogg.parse_next_page() {
                // The amount of samples is the difference between the last and current timestamp
                let sample_count = page_header.granule_position - last_granule;
                last_granule = page_header.granule_position;
                let sample_duration = Duration::from_millis(sample_count * 1000 / 48000);

                audio_track
                    .write_sample(&Sample {
                        data: page_data.freeze(),
                        duration: sample_duration,
                        ..Default::default()
                    })
                    .await
                    .unwrap();

                let _ = ticker.tick().await;
            }

            let _ = audio_done_tx.try_send(());

            Result::<()>::Ok(())
        });
    }

    // Set the handler for ICE connection state
    // This will notify you when the peer has connected/disconnected
    peer_connection.on_ice_connection_state_change(Box::new(
        move |connection_state: RTCIceConnectionState| {
            println!("Connection State has changed {connection_state}");
            if connection_state == RTCIceConnectionState::Connected {
                notify_tx.notify_waiters();
            }
            Box::pin(async {})
        },
    ));

    // Set the handler for Peer connection state
    // This will notify you when the peer has connected/disconnected
    peer_connection.on_peer_connection_state_change(Box::new(move |s: RTCPeerConnectionState| {
        println!("Peer Connection State has changed: {s}");

        if s == RTCPeerConnectionState::Failed {
            // Wait until PeerConnection has had no network activity for 30 seconds or another failure. It may be reconnected using an ICE Restart.
            // Use webrtc.PeerConnectionStateDisconnected if you are interested in detecting faster timeout.
            // Note that the PeerConnection may come back from PeerConnectionStateDisconnected.
            println!("Peer Connection has gone to failed exiting");
            let _ = done_tx.try_send(());
        }

        Box::pin(async {})
    }));

    // Wait for the offer to be pasted
    let offer = {
        let mut test_json = File::open("test.json").unwrap();
        let mut line: String = String::with_capacity(1024);
        let s = test_json.read_to_string(&mut line).unwrap();
        assert!(s <= 1024);

        serde_json::from_str::<RTCSessionDescription>(line.as_str()).unwrap()
    };

    // Set the remote SessionDescription
    peer_connection.set_remote_description(offer).await.unwrap();

    // Create an answer
    let answer = peer_connection.create_answer(None).await.unwrap();

    // Create channel that is blocked until ICE Gathering is complete
    let mut gather_complete = peer_connection.gathering_complete_promise().await;

    // Sets the LocalDescription, and starts our UDP listeners
    peer_connection.set_local_description(answer).await.unwrap();

    // Block until ICE Gathering is complete, disabling trickle ICE
    // we do this because we only can exchange one signaling message
    // in a production application you should exchange ICE Candidates via OnICECandidate
    let _ = gather_complete.recv().await;

    // Output the answer in base64 so we can paste it in browser
    if let Some(local_desc) = peer_connection.local_description().await {
        let json_str = serde_json::to_string(&local_desc).unwrap();
        println!("{json_str}");
    } else {
        println!("generate local_description failed!");
    }

    println!("Press ctrl-c to stop");
    tokio::select! {
        _ = done_rx.recv() => {
            println!("received done signal!");
        }
        _ = tokio::signal::ctrl_c() => {
            println!();
        }
    };

    peer_connection.close().await.expect("failed to close");
}
