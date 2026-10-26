use crate::input::{goggles3::bootstrap::Bootstrap, h264};

pub(super) fn parameter_sets(data: &[u8]) -> Vec<u8> {
    let nals: Vec<_> = h264::nals(data).collect();
    let mut headers = Vec::new();
    for (index, nal) in nals.iter().enumerate() {
        if matches!(nal.kind, 7 | 8) {
            let end = nals.get(index + 1).map_or(data.len(), |next| next.start);
            headers.extend_from_slice(&[0, 0, 0, 1]);
            headers.extend_from_slice(&data[nal.header..end]);
        }
    }
    headers
}

/// A replacement HDMI decoder joins an already running stream. Its reference
/// pictures are independent of the source's one-time USB bootstrap.
#[derive(Default)]
pub(super) struct DecoderStartup {
    bootstrap: Bootstrap,
    pub ready: bool,
}

impl DecoderStartup {
    pub fn before(&mut self, data: &[u8]) -> Option<Vec<u8>> {
        if self.ready {
            return None;
        }
        let nals: Vec<_> = h264::nals(data).collect();
        if nals.iter().any(|nal| nal.kind == h264::IDR) {
            self.ready = true;
            return None;
        }
        for (index, nal) in nals.iter().enumerate() {
            let end = nals.get(index + 1).map_or(data.len(), |next| next.start);
            let normalized = [&[0, 0, 0, 1][..], &data[nal.header..end]].concat();
            if let Some(primer) = self.bootstrap.before(&normalized) {
                match primer {
                    Ok(primer) => {
                        self.ready = true;
                        eprintln!(
                            "[hdmi] generated decoder startup IDR after display restart ({} bytes)",
                            primer.bytes.len()
                        );
                        return Some(primer.bytes);
                    }
                    Err(reason) => eprintln!("[hdmi] waiting for source IDR: {reason}"),
                }
            }
        }
        None
    }
}

#[cfg(test)]
#[path = "startup.test.rs"]
mod tests;
