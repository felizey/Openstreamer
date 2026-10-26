use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use axum::{Json, extract::State, http::StatusCode};
use tokio::sync::{broadcast, mpsc};

use webrtc::api::{
    API, APIBuilder,
    interceptor_registry::register_default_interceptors,
    media_engine::{MIME_TYPE_H264, MediaEngine},
};
use webrtc::interceptor::registry::Registry;
use webrtc::media::Sample;
use webrtc::peer_connection::{
    configuration::RTCConfiguration, peer_connection_state::RTCPeerConnectionState,
    sdp::session_description::RTCSessionDescription,
};
use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability;
use webrtc::track::track_local::TrackLocal;
use webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample;

use super::AppState;
use super::startup::Startup;
use crate::input::h264::{self, AccessUnitAssembler};

#[derive(Clone)]
pub(super) struct AccessUnit {
    // bytes::Bytes clones are O(1) (refcounted, no copy) - unlike
    // Arc<Vec<u8>>, which still needs a full deep copy wherever the raw
    // Vec<u8> gets dereferenced and cloned (see client_sender below)
    pub(super) data: bytes::Bytes,
    pub(super) keyframe: bool,
    pub(super) assembled: Instant,
}

pub(super) fn build_api() -> anyhow::Result<API> {
    let mut m = MediaEngine::default();
    m.register_default_codecs()?;
    let mut registry = Registry::new();
    registry = register_default_interceptors(registry, &mut m)?;
    Ok(APIBuilder::new()
        .with_media_engine(m)
        .with_interceptor_registry(registry)
        .build())
}

pub(super) async fn assemble_loop(
    mut rx: mpsc::UnboundedReceiver<Vec<u8>>,
    au_tx: broadcast::Sender<AccessUnit>,
) {
    let mut assembler = AccessUnitAssembler::new();
    while let Some(chunk) = rx.recv().await {
        assembler.push(&chunk);
        while let Some(bytes) = assembler.next_au() {
            let keyframe = h264::nals(&bytes).any(|nal| nal.kind == h264::IDR);
            let _ = au_tx.send(AccessUnit {
                data: bytes::Bytes::from(bytes),
                keyframe,
                assembled: Instant::now(),
            });
        }
    }
}

// Raising this past the ~29 AU GOP length was tried and made things worse
const MAX_BACKLOG: usize = 9;
static NEXT_CLIENT_ID: AtomicUsize = AtomicUsize::new(1);

