use super::*;

/// The ungated URL for this itag, or `None` when the track should be dropped.
///
/// The size check guards the byte ranges: they come from rustypipe and are only
/// valid against the exact same transcode, so a length mismatch means the two
/// clients are not describing the same file and the ranges would address the
/// wrong bytes.
///
/// The byte ranges a DASH `SegmentBase` needs.
///
/// yt-dlp reports everything about a format except these, so they are derived
/// from the container itself rather than taken from a second extractor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SegmentRanges {
    pub(crate) init_end: u64,
    pub(crate) index_start: u64,
    pub(crate) index_end: u64,
}

/// Walk ISO-BMFF boxes looking for `sidx`.
///
/// YouTube's DASH-ready MP4 is laid out `ftyp`, `moov`, `sidx`, then fragments,
/// so the initialisation segment is everything before `sidx` and the index is
/// the `sidx` box itself. Verified against the extractor's own numbers.
pub(crate) fn mp4_segment_ranges(head: &[u8]) -> Option<SegmentRanges> {
    let mut offset = 0usize;
    while offset + 8 <= head.len() {
        let declared = u32::from_be_bytes(head[offset..offset + 4].try_into().ok()?);
        let kind = &head[offset + 4..offset + 8];
        // A declared size of 1 means the real size is a 64-bit value following
        // the header. 0 means "to end of file", which cannot precede an index.
        let (size, header) = if declared == 1 {
            if offset + 16 > head.len() {
                return None;
            }
            (
                u64::from_be_bytes(head[offset + 8..offset + 16].try_into().ok()?),
                16u64,
            )
        } else {
            (u64::from(declared), 8u64)
        };
        if kind == b"sidx" {
            let start = u64::try_from(offset).ok()?;
            return Some(SegmentRanges {
                init_end: start.checked_sub(1)?,
                index_start: start,
                index_end: start.checked_add(size)?.checked_sub(1)?,
            });
        }
        if size < header {
            return None;
        }
        offset = offset.checked_add(usize::try_from(size).ok()?)?;
    }
    None
}

/// Read an EBML variable-length integer, returning its value and width.
///
/// The leading zero count of the first byte gives the width. The marker bit is
/// part of an element ID but not of a size, hence `keep_marker`.
pub(crate) fn ebml_vint(bytes: &[u8], offset: usize, keep_marker: bool) -> Option<(u64, usize)> {
    let first = *bytes.get(offset)?;
    if first == 0 {
        return None;
    }
    let width = first.leading_zeros() as usize + 1;
    if width > 8 || offset + width > bytes.len() {
        return None;
    }
    let mut value = if keep_marker {
        u64::from(first)
    } else if width == 8 {
        // Every value bit lives in the following bytes; the first is only the
        // marker, and shifting a u8 by its full width is an overflow.
        0
    } else {
        u64::from(first & (0xFFu8 >> width))
    };
    for index in 1..width {
        value = (value << 8) | u64::from(bytes[offset + index]);
    }
    Some((value, width))
}

pub(crate) const EBML_SEGMENT_ID: u64 = 0x1853_8067;

pub(crate) const EBML_CUES_ID: u64 = 0x1C53_BB6B;

/// Find the `Cues` element that indexes a WebM stream.
///
/// The initialisation segment is everything before `Cues` and the index is the
/// element itself, header included — the same shape as MP4's `sidx`, which is
/// why both share one return type.
pub(crate) fn webm_segment_ranges(head: &[u8]) -> Option<SegmentRanges> {
    fn scan(head: &[u8], start: usize, end: usize, descended: bool) -> Option<SegmentRanges> {
        let mut offset = start;
        while offset < end {
            let (id, id_width) = ebml_vint(head, offset, true)?;
            let (size, size_width) = ebml_vint(head, offset + id_width, false)?;
            let data = offset.checked_add(id_width)?.checked_add(size_width)?;
            if id == EBML_CUES_ID {
                let start = u64::try_from(offset).ok()?;
                let total = u64::try_from(id_width + size_width)
                    .ok()?
                    .checked_add(size)?;
                return Some(SegmentRanges {
                    init_end: start.checked_sub(1)?,
                    index_start: start,
                    index_end: start.checked_add(total)?.checked_sub(1)?,
                });
            }
            // Cues live inside the Segment master element, so that one is
            // stepped into rather than over.
            if id == EBML_SEGMENT_ID && !descended {
                let limit = usize::try_from(size)
                    .ok()
                    .and_then(|size| data.checked_add(size))
                    .unwrap_or(end)
                    .min(end);
                return scan(head, data, limit, true);
            }
            offset = data.checked_add(usize::try_from(size).ok()?)?;
        }
        None
    }
    scan(head, 0, head.len(), false)
}

/// How many segment-range probes may be in flight at once. See
/// `ytdlp_playback_source`: past about four, connection attempts start being
/// dropped and the resolve waits whole seconds on SYN retries.
pub(crate) const SEGMENT_PROBE_CONCURRENCY: usize = 4;

