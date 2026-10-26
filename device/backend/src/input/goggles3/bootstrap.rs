//! Prime a new decoder for the DJI stream that continues without an IDR.
//! The picture is generated locally as neutral I16x16 macroblocks, using the
//! live SPS and a separate CAVLC PPS. No recorded picture or proprietary
//! keyframe is stored. Unsupported syntax is passed through unchanged.

const PREFIX: &[u8] = &[0, 0, 0, 1];
const MAX_PARAMETER_BYTES: usize = 1024;
const MAX_MACROBLOCKS: u32 = 8192;

#[derive(Default)]
pub(crate) struct Bootstrap {
    sps: Option<Vec<u8>>,
    pps: Option<Vec<u8>>,
    pps_ids: [u64; 4],
    finished: bool,
}

pub(crate) struct Primer {
    pub bytes: Vec<u8>,
    pub coded_width: u32,
    pub coded_height: u32,
}

impl Bootstrap {
    /// Called on complete, normalized Annex B NALs, before forwarding the real
    /// NAL. At most one result is returned for each connection/gap.
    pub fn before(&mut self, nal: &[u8]) -> Option<Result<Primer, &'static str>> {
        if self.finished || !nal.starts_with(PREFIX) {
            return None;
        }
        match nal.get(4)? & 31 {
            7 => {
                self.sps = (nal.len() <= MAX_PARAMETER_BYTES).then(|| nal.to_vec());
            }
            8 => {
                self.pps = (nal.len() <= MAX_PARAMETER_BYTES).then(|| nal.to_vec());
                if let Some((id, _)) = parameter_ids(nal) {
                    self.pps_ids[id as usize / 64] |= 1 << (id % 64);
                }
            }
            5 => self.finished = true, // The source supplies its own IDR.
            1 => {
                let payload = nal.get(5..)?;
                let mut slice = Bits::new(&payload[..payload.len().min(64)]).ok()?;
                let (first_mb, kind, pps_id) =
                    (slice.ue().ok()?, slice.ue().ok()?, slice.ue().ok()?);
                // Start only at a whole P picture, not a later slice/partition.
                if first_mb != 0 || !matches!(kind, 0 | 5) {
                    return None;
                }
                let sps = self.sps.as_ref()?;
                let pps = self.pps.as_ref()?;
                let (saved_id, sps_id) = parameter_ids(pps)?;
                if pps_id != saved_id {
                    return None;
                }
                self.finished = true;
                let result = parse_sps(sps).and_then(|format| {
                    if format.id != sps_id {
                        return Err("SPS/PPS IDs do not match");
                    }
                    let temporary = (0..256_u32)
                        .find(|id| self.pps_ids[*id as usize / 64] & (1 << (*id % 64)) == 0)
                        .ok_or("no unused PPS ID")?;
                    Ok(make_primer(sps, pps, &format, temporary))
                });
                return Some(result);
            }
            _ => {}
        }
        None
    }
}

fn parameter_ids(nal: &[u8]) -> Option<(u32, u32)> {
    if nal.len() < 7 || nal.len() > MAX_PARAMETER_BYTES || nal.get(4)? & 31 != 8 {
        return None;
    }
    let mut bits = Bits::new(nal.get(5..)?).ok()?;
    let (id, sps_id) = (bits.ue().ok()?, bits.ue().ok()?);
    (id < 256 && sps_id < 32).then_some((id, sps_id))
}

struct Format {
    id: u32,
    frame_bits: u32,
    width_mbs: u32,
    height_mbs: u32,
}

fn parse_sps(nal: &[u8]) -> Result<Format, &'static str> {
    let mut bits = Bits::new(nal.get(5..).ok_or("truncated SPS")?)?;
    if bits.read(8)? != 100 {
        return Err("bootstrap supports H.264 High profile");
    }
    bits.read(8)?; // constraint flags
    bits.read(8)?; // level; HDMI applies its existing Pi level limit separately
    let id = bits.ue()?;
    if id > 31 || bits.ue()? != 1 || bits.ue()? != 0 || bits.ue()? != 0 {
        return Err("bootstrap requires 8-bit 4:2:0");
    }
    if bits.read(1)? != 0 || bits.read(1)? != 0 {
        return Err("bootstrap does not support SPS scaling matrices or transform bypass");
    }
    let frame_bits = bits.ue()?.checked_add(4).ok_or("invalid frame_num size")?;
    if frame_bits > 16 || bits.ue()? != 2 || bits.ue()? != 1 {
        return Err("bootstrap requires POC type 2 and one reference picture");
    }
    bits.read(1)?; // gaps_in_frame_num_allowed_flag
    let width_mbs = bits.ue()?.checked_add(1).ok_or("invalid width")?;
    let height_mbs = bits.ue()?.checked_add(1).ok_or("invalid height")?;
    if bits.read(1)? != 1 {
        return Err("bootstrap requires progressive pictures");
    }
    bits.read(1)?; // direct_8x8_inference_flag
    if bits.read(1)? != 0 {
        for _ in 0..4 {
            bits.ue()?; // the live SPS retains its own display crop
        }
    }
    bits.read(1)?; // vui_parameters_present_flag; VUI does not alter slice syntax
    let mbs = width_mbs
        .checked_mul(height_mbs)
        .ok_or("invalid dimensions")?;
    if width_mbs > 120 || height_mbs > 120 || mbs > MAX_MACROBLOCKS {
        return Err("bootstrap dimensions exceed the Pi 4 H.264 limit");
    }
    Ok(Format {
        id,
        frame_bits,
        width_mbs,
        height_mbs,
    })
}