async fn client_sender(
    mut au_rx: broadcast::Receiver<AccessUnit>,
    track: Arc<TrackLocalStaticSample>,
    mut connected: tokio::sync::watch::Receiver<bool>,
    mut keyframe_request: tokio::sync::watch::Receiver<u64>,
) {
    let id = NEXT_CLIENT_ID.fetch_add(1, Ordering::Relaxed);
    let mut pending: VecDeque<AccessUnit> = VecDeque::new();
    let mut last = Instant::now();
    let mut started = false;
    let mut startup = Startup::default();

    // write_sample() timing stats, reported once a second - to see whether
    // per-frame send cost (RTP packetization + SRTP encryption) is what's
    // eating CPU, and whether keyframes (much larger) dominate that cost
    let mut wr_count: u64 = 0;
    let mut wr_total = Duration::ZERO;
    let mut wr_max = Duration::ZERO;
    let mut wr_bytes: u64 = 0;
    let mut kf_count: u64 = 0;
    let mut kf_total = Duration::ZERO;
    let mut kf_max = Duration::ZERO;
    let mut stats_last = Instant::now();

    loop {
        let next = tokio::select! {
            changed = connected.changed() => {
                if changed.is_err() || !*connected.borrow_and_update() { return; }
                continue;
            },
            changed = keyframe_request.changed() => {
                if changed.is_err() { return; }
                pending.clear();
                started = false;
                startup = Startup::default();
                continue;
            },
            next = au_rx.recv() => next,
        };
        match next {
            Ok(au) => pending.push_back(au),
            Err(broadcast::error::RecvError::Closed) => return,
            Err(broadcast::error::RecvError::Lagged(n)) => {
                eprintln!("[webrtc {id}] lagged, dropped {n} AUs");
                pending.clear();
                started = false;
                startup = Startup::default();
                continue;
            }
        }
        loop {
            match au_rx.try_recv() {
                Ok(au) => pending.push_back(au),
                Err(broadcast::error::TryRecvError::Empty) => break,
                Err(broadcast::error::TryRecvError::Lagged(n)) => {
                    eprintln!("[webrtc {id}] lagged, dropped {n} AUs");
                    pending.clear();
                    started = false;
                    startup = Startup::default();
                    break;
                }
                Err(broadcast::error::TryRecvError::Closed) => return,
            }
        }

        if !started {
            while let Some(au) = pending.pop_front() {
                let primer = startup.before(&au);
                if startup.ready {
                    pending.push_front(au);
                    if let Some(primer) = primer {
                        pending.push_front(primer);
                    }
                    started = true;
                    last = Instant::now();
                    break;
                }
            }
            if !started {
                continue;
            }
        }

        if pending.len() > MAX_BACKLOG {
            let backlog_len = pending.len();
            if let Some(idx) = last_keyframe_index(&pending) {
                if crate::settings::verbose() {
                    println!("[webrtc {id}] backlog={backlog_len}, skipping {idx} AUs to keyframe");
                }
                for _ in 0..idx {
                    pending.pop_front();
                }
            } else {
                // No keyframe anywhere in the backlog - the real GOP length
                // (~29 AUs) is far longer than MAX_BACKLOG, so trimming to
                // MAX_BACKLOG here would keep P-frames that reference AUs
                // we're about to discard, corrupting decode
                if crate::settings::verbose() {
                    println!("[webrtc {id}] backlog={backlog_len}, no keyframe, waiting for next");
                }
                pending.clear();
                started = false;
                startup = Startup::default();
                continue;
            }
        }

        while let Some(au) = pending.pop_front() {
            let now = Instant::now();
            let mut dur = now.duration_since(last);
            if dur < Duration::from_millis(1) {
                dur = Duration::from_micros(16_667);
            } else if dur > Duration::from_millis(250) {
                dur = Duration::from_millis(250);
            }
            last = now;

            let sample_len = au.data.len() as u64;
            let is_keyframe = au.keyframe;
            let sample = Sample {
                data: au.data.clone(),
                duration: dur,
                ..Default::default()
            };

            let t0 = Instant::now();
            let res = track.write_sample(&sample).await;
            let elapsed = t0.elapsed();

            wr_count += 1;
            wr_total += elapsed;
            wr_bytes += sample_len;
            if elapsed > wr_max {
                wr_max = elapsed;
            }
            if is_keyframe {
                kf_count += 1;
                kf_total += elapsed;
                if elapsed > kf_max {
                    kf_max = elapsed;
                }
            }

            if res.is_err() {
                return;
            }
        }

        if stats_last.elapsed() >= Duration::from_secs(1) {
            if wr_count > 0 && crate::settings::verbose() {
                let avg = wr_total / wr_count as u32;
                let kf_avg = if kf_count > 0 {
                    kf_total / kf_count as u32
                } else {
                    Duration::ZERO
                };
                println!(
                    "[webrtc {id}] write_sample: {wr_count} calls, {wr_bytes} bytes, avg={avg:?} max={wr_max:?} | keyframes: {kf_count} calls avg={kf_avg:?} max={kf_max:?}"
                );
            }
            wr_count = 0;
            wr_total = Duration::ZERO;
            wr_max = Duration::ZERO;
            wr_bytes = 0;
            kf_count = 0;
            kf_total = Duration::ZERO;
            kf_max = Duration::ZERO;
            stats_last = Instant::now();
        }
    }
}

fn last_keyframe_index(aus: &VecDeque<AccessUnit>) -> Option<usize> {
    let mut found = None;
    for (i, au) in aus.iter().enumerate() {
        if au.keyframe {
            found = Some(i);
        }
    }
    found
}

