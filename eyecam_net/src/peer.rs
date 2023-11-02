use std::sync::Arc;

use byteorder::{ByteOrder, LittleEndian};
use tokio::sync::mpsc;

use webrtc::{
    api::{
        interceptor_registry::register_default_interceptors,
        media_engine::{MediaEngine, MIME_TYPE_H264},
        APIBuilder,
    },
    data_channel::data_channel_message::DataChannelMessage,
    ice_transport::{
        ice_candidate::RTCIceCandidate, ice_connection_state::RTCIceConnectionState,
        ice_server::RTCIceServer,
    },
    interceptor::registry::Registry,
    peer_connection::{
        configuration::RTCConfiguration, peer_connection_state::RTCPeerConnectionState,
        sdp::session_description::RTCSessionDescription,
    },
    rtp_transceiver::rtp_codec::RTCRtpCodecCapability,
    track::track_local::{track_local_static_sample::TrackLocalStaticSample, TrackLocal},
};

use crate::servos::Servos;
use crate::signal;

pub struct Connection {
    pub video_track: Arc<TrackLocalStaticSample>,
}

impl Connection {
    async fn initial_offer(
        incoming: &mut mpsc::Receiver<signal::Incoming>,
    ) -> Option<RTCSessionDescription> {
        while let Some(message) = incoming.recv().await {
            match message {
                signal::Incoming::Session(session) => return Some(session),
                _ => continue,
            }
        }
        None
    }

