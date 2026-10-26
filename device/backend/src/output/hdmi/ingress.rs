use crate::input::h264::AccessUnitAssembler;
use crate::settings::InputMode;

/// Independent H.264 streams must not share decoder references or partial AUs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Feed {
    Live(InputMode, u64),
    Splash,
}

pub(super) struct Ingress {
    pub assembler: AccessUnitAssembler,
    feed: Option<Feed>,
    discont: bool,
}

impl Ingress {
    pub fn new() -> Self {
        Self {
            assembler: AccessUnitAssembler::new(),
            feed: None,
            discont: true,
        }
    }

    /// Returns whether an existing decoder belongs to a different stream.
    /// The first feed uses the fresh pipeline without another rebuild.
    pub fn select(&mut self, feed: Feed) -> bool {
        if self.feed == Some(feed) {
            return false;
        }
        let rebuild = self.feed.is_some();
        self.discard_partial();
        self.feed = Some(feed);
        rebuild
    }

    /// A newly built pipeline has no previous stream or pending access unit.
    pub fn reset(&mut self) {
        self.feed = None;
        self.discard_partial();
    }

    /// Keep the stream identity while discarding data from a failed decoder.
    pub fn discard_partial(&mut self) {
        self.assembler = AccessUnitAssembler::new();
        self.discont = true;
    }

    pub fn take_discont(&mut self) -> bool {
        std::mem::take(&mut self.discont)
    }
}

#[cfg(test)]
#[path = "ingress.test.rs"]
mod tests;