/// The exact file whose container layout was probed.
///
/// The URLs expire but the container layout does not, so a probe is paid for
/// once per file rather than once per playback. The itag alone does not name
/// a file. Dubbed uploads ship one itag 251 (and one 140, 249, 250...) per
/// audio language, and the original's header is a different size from the
/// dubs'. Keyed by itag, every language got the ranges of whichever one was
/// probed first. Seen 2026-09-25 (XKSjCOKDtpk): English Opus's index range
/// pointed at cluster bytes, and Shaka failed the whole source with 3007
/// (WEBM_CUES_ELEMENT_MISSING). The transport then fell through to the gated
/// extractor source, which froze at about 1:01. Size alone is not enough
/// either: that video's Japanese and Turkish itag 140 are the same length.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct SegmentRangeKey {
    pub(crate) video_id: String,
    pub(crate) itag: u32,
    pub(crate) language: Option<String>,
    /// yt-dlp's exact `filesize`, never the estimate.
    pub(crate) size: u64,
}

/// Byte ranges by exact file. They are a fact about the file, so they never
/// go stale; the bound is on memory, not freshness.
pub(crate) static SEGMENT_RANGES: ServerCache<SegmentRangeKey, SegmentRanges> =
    ServerCache::new(Duration::from_secs(24 * 60 * 60), 20_000);

pub(crate) async fn cached_segment_ranges(key: &SegmentRangeKey) -> Option<SegmentRanges> {
    SEGMENT_RANGES.get(key).await
}

pub(crate) async fn store_segment_ranges(key: SegmentRangeKey, ranges: SegmentRanges) {
    SEGMENT_RANGES.insert(key, ranges).await;
}

/// Read enough of a stream to find its index.
///
/// 32 KiB covers `ftyp` + `moov` + `sidx` with room to spare on every format
/// measured; a container whose index sits beyond that is skipped rather than
/// chased, since a second round trip per track would cost more than the format
/// is worth.
pub(crate) async fn probe_segment_ranges(
    client: &reqwest::Client,
    key: Option<SegmentRangeKey>,
    url: &str,
) -> Option<SegmentRanges> {
    if let Some(key) = key.as_ref()
        && let Some(cached) = cached_segment_ranges(key).await
    {
        return Some(cached);
    }
    // One format that stalls is dropped from the manifest rather than holding
    // up the first frame for the client's whole timeout.
    let head = tokio::time::timeout(Duration::from_secs(4), async {
        let response = client
            .get(url)
            .header(reqwest::header::RANGE, "bytes=0-32767")
            .send()
            .await
            .ok()?;
        if !response.status().is_success() {
            return None;
        }
        response.bytes().await.ok()
    })
    .await
    .ok()??;
    let ranges = segment_ranges(&head)?;
    if let Some(key) = key {
        store_segment_ranges(key, ranges).await;
    }
    Some(ranges)
}

/// Byte ranges another extractor already reported, by `(itag, exact size)`.
///
/// Probing every format's head is most of a second of a cold resolve, and
/// rustypipe has already described the same files by the time it runs. An
/// itag and an exact byte count name one transcode, so its ranges hold for
/// yt-dlp's URL to it. Only a pair that appears once is kept: dubbed audio
/// repeats an itag per language, and two languages can share a size, so an
/// ambiguous pair is left to the probe. Ranges that are not the contiguous
/// `init` then `index` layout both containers use are left to it too.
pub(crate) fn unambiguous_segment_ranges(
    streams: impl IntoIterator<Item = (u32, u64, PlaybackByteRange, PlaybackByteRange)>,
) -> HashMap<(u32, u64), SegmentRanges> {
    let mut seen: HashMap<(u32, u64), Option<SegmentRanges>> = HashMap::new();
    for (itag, size, init, index) in streams {
        let ranges = (init.start == 0 && init.end.checked_add(1) == Some(index.start)).then_some(
            SegmentRanges {
                init_end: init.end,
                index_start: index.start,
                index_end: index.end,
            },
        );
        seen.entry((itag, size))
            .and_modify(|entry| *entry = None)
            .or_insert(ranges);
    }
    seen.into_iter()
        .filter_map(|(key, ranges)| Some((key, ranges?)))
        .collect()
}

/// [`unambiguous_segment_ranges`] for what a rustypipe player lists.
pub(crate) fn rustypipe_segment_ranges(
    player: &rustypipe::model::VideoPlayer,
) -> HashMap<(u32, u64), SegmentRanges> {
    let range = |range: &std::ops::Range<u32>| PlaybackByteRange {
        start: u64::from(range.start),
        end: u64::from(range.end),
    };
    let video = player.video_only_streams.iter().filter_map(|stream| {
        Some((
            stream.itag,
            stream.size?,
            range(stream.init_range.as_ref()?),
            range(stream.index_range.as_ref()?),
        ))
    });

    let audio = player.audio_streams.iter().filter_map(|stream| {
        Some((
            stream.itag,
            stream.size,
            range(stream.init_range.as_ref()?),
            range(stream.index_range.as_ref()?),
        ))
    });
    unambiguous_segment_ranges(video.chain(audio))
}

/// Pick a parser from the container's magic rather than its declared extension.
pub(crate) fn segment_ranges(head: &[u8]) -> Option<SegmentRanges> {
    if head.len() >= 8 && &head[4..8] == b"ftyp" {
        mp4_segment_ranges(head)
    } else if head.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        webm_segment_ranges(head)
    } else {
        None
    }
}
