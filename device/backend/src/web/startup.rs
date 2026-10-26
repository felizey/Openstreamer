use super::rtc::AccessUnit;
use crate::input::{goggles3::bootstrap::Bootstrap, h264};
use std::time::Duration;

/// Private reference bootstrap for a new or lagged viewer. Never inject into
/// the shared live feed: existing decoders must keep their reference pictures.
#[derive(Default)]
pub(super) struct Startup {
    bootstrap: Bootstrap,
    pub ready: bool,
}

impl Startup {
    pub fn before(&mut self, au: &AccessUnit) -> Option<AccessUnit> {
        if self.ready {
            return None;
        }
        if au.keyframe {
            self.ready = true;
            return None;
        }
        let nals: Vec<_> = h264::nals(&au.data).collect();
        for (index, nal) in nals.iter().enumerate() {
            let end = nals.get(index + 1).map_or(au.data.len(), |next| next.start);
            let mut normalized = vec![0, 0, 0, 1];
            normalized.extend_from_slice(&au.data[nal.header..end]);
            if let Some(result) = self.bootstrap.before(&normalized) {
                match result {
                    Ok(primer) => {
                        self.ready = true;
                        eprintln!(
                            "[web] generated viewer startup IDR ({} bytes)",
                            primer.bytes.len()
                        );
                        return Some(AccessUnit {
                            data: primer.bytes.into(),
                            keyframe: true,
                            assembled: au
                                .assembled
                                .checked_sub(Duration::from_micros(16_667))
                                .unwrap_or(au.assembled),
                        });
                    }
                    Err(reason) => eprintln!("[web] viewer startup IDR unavailable: {reason}"),
                }
            }
        }
        None
    }
}

#[cfg(test)]
#[path = "startup.test.rs"]
mod tests;
