use gst::prelude::*;
use gstreamer as gst;
use std::sync::{Arc, Mutex};

// kmssink 1.26 caches a DRM framebuffer in GstMemory qdata; that cache is not
// tee-safe. Give each branch its own memory wrappers, keeping the same DMA
// buffer and retaining the decoder's parent buffer until scanout releases it.
fn separate(buffer: &gst::Buffer) -> Result<gst::Buffer, &'static str> {
    let mut copy = buffer.copy();
    let out = copy.get_mut().expect("new buffer");
    out.remove_all_memory();
    for memory in buffer.iter_memories() {
        let memory = if memory.is_type("dmabuf") {
            if memory.flags().contains(gst::MemoryFlags::NO_SHARE) {
                return Err("The decoder supplied a non-shareable DMA buffer");
            }
            memory.share(..)
        } else {
            memory.to_owned()
        };
        out.append_memory(memory);
    }
    gst::ParentBufferMeta::add(out, buffer);
    Ok(copy)
}

pub(super) fn isolate(
    pipeline: &gst::Pipeline,
    index: usize,
    render: Arc<Mutex<()>>,
) -> Result<(), String> {
    let identity = pipeline
        .by_name(&format!("mirror{index}"))
        .ok_or("Missing HDMI mirror branch")?;
    let pad = identity
        .static_pad("src")
        .ok_or("Missing HDMI mirror pad")?;
    pad.add_probe(gst::PadProbeType::BUFFER, move |pad, info| {
        let Some(gst::PadProbeData::Buffer(buffer)) = info.data.take() else {
            return gst::PadProbeReturn::Ok;
        };
        // Both sinks use the same DRM file's GEM handle namespace. Serialize
        // their import/addfb/close cycle; simultaneous PRIME imports can return
        // the same handle and one sink can close it while the other adds its FB.
        let _render = render.lock().unwrap();
        let result = match separate(&buffer) {
            Ok(buffer) => pad.peer().ok_or("Missing HDMI sink").and_then(|peer| {
                peer.chain(buffer)
                    .map(|_| ())
                    .map_err(|_| "HDMI mirror sink rejected the frame")
            }),
            Err(error) => Err(error),
        };
        if let Err(error) = result
            && let Some(parent) = pad.parent_element()
        {
            gst::element_error!(parent, gst::ResourceError::Failed, ("{error}"));
        }
        // The frame has already been chained to the sink while holding render.
        // Events and queries keep following the normal linked pad path.
        gst::PadProbeReturn::Handled
    });
    Ok(())
}

#[cfg(test)]
#[path = "mirror.test.rs"]
mod tests;
