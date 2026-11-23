//! Recover completed MP4 fragments without requiring an external analysis tool.
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

/// Require a fragmented video header and a complete moof/mdat pair; ignore a torn tail.
/// This validates the recoverable container, not H.264 decoding (covered by ffprobe fixtures).
pub fn recoverable(path: &Path) -> bool {
    recovery_length(path).is_some()
}

/// Remove only the torn tail after the last complete fragment; preserve all usable frames.
pub fn recover(path: &Path) -> bool {
    let Some(length) = recovery_length(path) else {
        return false;
    };
    let Ok(file) = std::fs::OpenOptions::new().write(true).open(path) else {
        return false;
    };
    file.set_len(length).is_ok()
}
fn recovery_length(path: &Path) -> Option<u64> {
    let mut file = File::open(path).ok()?;
    let length = file.metadata().ok()?.len();
    let mut offset = 0_u64;
    let mut ftyp = false;
    let mut defaults = None;
    let mut pending: Option<Vec<(u64, u64)>> = None;
    let mut last_complete = 0;
    while offset.saturating_add(8) <= length {
        if file.seek(SeekFrom::Start(offset)).is_err() {
            break;
        }
        let mut header = [0_u8; 8];
        if file.read_exact(&mut header).is_err() {
            break;
        }
        let short = u32::from_be_bytes(header[..4].try_into().unwrap()) as u64;
        let (size, header_size) = if short == 1 {
            let mut wide = [0_u8; 8];
            if file.read_exact(&mut wide).is_err() {
                break;
            }
            (u64::from_be_bytes(wide), 16_u64)
        } else if short == 0 {
            // A zero-sized top-level box extends to EOF. Sample extents below
            // still have to fit the actual remaining media payload.
            (length - offset, 8)
        } else {
            (short, 8)
        };
        if size < header_size || offset.checked_add(size).map_or(true, |end| end > length) {
            break;
        }
        match &header[4..8] {
            b"ftyp" => {
                if ftyp || size < header_size + 8 {
                    break;
                }
                ftyp = true;
            }
            b"moov" | b"moof" => {
                // A malformed/unsupported tail never invalidates earlier completed fragments.
                if size > 16 * 1024 * 1024 {
                    break;
                }
                let mut body = vec![0; (size - header_size) as usize];
                if file.read_exact(&mut body).is_err() {
                    break;
                }
                if &header[4..8] == b"moov" {
                    if defaults.is_some() || !ftyp {
                        break;
                    }
                    let Some(movie) = movie_defaults(&body) else {
                        break;
                    };
                    defaults = Some(movie);
                } else {
                    if pending.is_some() {
                        break;
                    }
                    let Some(movie) = defaults.as_ref() else {
                        break;
                    };
                    let Some(ranges) = fragment_ranges(&body, offset, movie) else {
                        break;
                    };
                    pending = Some(ranges);
                }
            }
            b"mdat" => {
                if let Some(samples) = pending.as_mut() {
                    // Keep only unresolved runs; allocation stays bounded by moof metadata.
                    samples.retain(|(start, end)| {
                        *start < offset + header_size || *end > offset + size
                    });
                    if samples.is_empty() {
                        last_complete = offset + size;
                        pending = None;
                    }
                }
            }
            _ => {}
        }
        offset += size;
    }
    (ftyp && defaults.is_some() && last_complete > 0).then_some(
        if offset == length && pending.is_none() {
            length
        } else {
            last_complete
        },
    )
}

fn number(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(
        bytes.get(at..at.checked_add(4)?)?.try_into().ok()?,
    ))
}
fn child_boxes(mut bytes: &[u8]) -> Option<Vec<(&[u8], &[u8])>> {
    let mut children = Vec::new();
    while !bytes.is_empty() {
        let short = number(bytes, 0)?;
        let kind = bytes.get(4..8)?;
        let (size, header) = match short {
            0 => (bytes.len(), 8),
            1 => (
                usize::try_from(u64::from_be_bytes(bytes.get(8..16)?.try_into().ok()?)).ok()?,
                16,
            ),
            _ => (short as usize, 8),
        };
        if size < header || size > bytes.len() {
            return None;
        }
        children.push((kind, &bytes[header..size]));
        bytes = &bytes[size..];
    }
    Some(children)
}

