//! DJI Goggles 3 USB-A: the Pi hosts the goggles' USB network device.
//! This source receives H.264 only; aircraft commands still belong to DUML.

pub(crate) mod bootstrap;
mod network;
pub mod protocol;

use std::io;
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;

use super::{Fanout, InputState};
use crate::settings::{Goggles3Settings, InputMode};
use protocol::{Channel, VideoAssembler};

const TICK: Duration = Duration::from_millis(100);
const POLL: Duration = Duration::from_millis(50);
const RETRY: Duration = Duration::from_secs(1);
const RX_GONE: Duration = Duration::from_secs(3);
const VIDEO_GONE: Duration = Duration::from_secs(2);
const DIAGNOSTIC_SUMMARY: Duration = Duration::from_secs(2);
const IDR_NOTICE: Duration = Duration::from_secs(5);

/// The same identity check also keeps these goggles out of the phone tether UI.
pub(crate) fn is_usb_network_interface(name: &str) -> bool {
    network::is_usb_network_interface(name)
}

#[derive(Serialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Goggles3State {
    #[default]
    Idle,
    Waiting,
    Connecting,
    Live,
    Error,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Goggles3Status {
    pub state: Goggles3State,
    pub interface: Option<String>,
    pub peer: Option<String>,
    pub error: Option<String>,
}

pub struct Goggles3Source {
    state: Arc<InputState>,
    config: Mutex<Goggles3Settings>,
    generation: AtomicU64,
}

impl Goggles3Source {
    pub fn start(state: Arc<InputState>, cfg: &Goggles3Settings, fanout: Arc<Fanout>) -> Arc<Self> {
        let source = Arc::new(Self {
            state,
            config: Mutex::new(cfg.clone()),
            generation: AtomicU64::new(0),
        });
        let this = Arc::clone(&source);
        thread::spawn(move || {
            let mut last_driver_attempt = None;
            let mut recovered = false;
            loop {
                let (cfg, generation) = {
                    let config = this.config.lock().unwrap();
                    (config.clone(), this.generation.load(Ordering::Acquire))
                };
                if !recovered {
                    match network::recover_profiles() {
                        Ok(()) => recovered = true,
                        Err(error) => {
                            this.set(Goggles3State::Error, None, Some(error));
                            thread::sleep(RETRY);
                            continue;
                        }
                    }
                }
                if this.current(generation) {
                    if let Err(error) = this.session(&cfg, generation, &fanout) {
                        this.set(Goggles3State::Error, None, Some(error));
                    }
                    if this.current(generation)
                        && last_driver_attempt
                            .is_none_or(|at: Instant| at.elapsed() > Duration::from_secs(30))
                    {
                        // The kernel owns the USB network interface; never claim its endpoints.
                        network::load_usb_drivers_if_needed();
                        last_driver_attempt = Some(Instant::now());
                    }
                } else {
                    this.set(Goggles3State::Idle, None, None);
                }
                let until = Instant::now() + RETRY;
                while Instant::now() < until {
                    if this.generation.load(Ordering::Acquire) != generation {
                        break;
                    }
                    thread::sleep(TICK);
                }
            }
        });
        source
    }

    pub fn apply(&self, cfg: &Goggles3Settings) {
        let mut current = self.config.lock().unwrap();
        if *current != *cfg {
            *current = cfg.clone();
            self.generation.fetch_add(1, Ordering::Release);
        }
    }

    pub fn status(&self) -> Goggles3Status {
        self.state.goggles3.lock().unwrap().clone()
    }

    fn current(&self, generation: u64) -> bool {
        self.state.mode() == InputMode::DjiGoggles3
            && self.generation.load(Ordering::Acquire) == generation
    }

    fn set(&self, state: Goggles3State, interface: Option<&str>, error: Option<String>) {
        let next = Goggles3Status {
            state,
            interface: interface.map(str::to_owned),
            peer: interface.map(|_| peer().to_string()),
            error,
        };
        let mut status = self.state.goggles3.lock().unwrap();
        if next.error.is_some() && next.error != status.error {
            eprintln!("[goggles3] {}", next.error.as_deref().unwrap_or_default());
        }
        *status = next;
    }