fn make_primer(sps: &[u8], pps: &[u8], format: &Format, temporary: u32) -> Primer {
    let mut p = Writer::new();
    p.ue(temporary);
    p.ue(format.id);
    p.write(0, 2); // CAVLC, no bottom-field POC
    p.ue(0); // no slice groups
    p.ue(0); // default L0 references minus 1
    p.ue(0); // default L1 references minus 1
    p.write(0, 3); // no weighted prediction
    p.ue(0); // pic_init_qp_minus26, se(0)
    p.ue(0); // pic_init_qs_minus26, se(0)
    p.ue(0); // chroma_qp_index_offset, se(0)
    p.write(1, 1); // deblocking_filter_control_present_flag
    p.write(0, 2); // no constrained intra / redundant picture count
    let temporary_pps = p.finish();

    let mut picture = Writer::new();
    picture.ue(0); // first_mb_in_slice
    picture.ue(2); // I slice
    picture.ue(temporary);
    picture.write(0, format.frame_bits);
    picture.ue(0); // idr_pic_id
    picture.write(0, 2); // no_output_of_prior_pics, long_term_reference
    picture.ue(0); // slice_qp_delta, se(0)
    picture.ue(1); // disable deblocking
    for _ in 0..format.width_mbs * format.height_mbs {
        // I_16x16, DC prediction, no AC/chroma residual. With no neighbors at
        // the top-left, DC starts at 128 and propagates through the picture.
        // This is one byte per MB, fitting the V4L2 compressed-input buffer.
        picture.ue(3);
        picture.ue(0); // intra_chroma_pred_mode = DC
        picture.ue(0); // mb_qp_delta, se(0)
        picture.write(1, 1); // CAVLC coeff_token: TotalCoeff=0, TrailingOnes=0
    }
    let picture = picture.finish();
    let mut bytes = Vec::with_capacity(picture.len() + 256);
    bytes.extend_from_slice(&[0, 0, 0, 1, 9, 0xf0]);
    bytes.extend_from_slice(sps);
    bytes.extend_from_slice(pps); // retain the original live PPS
    append_nal(&mut bytes, 0x68, &temporary_pps);
    append_nal(&mut bytes, 0x65, &picture);
    bytes.extend_from_slice(&[0, 0, 0, 1, 9, 0xf0]); // finish the generated AU
    Primer {
        bytes,
        coded_width: format.width_mbs * 16,
        coded_height: format.height_mbs * 16,
    }
}

fn append_nal(output: &mut Vec<u8>, header: u8, rbsp: &[u8]) {
    output.extend_from_slice(PREFIX);
    output.push(header);
    let mut zeros = 0;
    for &byte in rbsp {
        if zeros == 2 && byte <= 3 {
            output.push(3);
            zeros = 0;
        }
        output.push(byte);
        zeros = if byte == 0 { zeros + 1 } else { 0 };
    }
}

struct Writer {
    bytes: Vec<u8>,
    partial: u8,
    count: u32,
}

impl Writer {
    fn new() -> Self {
        Self {
            bytes: Vec::new(),
            partial: 0,
            count: 0,
        }
    }
    fn write(&mut self, value: u32, count: u32) {
        for bit in (0..count).rev() {
            self.partial = (self.partial << 1) | ((value >> bit) & 1) as u8;
            self.count += 1;
            if self.count == 8 {
                self.bytes.push(self.partial);
                self.partial = 0;
                self.count = 0;
            }
        }
    }
    fn ue(&mut self, value: u32) {
        let value = value + 1;
        let count = 32 - value.leading_zeros();
        self.write(0, count - 1);
        self.write(value, count);
    }
    fn align(&mut self) {
        if self.count != 0 {
            self.write(0, 8 - self.count);
        }
    }
    fn finish(mut self) -> Vec<u8> {
        self.write(1, 1);
        self.align();
        self.bytes
    }
}

struct Bits {
    bytes: Vec<u8>,
    position: usize,
}

impl Bits {
    fn new(ebsp: &[u8]) -> Result<Self, &'static str> {
        let mut bytes = Vec::with_capacity(ebsp.len());
        let mut zeros = 0;
        for (index, &byte) in ebsp.iter().enumerate() {
            if zeros == 2 && byte == 3 {
                if ebsp.get(index + 1).is_none_or(|next| *next > 3) {
                    return Err("invalid emulation prevention");
                }
                zeros = 0;
                continue;
            }
            bytes.push(byte);
            zeros = if byte == 0 { (zeros + 1).min(2) } else { 0 };
        }
        Ok(Self { bytes, position: 0 })
    }
    fn read(&mut self, count: u32) -> Result<u32, &'static str> {
        if count > 32 || self.position + count as usize > self.bytes.len() * 8 {
            return Err("truncated header");
        }
        let mut value = 0;
        for _ in 0..count {
            value = (value << 1)
                | u32::from((self.bytes[self.position / 8] >> (7 - self.position % 8)) & 1);
            self.position += 1;
        }
        Ok(value)
    }
    fn ue(&mut self) -> Result<u32, &'static str> {
        let mut zeros = 0;
        while self.read(1)? == 0 {
            zeros += 1;
            if zeros > 31 {
                return Err("invalid Exp-Golomb value");
            }
        }
        Ok((1_u32 << zeros) - 1 + self.read(zeros)?)
    }
}

#[cfg(test)]
#[path = "bootstrap.test.rs"]
mod tests;