// Only defaults relevant to sample extents are needed; codec validation stays with the decoder.
fn movie_defaults(bytes: &[u8]) -> Option<std::collections::BTreeMap<u32, u32>> {
    if !has_video_handler(bytes) {
        return None;
    }
    let children = child_boxes(bytes)?;
    let mut mvex = children.iter().filter(|(kind, _)| *kind == b"mvex");
    let (_, body) = mvex.next()?;
    if mvex.next().is_some() {
        return None;
    }
    let mut defaults = std::collections::BTreeMap::new();
    for (kind, body) in child_boxes(body)? {
        if kind == b"trex" {
            if body.len() != 24 || number(body, 0)? != 0 {
                return None;
            }
            let track = number(body, 4)?;
            if track == 0 || defaults.insert(track, number(body, 16)?).is_some() {
                return None;
            }
        }
    }
    Some(defaults)
}

/// Capture writes one video traf. Reject unsupported layouts rather than guess
/// cross-track implicit bases. Every trun must resolve known positive sample sizes.
fn fragment_ranges(
    bytes: &[u8],
    moof_start: u64,
    defaults: &std::collections::BTreeMap<u32, u32>,
) -> Option<Vec<(u64, u64)>> {
    let children = child_boxes(bytes)?;
    let mut trafs = children.iter().filter(|(kind, _)| *kind == b"traf");
    let (_, traf) = trafs.next()?;
    if trafs.next().is_some() {
        return None;
    }
    let boxes = child_boxes(traf)?;
    let mut headers = boxes.iter().filter(|(kind, _)| *kind == b"tfhd");
    let (_, header) = headers.next()?;
    if headers.next().is_some() {
        return None;
    }
    let flags = number(header, 0)?;
    if flags & !0x02003b != 0 {
        return None;
    }
    let track = number(header, 4)?;
    if track == 0 || !defaults.contains_key(&track) {
        return None;
    }
    let mut at = 8;
    let base = if flags & 1 != 0 {
        let base = u64::from_be_bytes(header.get(at..at + 8)?.try_into().ok()?);
        at += 8;
        base
    } else {
        moof_start
    };
    if flags & 2 != 0 {
        at += 4;
    }
    if flags & 8 != 0 {
        at += 4;
    }
    let size = if flags & 0x10 != 0 {
        let size = number(header, at)?;
        at += 4;
        size
    } else {
        defaults.get(&track).copied().unwrap_or(0)
    };
    if flags & 0x20 != 0 {
        at += 4;
    }
    if at != header.len() {
        return None;
    }
    let mut cursor = base;
    let mut ranges = Vec::new();
    for (_, run) in boxes.iter().filter(|(kind, _)| *kind == b"trun") {
        let full = number(run, 0)?;
        let flags = full & 0x00ff_ffff;
        if full >> 24 > 1 || flags & !0xf05 != 0 || flags & 0x404 == 0x404 {
            return None;
        }
        let count = number(run, 4)?;
        if count == 0 {
            return None;
        }
        let mut at = 8;
        if flags & 1 != 0 {
            cursor = base.checked_add_signed(i64::from(number(run, at)? as i32))?;
            at += 4;
        }
        if flags & 4 != 0 {
            at += 4;
        }
        let stride = [0x100, 0x200, 0x400, 0x800]
            .iter()
            .filter(|bit| flags & **bit != 0)
            .count()
            * 4;
        if at.checked_add((count as usize).checked_mul(stride)?)? != run.len() {
            return None;
        }
        let total = if flags & 0x200 == 0 {
            if size == 0 {
                return None;
            }
            u64::from(count).checked_mul(u64::from(size))?
        } else {
            let mut total = 0_u64;
            for _ in 0..count {
                if flags & 0x100 != 0 {
                    at += 4;
                }
                let size = number(run, at)?;
                if size == 0 {
                    return None;
                }
                total = total.checked_add(u64::from(size))?;
                at += 4;
                if flags & 0x400 != 0 {
                    at += 4;
                }
                if flags & 0x800 != 0 {
                    at += 4;
                }
            }
            total
        };
        let end = cursor.checked_add(total)?;
        ranges.push((cursor, end));
        cursor = end;
    }
    (!ranges.is_empty()).then_some(ranges)
}
fn container(kind: &[u8]) -> bool {
    [b"trak", b"mdia", b"minf", b"stbl", b"mvex", b"traf"]
        .iter()
        .any(|tag| tag.as_slice() == kind)
}
fn search(bytes: &[u8], depth: usize, predicate: &impl Fn(&[u8], &[u8]) -> bool) -> bool {
    if depth > 12 {
        return false;
    }
    let Some(children) = child_boxes(bytes) else {
        return false;
    };
    children.into_iter().any(|(kind, body)| {
        predicate(kind, body) || (container(kind) && search(body, depth + 1, predicate))
    })
}
fn has_video_handler(bytes: &[u8]) -> bool {
    search(bytes, 0, &|kind, body| {
        kind == b"hdlr" && body.get(8..12) == Some(b"vide")
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn atom(tag: &[u8; 4], body: &[u8]) -> Vec<u8> {
        [
            ((body.len() + 8) as u32).to_be_bytes().as_slice(),
            tag,
            body,
        ]
        .concat()
    }
    fn initialization(default_size: u32) -> Vec<u8> {
        let hdlr = atom(b"hdlr", &[&[0_u8; 8][..], b"vide"].concat());
        let trex = [0_u32, 1, 1, 200, default_size, 0]
            .map(u32::to_be_bytes)
            .concat();
        [
            atom(b"ftyp", b"isom0000"),
            atom(
                b"moov",
                &[
                    atom(b"mvex", &atom(b"trex", &trex)),
                    atom(b"trak", &atom(b"mdia", &hdlr)),
                ]
                .concat(),
            ),
        ]
        .concat()
    }
    fn tfhd(default_size: Option<u32>, explicit_base: Option<u64>) -> Vec<u8> {
        let flags = if explicit_base.is_some() {
            1_u32
        } else {
            0x020000
        } | if default_size.is_some() { 0x10 } else { 0 };
        let mut bytes = [flags.to_be_bytes(), 1_u32.to_be_bytes()].concat();
        if let Some(base) = explicit_base {
            bytes.extend(base.to_be_bytes());
        }
        if let Some(size) = default_size {
            bytes.extend(size.to_be_bytes());
        }
        atom(b"tfhd", &bytes)
    }
    fn trun(count: u32, offset: Option<i32>, sizes: Option<&[u32]>) -> Vec<u8> {
        let flags =
            if offset.is_some() { 1_u32 } else { 0 } | if sizes.is_some() { 0x200 } else { 0 };
        let mut bytes = [flags.to_be_bytes(), count.to_be_bytes()].concat();
        if let Some(offset) = offset {
            bytes.extend(offset.to_be_bytes());
        }
        if let Some(sizes) = sizes {
            for size in sizes {
                bytes.extend(size.to_be_bytes());
            }
        }
        atom(b"trun", &bytes)
    }
    fn moof(header: &[u8], runs: &[Vec<u8>]) -> Vec<u8> {
        atom(
            b"moof",
            &atom(b"traf", &[header.to_vec(), runs.concat()].concat()),
        )
    }
    fn relative_pair(
        default: Option<u32>,
        count: u32,
        sizes: Option<&[u32]>,
        payload: &[u8],
    ) -> Vec<u8> {
        let header = tfhd(default, None);
        let empty_offset = moof(&header, &[trun(count, Some(0), sizes)]);
        let fragment = moof(
            &header,
            &[trun(count, Some((empty_offset.len() + 8) as i32), sizes)],
        );
        [fragment, atom(b"mdat", payload)].concat()
    }
    struct TempMp4(std::path::PathBuf);
    impl TempMp4 {
        fn new(bytes: &[u8]) -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "clovy-mp4-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::write(&path, bytes).unwrap();
            Self(path)
        }
    }
    impl Drop for TempMp4 {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[test]
    fn eof_immediately_after_moof_trims_the_orphan_fragment() {
        let mut bytes = [
            initialization(0),
            relative_pair(None, 1, Some(&[6]), b"sample"),
        ]
        .concat();
        let complete = bytes.len() as u64;
        bytes.extend(moof(&tfhd(Some(6), None), &[trun(1, Some(64), None)]));
        let file = TempMp4::new(&bytes);
        assert!(recover(&file.0));
        assert_eq!(std::fs::metadata(&file.0).unwrap().len(), complete);
    }
    #[test]
    fn oversized_complete_tail_preserves_prior_fragments() {
        let mut bytes = [
            initialization(0),
            relative_pair(None, 1, Some(&[6]), b"sample"),
        ]
        .concat();
        let complete = bytes.len() as u64;
        bytes.extend(atom(b"moof", &vec![0; 16 * 1024 * 1024]));
        let file = TempMp4::new(&bytes);
        assert!(recover(&file.0));
        assert_eq!(std::fs::metadata(&file.0).unwrap().len(), complete);
    }
    #[test]
    fn trun_sizes_cannot_claim_missing_media_bytes() {
        let file = TempMp4::new(
            &[
                initialization(0),
                relative_pair(None, 1, Some(&[100]), b"x"),
            ]
            .concat(),
        );
        assert!(!recoverable(&file.0));
    }
    #[test]
    fn invalid_fragment_tail_preserves_the_prior_complete_prefix() {
        let mut bytes = [
            initialization(0),
            relative_pair(None, 1, Some(&[6]), b"sample"),
        ]
        .concat();
        let complete = bytes.len() as u64;
        bytes.extend(relative_pair(None, 1, Some(&[100]), b"x"));
        let file = TempMp4::new(&bytes);
        assert!(recover(&file.0));
        assert_eq!(std::fs::metadata(&file.0).unwrap().len(), complete);
    }
    #[test]
    fn tfhd_and_trex_default_sizes_are_checked() {
        for (trex, tfhd_size) in [(0, Some(3)), (100, Some(3)), (3, None)] {
            let valid = TempMp4::new(
                &[
                    initialization(trex),
                    relative_pair(tfhd_size, 2, None, b"123456"),
                ]
                .concat(),
            );
            assert!(recoverable(&valid.0));
            let short = TempMp4::new(
                &[
                    initialization(trex),
                    relative_pair(tfhd_size, 2, None, b"12345"),
                ]
                .concat(),
            );
            assert!(!recoverable(&short.0));
        }
    }
    #[test]
    fn explicit_sizes_override_defaults_and_zero_default_is_not_inherited() {
        let explicit = TempMp4::new(
            &[
                initialization(99),
                relative_pair(Some(100), 1, Some(&[6]), b"sample"),
            ]
            .concat(),
        );
        assert!(recoverable(&explicit.0));
        let zero_override = TempMp4::new(
            &[
                initialization(3),
                relative_pair(Some(0), 2, None, b"123456"),
            ]
            .concat(),
        );
        assert!(!recoverable(&zero_override.0));
        for sizes in [None, Some([1_u32].as_slice())] {
            let huge_count = TempMp4::new(
                &[
                    initialization(0),
                    relative_pair(Some(u32::MAX), u32::MAX, sizes, b"sample"),
                ]
                .concat(),
            );
            assert!(!recoverable(&huge_count.0));
        }
    }

    #[test]
    fn consecutive_truns_continue_at_previous_run_end() {
        let header = tfhd(Some(3), None);
        let template = moof(&header, &[trun(1, Some(0), None), trun(1, None, None)]);
        let fragment = moof(
            &header,
            &[
                trun(1, Some((template.len() + 8) as i32), None),
                trun(1, None, None),
            ],
        );
        for (payload, expected) in [(b"123456".as_slice(), true), (b"12345".as_slice(), false)] {
            let file = TempMp4::new(
                &[initialization(0), fragment.clone(), atom(b"mdat", payload)].concat(),
            );
            assert_eq!(recoverable(&file.0), expected);
        }
    }
    #[test]
    fn explicit_base_and_signed_run_offsets_must_land_in_current_mdat() {
        let init = initialization(0);
        let header = tfhd(Some(6), Some(0));
        let template = moof(&header, &[trun(1, Some(0), None)]);
        let media = (init.len() + template.len() + 8) as u64;
        let valid = moof(&tfhd(Some(6), Some(media + 2)), &[trun(1, Some(-2), None)]);
        let file = TempMp4::new(&[init.clone(), valid, atom(b"mdat", b"sample")].concat());
        assert!(recoverable(&file.0));
        for fragment in [
            moof(&tfhd(Some(6), None), &[trun(1, Some(-1), None)]),
            moof(
                &tfhd(Some(6), Some(u64::MAX - 2)),
                &[trun(1, Some(8), None)],
            ),
            moof(&tfhd(Some(6), Some(u64::MAX - 2)), &[trun(1, None, None)]),
        ] {
            let file = TempMp4::new(&[init.clone(), fragment, atom(b"mdat", b"sample")].concat());
            assert!(!recoverable(&file.0));
        }
    }
    #[test]
    fn unresolved_sizes_and_unsupported_multiple_traf_are_rejected() {
        let unknown = relative_pair(None, 1, None, b"sample");
        let traf = atom(
            b"traf",
            &[tfhd(Some(3), None), trun(1, Some(0), None)].concat(),
        );
        let multiple = [
            atom(b"moof", &[traf.clone(), traf].concat()),
            atom(b"mdat", b"sample"),
        ]
        .concat();
        for fragment in [unknown, multiple] {
            let file = TempMp4::new(&[initialization(0), fragment].concat());
            assert!(!recoverable(&file.0));
        }
    }
    #[test]
    fn fragment_track_must_be_declared_in_movie_defaults() {
        let mut unknown = relative_pair(None, 1, Some(&[6]), b"sample");
        let track = unknown.windows(4).position(|kind| kind == b"tfhd").unwrap() + 8;
        unknown[track..track + 4].copy_from_slice(&2_u32.to_be_bytes());
        let first = TempMp4::new(&[initialization(0), unknown.clone()].concat());
        assert!(
            !recoverable(&first.0),
            "explicit sizes cannot authorize an undeclared track"
        );
        let mut tail = [
            initialization(0),
            relative_pair(None, 1, Some(&[6]), b"sample"),
        ]
        .concat();
        let complete = tail.len() as u64;
        tail.extend(unknown);
        let file = TempMp4::new(&tail);
        assert!(recover(&file.0));
        assert_eq!(std::fs::metadata(&file.0).unwrap().len(), complete);
    }

    fn zero_sized_media_pair(payload: &[u8], sample_size: u32) -> Vec<u8> {
        let mut pair = relative_pair(None, 1, Some(&[sample_size]), payload);
        let mdat = pair.len() - payload.len() - 8;
        assert_eq!(&pair[mdat + 4..mdat + 8], b"mdat");
        pair[mdat..mdat + 4].fill(0);
        pair
    }

    #[test]
    fn zero_sized_final_mdat_uses_eof_without_discarding_complete_samples() {
        let bytes = [initialization(0), zero_sized_media_pair(b"sample", 6)].concat();
        let file = TempMp4::new(&bytes);
        assert!(recoverable(&file.0));
        assert!(recover(&file.0));
        assert_eq!(std::fs::read(&file.0).unwrap(), bytes);
    }

    #[test]
    fn zero_sized_mdat_still_requires_all_declared_sample_bytes() {
        let tail = zero_sized_media_pair(b"sam", 6);
        let file = TempMp4::new(&[initialization(0), tail.clone()].concat());
        assert!(!recoverable(&file.0));
        assert!(!recover(&file.0));

        let complete = [
            initialization(0),
            relative_pair(None, 1, Some(&[6]), b"sample"),
        ]
        .concat();
        let file = TempMp4::new(&[complete.clone(), tail].concat());
        assert!(recover(&file.0));
        assert_eq!(std::fs::read(&file.0).unwrap(), complete);
    }

    #[test]
    fn extended_track_and_media_containers_preserve_the_video_handler() {
        fn extended_atom(tag: &[u8; 4], body: &[u8], declared_size: u64) -> Vec<u8> {
            [
                1_u32.to_be_bytes().as_slice(),
                tag,
                &declared_size.to_be_bytes(),
                body,
            ]
            .concat()
        }
        let handler = atom(b"hdlr", &[&[0_u8; 8][..], b"vide"].concat());
        let trex = [0_u32, 1, 1, 200, 0, 0].map(u32::to_be_bytes).concat();
        for (wide_track, wide_media) in [(true, false), (false, true), (true, true)] {
            let media = if wide_media {
                extended_atom(b"mdia", &handler, handler.len() as u64 + 16)
            } else {
                atom(b"mdia", &handler)
            };
            let track = if wide_track {
                extended_atom(b"trak", &media, media.len() as u64 + 16)
            } else {
                atom(b"trak", &media)
            };
            let bytes = [
                atom(b"ftyp", b"isom0000"),
                atom(
                    b"moov",
                    &[atom(b"mvex", &atom(b"trex", &trex)), track].concat(),
                ),
                relative_pair(None, 1, Some(&[6]), b"sample"),
            ]
            .concat();
            let file = TempMp4::new(&bytes);
            assert!(
                recoverable(&file.0),
                "track={wide_track}, media={wide_media}"
            );
            assert!(recover(&file.0));
            assert_eq!(std::fs::read(&file.0).unwrap(), bytes);
        }

        // An extended header cannot claim fewer than its own sixteen bytes.
        let invalid = extended_atom(b"trak", &handler, 8);
        let bytes = [
            atom(b"ftyp", b"isom0000"),
            atom(
                b"moov",
                &[atom(b"mvex", &atom(b"trex", &trex)), invalid].concat(),
            ),
            relative_pair(None, 1, Some(&[6]), b"sample"),
        ]
        .concat();
        assert!(!recoverable(&TempMp4::new(&bytes).0));
    }

    #[test]
    fn arbitrary_bytes_are_never_an_mp4() {
        let file = TempMp4::new(&[1_u8; 64]);
        assert!(!recoverable(&file.0));
    }
}