#[derive(serde::Deserialize)]
pub(super) struct OfferReq {
    sdp: String,
    #[serde(rename = "type")]
    _typ: String,
}

#[derive(serde::Serialize)]
pub(super) struct AnswerResp {
    sdp: String,
    #[serde(rename = "type")]
    typ: String,
}

pub(super) async fn offer_handler(
    State(state): State<AppState>,
    Json(offer): Json<OfferReq>,
) -> Result<Json<AnswerResp>, StatusCode> {
    if state.viewer_limit_reached().await {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    match negotiate(state, offer).await {
        Ok(answer) => Ok(Json(answer)),
        Err(e) => {
            eprintln!("[web] negotiate error: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

async fn negotiate(state: AppState, offer: OfferReq) -> anyhow::Result<AnswerResp> {
    let config = RTCConfiguration {
        ice_servers: vec![],
        ..Default::default()
    };

    let pc = Arc::new(state.api.new_peer_connection(config).await?);

    let track = Arc::new(TrackLocalStaticSample::new(
        RTCRtpCodecCapability {
            mime_type: MIME_TYPE_H264.to_owned(),
            ..Default::default()
        },
        "video".to_owned(),
        "openstreamer".to_owned(),
    ));

    let rtp_sender = pc
        .add_track(Arc::clone(&track) as Arc<dyn TrackLocal + Send + Sync>)
        .await?;
    let (request_tx, request_rx) = tokio::sync::watch::channel(0u64);
    tokio::spawn(async move {
        use webrtc::rtcp::payload_feedbacks::{
            full_intra_request::FullIntraRequest, picture_loss_indication::PictureLossIndication,
        };
        let mut last_request: Option<Instant> = None;
        while let Ok((packets, _)) = rtp_sender.read_rtcp().await {
            if packets.iter().any(|packet| {
                packet.as_any().is::<PictureLossIndication>()
                    || packet.as_any().is::<FullIntraRequest>()
            }) && last_request.is_none_or(|at| at.elapsed() >= Duration::from_secs(1))
            {
                request_tx.send_modify(|generation| *generation = generation.wrapping_add(1));
                last_request = Some(Instant::now());
            }
        }
    });

    let (connected_tx, mut connected_rx) = tokio::sync::watch::channel(false);
    let au_tx = state.au_tx.clone();
    tokio::spawn(async move {
        while !*connected_rx.borrow_and_update() {
            if connected_rx.changed().await.is_err() {
                return;
            }
        }
        // Subscribing after ICE/DTLS connection keeps the primer from being
        // consumed before the track has a negotiated browser binding.
        client_sender(au_tx.subscribe(), track, connected_rx, request_rx).await;
    });

    let peers_for_cb = Arc::clone(&state.peers);
    let pc_weak = Arc::downgrade(&pc);
    pc.on_peer_connection_state_change(Box::new(move |s: RTCPeerConnectionState| {
        let peers_for_cb = Arc::clone(&peers_for_cb);
        let pc_weak = pc_weak.clone();
        let _ = connected_tx.send(s == RTCPeerConnectionState::Connected);
        Box::pin(async move {
            if matches!(
                s,
                RTCPeerConnectionState::Failed
                    | RTCPeerConnectionState::Disconnected
                    | RTCPeerConnectionState::Closed
            ) && let Some(pc) = pc_weak.upgrade()
            {
                let mut peers = peers_for_cb.lock().await;
                peers.retain(|p| !Arc::ptr_eq(p, &pc));
            }
        })
    }));

    let remote = RTCSessionDescription::offer(offer.sdp)?;
    pc.set_remote_description(remote).await?;

    let answer = pc.create_answer(None).await?;
    let mut gather_complete = pc.gathering_complete_promise().await;
    pc.set_local_description(answer).await?;
    let _ = gather_complete.recv().await;

    let local = pc
        .local_description()
        .await
        .ok_or_else(|| anyhow::anyhow!("no local description"))?;

    state.peers.lock().await.push(pc);

    Ok(AnswerResp {
        sdp: local.sdp,
        typ: "answer".to_owned(),
    })
}