    fn session(
        &self,
        cfg: &Goggles3Settings,
        generation: u64,
        fanout: &Fanout,
    ) -> Result<(), String> {
        let Some(interface) = network::resolve(cfg)? else {
            self.set(Goggles3State::Waiting, None, None);
            return Ok(());
        };
        let _usb = network::UsbNetwork::prepare(interface.clone(), || self.current(generation))?;
        while self.current(generation) && interface.present() {
            let socket = socket(&interface.name).map_err(|e| format!("UDP socket: {e}"))?;
            // The old HDMI parser/decoder must not survive a new goggles session,
            // even if a replug was quicker than the splash video timeout.
            if !fanout.begin_stream(InputMode::DjiGoggles3) {
                return Ok(());
            }
            println!(
                "[goggles3] UDP session on {}: {} -> {}",
                interface.name,
                socket
                    .local_addr()
                    .map_err(|e| format!("UDP address: {e}"))?,
                peer(),
            );
            self.set(Goggles3State::Connecting, Some(&interface.name), None);
            stream(
                &socket,
                peer(),
                || self.current(generation),
                |state| self.set(state, Some(&interface.name), None),
                |nal| {
                    fanout.video(InputMode::DjiGoggles3, nal);
                },
            )
            .map_err(|e| format!("{}: {e}", interface.name))?;
            // Silence restarts the UDP protocol without toggling the USB network.
        }
        Ok(())
    }
}

fn peer() -> SocketAddr {
    SocketAddr::from((Ipv4Addr::new(192, 168, 60, 2), 9003))
}

fn socket(interface: &str) -> io::Result<UdpSocket> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    let mut device = interface.as_bytes().to_vec();
    device.push(0);
    // SAFETY: both pointers refer to live, correctly sized buffers for setsockopt.
    if unsafe {
        libc::setsockopt(
            socket.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_BINDTODEVICE,
            device.as_ptr().cast(),
            device.len() as libc::socklen_t,
        )
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    let receive_bytes: libc::c_int = 2 * 1024 * 1024;
    // SAFETY: receive_bytes is a live integer with the advertised option size.
    if unsafe {
        libc::setsockopt(
            socket.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_RCVBUF,
            (&receive_bytes as *const libc::c_int).cast(),
            std::mem::size_of_val(&receive_bytes) as libc::socklen_t,
        )
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    socket.set_read_timeout(Some(POLL))?;
    socket.set_write_timeout(Some(Duration::from_millis(250)))?;
    Ok(socket)
}

#[derive(Default, Debug, PartialEq, Eq)]
struct NalCounts {
    total: u64,
    vcl: u64,
    sps: u64,
    pps: u64,
    idr: u64,
}

/// Observes real completed input NALs, independently of decoder readiness.
/// There may be several VCL/IDR slices in one picture, so these are not FPS.
#[derive(Default)]
struct SessionDiagnostics {
    counts: NalCounts,
    first_vcl: Option<Duration>,
    summary_sent: bool,
    idr_notice_sent: bool,
}

impl SessionDiagnostics {
    fn observe(&mut self, nal: &[u8], elapsed: Duration) -> Option<&'static str> {
        let header = *nal.get(4)?;
        let kind = header & 0x1f;
        if header & 0x80 != 0 || !(1..=23).contains(&kind) {
            return None;
        }
        self.counts.total += 1;
        if (1..=5).contains(&kind) {
            self.counts.vcl += 1;
            self.first_vcl.get_or_insert(elapsed);
        }
        let (count, name) = match kind {
            7 => (&mut self.counts.sps, "SPS"),
            8 => (&mut self.counts.pps, "PPS"),
            5 => (&mut self.counts.idr, "IDR"),
            _ => return None,
        };
        *count += 1;
        (*count == 1).then_some(name)
    }

    fn summary_due(&mut self, elapsed: Duration) -> bool {
        if !self.summary_sent && (self.first_vcl.is_some() || elapsed >= DIAGNOSTIC_SUMMARY) {
            self.summary_sent = true;
            true
        } else {
            false
        }
    }

    fn idr_notice_due(&mut self, elapsed: Duration) -> bool {
        if !self.idr_notice_sent
            && self.counts.idr == 0
            && self
                .first_vcl
                .is_some_and(|first| elapsed.saturating_sub(first) >= IDR_NOTICE)
        {
            self.idr_notice_sent = true;
            true
        } else {
            false
        }
    }

    fn report(&mut self, elapsed: Duration, port: u16) {
        if self.summary_due(elapsed) {
            println!(
                "[goggles3] UDP {port}: input after {} ms: NALs={}, VCL={}, SPS={}, PPS={}, IDR={}",
                elapsed.as_millis(),
                self.counts.total,
                self.counts.vcl,
                self.counts.sps,
                self.counts.pps,
                self.counts.idr,
            );
        }
        if self.idr_notice_due(elapsed) {
            eprintln!(
                "[goggles3] UDP {port}: no IDR observed in the first 5 s of video; SPS={}, PPS={}, VCL={} (source NAL counts; generated startup IDR excluded)",
                self.counts.sps, self.counts.pps, self.counts.vcl,
            );
        }
    }
}

