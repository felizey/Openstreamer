use super::*;
use gst::glib::translate::{ToGlibPtr, from_glib_full};

#[link(name = "gstallocators-1.0")]
unsafe extern "C" {
    fn gst_dmabuf_allocator_new() -> *mut gst::ffi::GstAllocator;
    fn gst_dmabuf_allocator_alloc(
        allocator: *mut gst::ffi::GstAllocator,
        fd: i32,
        size: usize,
    ) -> *mut gst::ffi::GstMemory;
    fn gst_dmabuf_memory_get_fd(memory: *mut gst::ffi::GstMemory) -> i32;
}

fn dma_buffer() -> gst::Buffer {
    gst::init().unwrap();
    let fd = unsafe { libc::memfd_create(c"hdmi-mirror-test".as_ptr(), libc::MFD_CLOEXEC) };
    assert!(fd >= 0);
    assert_eq!(unsafe { libc::ftruncate(fd, 16) }, 0);
    let bytes = [7u8; 16];
    assert_eq!(
        unsafe { libc::pwrite(fd, bytes.as_ptr().cast(), bytes.len(), 0) },
        16
    );
    let allocator: gst::Allocator = unsafe { from_glib_full(gst_dmabuf_allocator_new()) };
    let memory: gst::Memory = unsafe {
        from_glib_full(gst_dmabuf_allocator_alloc(
            allocator.to_glib_none().0,
            fd,
            16,
        ))
    };
    let mut buffer = gst::Buffer::new();
    let out = buffer.get_mut().unwrap();
    out.append_memory(memory);
    out.set_pts(gst::ClockTime::from_mseconds(42));
    out.set_flags(gst::BufferFlags::DISCONT);
    buffer
}

#[test]
fn branches_have_private_kms_cache_keys_but_share_the_same_dma_pixels() {
    let original = dma_buffer();
    let left = separate(&original).unwrap();
    let right = separate(&original).unwrap();
    let a = left.peek_memory(0);
    let b = right.peek_memory(0);
    let source = original.peek_memory(0);
    assert_ne!(a.as_ptr(), b.as_ptr());
    assert_ne!(a.as_ptr(), source.as_ptr());
    assert_ne!(b.as_ptr(), source.as_ptr());
    assert!(a.is_type("dmabuf") && b.is_type("dmabuf"));
    assert_eq!(
        unsafe { gst_dmabuf_memory_get_fd(a.as_ptr() as *mut _) },
        unsafe { gst_dmabuf_memory_get_fd(b.as_ptr() as *mut _) }
    );
    assert_eq!(left.pts(), original.pts());
    assert_eq!(left.flags(), original.flags());
    assert_eq!(
        left.meta::<gst::ParentBufferMeta>()
            .unwrap()
            .parent()
            .as_ptr(),
        original.as_ptr()
    );
    drop(original);
    drop(left);
    assert_eq!(right.map_readable().unwrap().as_slice(), &[7u8; 16]);
}

#[test]
fn independent_sink_branches_receive_the_same_picture() {
    use gstreamer_app::{AppSink, AppSrc};
    let original = dma_buffer();
    let description = format!(
        "appsrc name=src is-live=true format=time ! tee name=video {} {}",
        super::super::branch(0, crate::settings::Scaling::Fit, 1920, 1080, "appsink"),
        super::super::branch(1, crate::settings::Scaling::Fit, 1280, 720, "appsink")
    );
    let pipeline = gst::parse::launch(&description)
        .unwrap()
        .dynamic_cast::<gst::Pipeline>()
        .unwrap();
    let render = Arc::new(Mutex::new(()));
    isolate(&pipeline, 0, Arc::clone(&render)).unwrap();
    isolate(&pipeline, 1, render).unwrap();
    let source = pipeline
        .by_name("src")
        .unwrap()
        .dynamic_cast::<AppSrc>()
        .unwrap();
    source.set_caps(Some(
        &gst::Caps::builder("video/x-raw")
            .field("format", "NV12")
            .field("width", 4i32)
            .field("height", 2i32)
            .field("framerate", gst::Fraction::new(30, 1))
            .build(),
    ));
    pipeline.set_state(gst::State::Playing).unwrap();
    source.push_buffer(original.clone()).unwrap();
    let samples: Vec<_> = ["sink", "sink1"]
        .into_iter()
        .map(|name| {
            pipeline
                .by_name(name)
                .unwrap()
                .dynamic_cast::<AppSink>()
                .unwrap()
                .try_pull_sample(gst::ClockTime::from_seconds(2))
                .unwrap()
        })
        .collect();
    let left = samples[0].buffer().unwrap();
    let right = samples[1].buffer().unwrap();
    assert_ne!(left.peek_memory(0).as_ptr(), right.peek_memory(0).as_ptr());
    assert_eq!(
        left.map_readable().unwrap().as_slice(),
        right.map_readable().unwrap().as_slice()
    );
    assert_eq!(left.pts(), original.pts());
    assert_eq!(right.pts(), original.pts());
    pipeline.set_state(gst::State::Null).unwrap();
}