    pub async fn wait_for_new(name: &str) -> anyhow::Result<Self> {
        let broker = signal::Broker::new(name);
        let mut signal_receiver = broker.open_incoming_channel();

        let offer = loop {
            if let Some(o) = Self::initial_offer(&mut signal_receiver).await {
                break o;
            }
            tracing::warn!("Broker died while waiting for offer. Restarting it..");
            signal_receiver = broker.open_incoming_channel();
        };

        let mut m = MediaEngine::default();
        m.register_default_codecs().unwrap();

        let mut registry = Registry::new();
        registry = register_default_interceptors(registry, &mut m).unwrap();

        let api = APIBuilder::new()
            .with_media_engine(m)
            .with_interceptor_registry(registry)
            .build();

        let config = RTCConfiguration {
            ice_servers: vec![RTCIceServer {
                urls: vec![
                    "stun:stun.l.google.com:19302".to_string(),
                    "stun:global.stun.twilio.com:3478".to_string(),
                ],
                ..Default::default()
            }],
            ..Default::default()
        };

        let peer_connection = Arc::new(api.new_peer_connection(config).await?);

        let video_track = Arc::new(TrackLocalStaticSample::new(
            RTCRtpCodecCapability {
                mime_type: MIME_TYPE_H264.to_string(),
                ..Default::default()
            },
            "eye".into(),
            "camera".into(),
        ));

        // Add this newly created track to the PeerConnection
        let rtp_sender = peer_connection
            .add_track(Arc::clone(&video_track) as Arc<dyn TrackLocal + Send + Sync>)
            .await?;

        // Read incoming RTCP packets
        // Before these packets are returned they are processed by interceptors. For things
        // like NACK this needs to be called.
        tokio::spawn(async move {
            let mut rtcp_buf = vec![0u8; 2048];
            tracing::debug!("RTP SENDER READ?");
            while let Ok((_, _)) = rtp_sender.read(&mut rtcp_buf).await {}
        });

        // This fires when connection state updates.
        let (state_tx, mut state_rx) = mpsc::channel(1);

        // Data channel
        let position_channel = peer_connection
            .create_data_channel(
                "position",
                Some(
                    webrtc::data_channel::data_channel_init::RTCDataChannelInit {
                        ordered: Some(false),
                        max_packet_life_time: None,
                        max_retransmits: Some(0),
                        protocol: None,
                        negotiated: Some(1),
                    },
                ),
            )
            .await?;

        position_channel.on_close(Box::new(move || {
            tracing::warn!("Data channel closed");
            // Just exit for now
            std::process::exit(52);
            // Box::pin(async {})
        }));

        // let d = position_channel.clone();
        position_channel.on_open(Box::new(move || {
            tracing::info!("Data channel open.");

            Box::pin(async {})
            // Box::pin(async move {
            //     let mut result = anyhow::Result::<usize>::Ok(0);
            //     while result.is_ok() {
            //         let timeout = tokio::time::sleep(Duration::from_secs(5));
            //         tokio::pin!(timeout);

            //         tokio::select! {
            //             _ = timeout.as_mut() =>{
            //                 let message = math_rand_alpha(15);
            //                 tracing::info!("Sending '{message}'");
            //                 result = d.send_text(message).await.map_err(Into::into);
            //             }
            //         };
            //     }
            // })
        }));

        // Send incoming message out for the servo (connected indirectly)
        let (pos_tx, mut pos_rx) = mpsc::channel(512);
        position_channel.on_message(Box::new(move |msg: DataChannelMessage| {
            tracing::debug!("Message from DataChannel: '{:?}'", msg.data);
            let tx = pos_tx.clone();
            Box::pin(async move {
                match tx.send(msg.data).await {
                    Ok(()) => {}
                    Err(e) => {
                        tracing::warn!("Position listener closed? {e:?}");
                    }
                }
            })
        }));
        std::thread::spawn(move || {
            let servos = Servos::new();

            while let Some(pos_data) = pos_rx.blocking_recv() {
                let y = LittleEndian::read_f32(&pos_data[0..4]);
                let x = LittleEndian::read_f32(&pos_data[4..8]);

                servos.set_rotation_x(x);
                servos.set_rotation_y(y);
            }

            tracing::info!("Servos thread exiting");
        });

        // Set the handler for ICE connection state
        // This will notify you when the peer has connected/disconnected
        peer_connection.on_ice_connection_state_change(Box::new(
            move |connection_state: RTCIceConnectionState| {
                tracing::info!("ICE Connection State has changed {connection_state}");

                match connection_state {
                    RTCIceConnectionState::Connected
                    | RTCIceConnectionState::Failed
                    | RTCIceConnectionState::Closed
                    | RTCIceConnectionState::Disconnected => {
                        let _ = state_tx.try_send(connection_state);
                    }

                    _ => {}
                }
                Box::pin(async {})
            },
        ));

        // Set the handler for Peer connection state
        // This will notify you when the peer has connected/disconnected
        peer_connection.on_peer_connection_state_change(Box::new(
            move |s: RTCPeerConnectionState| {
                tracing::info!("Peer Connection State has changed: {s}");

                if s == RTCPeerConnectionState::Disconnected {
                    tracing::warn!("Peer Connection has gone to failed");
                    std::process::exit(50);
                }

                Box::pin(async {})
            },
        ));

        // Open return connection for sending signals back
        let signal_sender = broker.open_outgoing_channel();

        // When we get a new ice candidate, "trickle" it to the peer
        let candidate_sender = signal_sender.clone();
        peer_connection.on_ice_candidate(Box::new(move |candidate: Option<RTCIceCandidate>| {
            let sender = candidate_sender.clone();
            Box::pin(async move {
                tracing::info!("Sending candidate {candidate:?}");
                if let Err(e) = sender.send(signal::Outgoing::Candidate(candidate)).await {
                    tracing::error!("Failed to signal ice candidate {e:?}");
                }
            })
        }));

        // When the peer gets new ice candidates, add them
        let pc = peer_connection.clone();
        tokio::spawn(async move {
            // TODO: if there's no signal for an hour, this receiver will be closed
            // and this task will quit.
            while let Some(message) = signal_receiver.recv().await {
                match message {
                    signal::Incoming::Session(desc) => {
                        if let Err(e) = pc.set_remote_description(desc).await {
                            tracing::error!("Failed to update remote description {e:?}");
                        }
                    }
                    signal::Incoming::Candidate(ice_candidate) => {
                        if let Err(e) = pc.add_ice_candidate(ice_candidate).await {
                            tracing::error!("Failed to add ice candidate {e:?}");
                        }
                    }
                }
            }
            tracing::info!("Signal task exiting.");
        });

        peer_connection.set_remote_description(offer).await?;
        let answer = peer_connection.create_answer(None).await?;
        peer_connection.set_local_description(answer).await?;

        let Some(our_desc) = peer_connection.local_description().await else {
            panic!("TODO handle this..");
        };
        signal_sender
            .send(signal::Outgoing::Session(our_desc))
            .await?;

        let state = state_rx.recv().await;
        tracing::info!("RTC state: {state:?}");

        if let Some(RTCIceConnectionState::Connected) = state {
            Ok(Self { video_track })
        } else {
            anyhow::bail!("Failed to connect");
        }
    }
}