/// One bounded protocol session. Only packets from the selected goggles count.
/// The callbacks keep protocol tests independent of hardware and the output graph.
fn stream(
    socket: &UdpSocket,
    peer: SocketAddr,
    mut current: impl FnMut() -> bool,
    mut progress: impl FnMut(Goggles3State),
    mut video: impl FnMut(&[u8]),
) -> io::Result<()> {
    let mut buffer = [0_u8; 65_536];
    let mut assembler = VideoAssembler::new();
    let mut diagnostics = SessionDiagnostics::default();
    let mut bootstrap = bootstrap::Bootstrap::default();
    let started = Instant::now();
    let port = socket.local_addr()?.port();
    let mut control_ready = false;
    let mut header = None;
    let mut last_tick = Instant::now() - TICK;
    let mut last_rx = Instant::now();
    let mut last_video: Option<Instant> = None;
    while current() {
        if last_tick.elapsed() >= TICK {
            if control_ready {
                if let Some(header) = header {
                    let ack = protocol::ack(header);
                    socket.send_to(&ack, peer)?;
                }
            } else {
                socket.send_to(&protocol::DISCOVERY, peer)?;
            }
            last_tick = Instant::now();
        }
        match socket.recv_from(&mut buffer) {
            Ok((length, from)) if from.ip() == peer.ip() => {
                if let Some(packet) = protocol::parse_packet(&buffer[..length]) {
                    last_rx = Instant::now();
                    header = Some(packet.header);
                    match packet.header.channel {
                        Channel::Control => {
                            if !control_ready {
                                control_ready = true;
                                if last_video.is_none() {
                                    progress(Goggles3State::Waiting);
                                }
                            }
                        }
                        Channel::Video => {
                            if last_video.is_some_and(|at| {
                                at.elapsed() > crate::output::splash::VIDEO_TIMEOUT
                            }) {
                                assembler.reset();
                                bootstrap = bootstrap::Bootstrap::default();
                                last_video = None;
                            }
                            for nal in assembler.push(packet.payload) {
                                if !current() {
                                    return Ok(());
                                }
                                let elapsed = started.elapsed();
                                if let Some(kind) = diagnostics.observe(&nal, elapsed) {
                                    println!(
                                        "[goggles3] UDP {port}: first {kind} NAL at {} ms",
                                        elapsed.as_millis(),
                                    );
                                }
                                if let Some(primer) = bootstrap.before(&nal) {
                                    match primer {
                                        Ok(primer) => {
                                            println!(
                                                "[goggles3] UDP {port}: generated startup IDR {}x{} ({} bytes); source has not supplied an IDR",
                                                primer.coded_width,
                                                primer.coded_height,
                                                primer.bytes.len()
                                            );
                                            video(&primer.bytes);
                                        }
                                        Err(reason) => eprintln!(
                                            "[goggles3] UDP {port}: startup IDR unavailable: {reason}"
                                        ),
                                    }
                                }
                                video(&nal);
                                let ack = protocol::ack(packet.header);
                                socket.send_to(&ack, peer)?;
                                // Parameter sets/AUDs alone do not mean live pictures.
                                if nal
                                    .get(4)
                                    .is_some_and(|header| (1..=5).contains(&(header & 0x1f)))
                                {
                                    if last_video.is_none() {
                                        progress(Goggles3State::Live);
                                    }
                                    last_video = Some(Instant::now());
                                }
                            }
                        }
                    }
                }
            }
            Ok(_) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock
                        | io::ErrorKind::TimedOut
                        | io::ErrorKind::Interrupted
                ) => {}
            Err(error) => return Err(error),
        }
        diagnostics.report(started.elapsed(), port);
        if last_video.is_some_and(|at| at.elapsed() > VIDEO_GONE) {
            last_video = None;
            assembler.reset();
            bootstrap = bootstrap::Bootstrap::default();
            progress(Goggles3State::Waiting);
        }
        if last_rx.elapsed() > RX_GONE {
            return Ok(());
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "goggles3/test.rs"]
mod tests;
