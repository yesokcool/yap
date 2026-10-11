//! Audio redundancy (RED, RFC 2198) on the voice (PROTOCOL.md › Voice ›
//! Redundancy): each packet also carries the 20 ms frame before it, so a lost
//! packet is rebuilt from the next one, word for word. webrtc-rs only carries
//! the packets; packing and unpacking them is ours.
//!
//! A RED payload is a list of block headers, then the blocks' bytes in the
//! same order: every block but the last has a 4-byte header (F=1, its payload
//! type, a 14-bit timestamp offset back from the packet's, a 10-bit length);
//! the last one (the primary, this packet's own frame) has a 1-byte header
//! (F=0, its payload type) and takes the rest.

/// RFC 2198's block length field is 10 bits.
pub const MAX_BLOCK: usize = 1023;
/// …and we keep a packet's payload under this (what browsers keep RTP under).
pub const MAX_PAYLOAD: usize = 1200;
/// The timestamp offset field is 14 bits.
const MAX_OFFSET: u32 = (1 << 14) - 1;
/// More redundant blocks than this in one packet is nobody's real stream.
const MAX_BLOCKS: usize = 8;

/// The fmtp line of `red/48000/2` for one earlier Opus frame (distance 1).
pub fn fmtp(opus_pt: u8) -> String {
    format!("{opus_pt}/{opus_pt}")
}

/// Pack `primary` (this frame) with `previous` (the frame `offset` samples
/// earlier) into `out`. The copy is left out when it's empty, too long for a
/// block (> 1023 bytes, only near 510 kbps) or would make the payload too big
/// (> 1200 bytes): then the packet carries the primary block alone. Returns
/// whether the copy went in.
pub fn pack(out: &mut Vec<u8>, opus_pt: u8, primary: &[u8], previous: &[u8], offset: u32) -> bool {
    out.clear();
    let pt = opus_pt & 0x7f;
    let with = !previous.is_empty() && previous.len() <= MAX_BLOCK && offset <= MAX_OFFSET && 5 + previous.len() + primary.len() <= MAX_PAYLOAD;
    if with {
        let len = previous.len() as u32;
        out.push(0x80 | pt);
        out.push((offset >> 6) as u8);
        out.push((((offset & 0x3f) << 2) | (len >> 8)) as u8);
        out.push((len & 0xff) as u8);
    }
    out.push(pt);
    if with {
        out.extend_from_slice(previous);
    }
    out.extend_from_slice(primary);
    with
}

/// One block of a RED packet: its payload type, how far back it is (RTP
/// timestamp units; 0 for the primary) and its bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Block<'a> {
    pub pt: u8,
    pub offset: u32,
    pub data: &'a [u8],
}

/// The blocks of a RED payload: the redundant ones (oldest first, as sent),
/// then the primary. `None` for anything malformed (a header running off the
/// end, lengths longer than the packet, no final header): such a packet is
/// dropped, never half-read.
pub fn unpack(payload: &[u8]) -> Option<(Vec<Block<'_>>, Block<'_>)> {
    let mut heads: Vec<(u8, u32, usize)> = Vec::new();
    let mut i = 0;
    let primary_pt = loop {
        let b = *payload.get(i)?;
        if b & 0x80 == 0 {
            i += 1;
            break b & 0x7f;
        }
        let h = payload.get(i..i + 4)?;
        if heads.len() == MAX_BLOCKS {
            return None;
        }
        let offset = ((h[1] as u32) << 6) | ((h[2] as u32) >> 2);
        let len = (((h[2] & 0x03) as usize) << 8) | h[3] as usize;
        heads.push((b & 0x7f, offset, len));
        i += 4;
    };
    let mut blocks = Vec::with_capacity(heads.len());
    for (pt, offset, len) in heads {
        let data = payload.get(i..i.checked_add(len)?)?;
        blocks.push(Block { pt, offset, data });
        i += len;
    }
    Some((blocks, Block { pt: primary_pt, offset: 0, data: &payload[i..] }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_primary_and_the_frame_before() {
        let mut out = Vec::new();
        assert!(pack(&mut out, 111, &[1, 2, 3], &[9, 8], 960));
        // F=1|111, offset 960 (14 bits) + length 2 (10 bits), then F=0|111, then the copy, then the primary.
        assert_eq!(out, [0x80 | 111, (960 >> 6) as u8, ((960 & 0x3f) << 2) as u8, 2, 111, 9, 8, 1, 2, 3]);
        let (red, primary) = unpack(&out).unwrap();
        assert_eq!(red, [Block { pt: 111, offset: 960, data: &[9, 8] }]);
        assert_eq!(primary, Block { pt: 111, offset: 0, data: &[1, 2, 3] });
    }

    #[test]
    fn primary_alone_when_the_copy_cant_go() {
        let mut out = Vec::new();
        // No earlier frame yet.
        assert!(!pack(&mut out, 111, &[1, 2, 3], &[], 960));
        assert_eq!(out, [111, 1, 2, 3]);
        let (red, primary) = unpack(&out).unwrap();
        assert!(red.is_empty());
        assert_eq!(primary.data, &[1, 2, 3]);
        // A copy over 1023 bytes (only near 510 kbps): RFC 2198's length field can't say it.
        assert!(!pack(&mut out, 111, &[0; 10], &[7; 1024], 960));
        assert_eq!(out.len(), 11);
        // Exactly 1023 fits the field, but not with a big primary: the payload stays ≤ 1200 bytes.
        assert!(pack(&mut out, 111, &[0; 172], &[7; 1023], 960));
        assert_eq!(out.len(), 1200);
        assert!(!pack(&mut out, 111, &[0; 173], &[7; 1023], 960));
        assert_eq!(out.len(), 174);
        // An offset the 14-bit field can't hold.
        assert!(!pack(&mut out, 111, &[1], &[2], 1 << 14));
    }

    #[test]
    fn roundtrips_lengths_and_offsets_at_the_edges() {
        let mut out = Vec::new();
        for (len, offset) in [(1usize, 1u32), (255, 960), (256, 960), (700, 16383), (1023, 0)] {
            let prev = vec![0xab; len];
            assert!(pack(&mut out, 63, &[5], &prev, offset));
            let (red, primary) = unpack(&out).unwrap();
            assert_eq!(red.len(), 1);
            assert_eq!((red[0].pt, red[0].offset, red[0].data.len()), (63, offset, len));
            assert_eq!(primary.data, &[5]);
        }
    }

    /// What browsers send: Opus 111 in RED 63, one redundant block, and RED
    /// packets with two (distance 2) or none.
    #[test]
    fn reads_several_blocks() {
        let mut p = vec![0x80 | 111, (1920 >> 6) as u8, ((1920 & 0x3f) << 2) as u8, 1];
        p.extend([0x80 | 111, (960 >> 6) as u8, ((960 & 0x3f) << 2) as u8, 2]);
        p.push(111);
        p.extend([0xaa, 0xbb, 0xbb, 0xcc, 0xcc, 0xcc]);
        let (red, primary) = unpack(&p).unwrap();
        assert_eq!(red.len(), 2);
        assert_eq!((red[0].offset, red[0].data), (1920, &[0xaa][..]));
        assert_eq!((red[1].offset, red[1].data), (960, &[0xbb, 0xbb][..]));
        assert_eq!(primary.data, &[0xcc, 0xcc, 0xcc]);
        // A primary with no bytes is fine (nothing to play).
        assert_eq!(unpack(&[111]).unwrap().1.data.len(), 0);
    }

    /// Malformed packets never crash or panic: they're refused whole.
    #[test]
    fn malformed_packets_are_refused() {
        assert_eq!(unpack(&[]), None, "empty");
        assert_eq!(unpack(&[0x80 | 111]), None, "header cut short");
        assert_eq!(unpack(&[0x80 | 111, 15, 0]), None, "header cut short");
        assert_eq!(unpack(&[0x80 | 111, 15, 0, 5, 111, 1, 2]), None, "block longer than the packet");
        assert_eq!(unpack(&[0x80 | 111, 15, 0, 1, 0x80 | 111, 15, 0, 1]), None, "no final header");
        let many: Vec<u8> = (0..9).flat_map(|_| [0x80 | 111, 15, 0, 0]).chain([111]).collect();
        assert_eq!(unpack(&many), None, "too many blocks");
        // Every byte string up to 6 bytes, and noise: never a panic.
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        for n in 0..4000 {
            let len = n % 40;
            let bytes: Vec<u8> = (0..len)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    seed as u8
                })
                .collect();
            if let Some((red, primary)) = unpack(&bytes) {
                let total: usize = red.iter().map(|b| b.data.len()).sum::<usize>() + primary.data.len();
                assert!(total < bytes.len().max(1));
            }
        }
        for a in 0..=255u8 {
            for b in [0u8, 1, 0x7f, 0xff] {
                let _ = unpack(&[a, b, b, a]);
                let _ = unpack(&[a, b]);
            }
        }
    }
}
