use super::header::{BLOCK_HEADER_SIZE, SEGMENT_HEADER_SIZE};
use super::reader::{SegmentError, SegmentReader};
use super::recovery::recover_segment;
use super::writer::SegmentWriter;
use crate::event::model::{Event, Outcome, BIN_SIZE};
use std::fs::{self, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;

static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

fn temp_segment_path(prefix: &str) -> PathBuf {
    let id = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut path = std::env::temp_dir();
    path.push(format!(
        "evtlog_test_{}_{}_{}.seg",
        prefix,
        std::process::id(),
        id
    ));
    path
}

fn make_event(seq: u64, timestamp: i64) -> Event {
    let outcome = match seq % 3 {
        0 => Outcome::Unknown,
        1 => Outcome::Success,
        _ => Outcome::Failure,
    };

    Event {
        timestamp,
        event_code: (seq % u16::MAX as u64) as u16,
        outcome,
        location_id: (seq % 100) as u16,
        operation_id: (seq % 50) as u16,
        actor_id: seq as u32,
        target_id: (seq * 10) as u32,
        payload_id: (seq * 100) as u32,
    }
}

#[test]
fn test_write_and_sequential_read() {
    let path = temp_segment_path("seq_read");
    let segment_id = 42;
    let created_at = 1_700_000_000;
    let max_block_records = 3;

    let mut writer =
        SegmentWriter::create(&path, segment_id, created_at, max_block_records).unwrap();

    let expected_events: Vec<Event> = (0..7)
        .map(|i| make_event(i, created_at + (i as i64) * 100))
        .collect();

    for ev in &expected_events {
        writer.append(ev).unwrap();
    }
    let total_written = writer.finish().unwrap();

    let expected_file_len =
        (SEGMENT_HEADER_SIZE + 3 * BLOCK_HEADER_SIZE + 7 * BIN_SIZE) as u64;
    assert_eq!(total_written, expected_file_len);
    assert_eq!(fs::metadata(&path).unwrap().len(), expected_file_len);

    let mut reader = SegmentReader::open(&path).unwrap();
    assert_eq!(reader.header().segment_id, segment_id);
    assert_eq!(reader.header().created_at, created_at);

    let read_events: Vec<Event> = (&mut reader).map(|r| r.unwrap()).collect();
    assert_eq!(read_events, expected_events);
    assert_eq!(reader.current_offset(), expected_file_len);

    let _ = fs::remove_file(&path);
}

#[test]
fn test_random_access_read_at() {
    let path = temp_segment_path("read_at");
    let mut writer = SegmentWriter::create(&path, 1, 1000, 4).unwrap();

    let mut events = Vec::new();
    let mut refs = Vec::new();

    for i in 0..10 {
        let ev = make_event(i, 1000 + i as i64);
        let event_ref = writer.append(&ev).unwrap();
        events.push(ev);
        refs.push(event_ref);
    }
    writer.finish().unwrap();

    let mut reader = SegmentReader::open(&path).unwrap();

    for &idx in &[9, 4, 0, 7, 2, 8, 1, 5, 3, 6] {
        let ev = reader.read_at(refs[idx]).unwrap();
        assert_eq!(ev, events[idx], "Ошибка при чтении индекса {}", idx);
    }

    let mut bad_seg_ref = refs[0];
    bad_seg_ref.segment_id = 999;
    assert!(matches!(
        reader.read_at(bad_seg_ref),
        Err(SegmentError::SegmentIdMismatch {
            expected: 1,
            actual: 999
        })
    ));

    let mut oob_ref = refs[8];
    oob_ref.record_index = 2;
    assert!(matches!(
        reader.read_at(oob_ref),
        Err(SegmentError::RecordIndexOutOfBounds { index: 2, count: 2 })
    ));

    let _ = fs::remove_file(&path);
}

#[test]
fn test_recover_healthy_segment() {
    let path = temp_segment_path("recover_healthy");
    let mut writer = SegmentWriter::create(&path, 7, 5000, 2).unwrap();
    for i in 0..4 {
        writer.append(&make_event(i, 5000 + i as i64)).unwrap();
    }
    let valid_len = writer.finish().unwrap();

    let report = recover_segment(&path).unwrap();
    assert_eq!(report.valid_bytes, valid_len);
    assert_eq!(report.truncated_bytes, 0);
    assert_eq!(report.valid_blocks, 2);
    assert_eq!(report.valid_events, 4);
    assert!(report.corruption_reason.is_none());

    let _ = fs::remove_file(&path);
}

#[test]
fn test_recover_truncated_block_header_and_payload() {
    let path = temp_segment_path("recover_truncated");
    let mut writer = SegmentWriter::create(&path, 1, 1000, 2).unwrap();

    for i in 0..5 {
        writer.append(&make_event(i, 1000 + i as i64)).unwrap();
    }
    writer.finish().unwrap();

    let two_blocks_offset =
        (SEGMENT_HEADER_SIZE + 2 * (BLOCK_HEADER_SIZE + 2 * BIN_SIZE)) as u64;

    let file = OpenOptions::new().write(true).open(&path).unwrap();
    file.set_len(two_blocks_offset + BLOCK_HEADER_SIZE as u64 + 5)
        .unwrap();
    drop(file);

    let report = recover_segment(&path).unwrap();
    assert_eq!(report.valid_bytes, two_blocks_offset);
    assert_eq!(report.truncated_bytes, BLOCK_HEADER_SIZE as u64 + 5);
    assert_eq!(report.valid_blocks, 2);
    assert_eq!(report.valid_events, 4);
    assert!(report.corruption_reason.is_some());

    let reader = SegmentReader::open(&path).unwrap();
    let recovered_events: Vec<Event> = reader.map(|r| r.unwrap()).collect();
    assert_eq!(recovered_events.len(), 4);

    let mut file = OpenOptions::new().append(true).open(&path).unwrap();
    file.write_all(&[0xAB; 10]).unwrap();
    drop(file);

    let report2 = recover_segment(&path).unwrap();
    assert_eq!(report2.valid_bytes, two_blocks_offset);
    assert_eq!(report2.truncated_bytes, 10);
    assert_eq!(report2.valid_blocks, 2);
    assert_eq!(report2.valid_events, 4);

    let _ = fs::remove_file(&path);
}

#[test]
fn test_recover_corrupted_payload_crc() {
    let path = temp_segment_path("recover_crc");
    let mut writer = SegmentWriter::create(&path, 1, 1000, 2).unwrap();
    for i in 0..4 {
        writer.append(&make_event(i, 1000 + i as i64)).unwrap();
    }
    let full_len = writer.finish().unwrap();

    let first_block_end =
        (SEGMENT_HEADER_SIZE + BLOCK_HEADER_SIZE + 2 * BIN_SIZE) as u64;

    let mut file = OpenOptions::new().read(true).write(true).open(&path).unwrap();
    file.seek(SeekFrom::Start(full_len - 1)).unwrap();
    file.write_all(&[0xFF]).unwrap();
    drop(file);

    let mut reader = SegmentReader::open(&path).unwrap();
    let results: Vec<Result<Event, SegmentError>> = (&mut reader).collect();
    assert_eq!(results.len(), 3);
    assert!(matches!(
        results[2],
        Err(SegmentError::ChecksumMismatch { .. })
    ));

    let report = recover_segment(&path).unwrap();
    assert_eq!(report.valid_bytes, first_block_end);
    assert_eq!(report.truncated_bytes, full_len - first_block_end);
    assert_eq!(report.valid_blocks, 1);
    assert_eq!(report.valid_events, 2);

    let _ = fs::remove_file(&path);
}

#[test]
fn test_corrupted_segment_header_cannot_be_recovered() {
    let path = temp_segment_path("bad_seg_header");
    let mut writer = SegmentWriter::create(&path, 1, 1000, 2).unwrap();
    writer.append(&make_event(1, 1000)).unwrap();
    writer.finish().unwrap();

    let mut file = OpenOptions::new().write(true).open(&path).unwrap();
    file.seek(SeekFrom::Start(0)).unwrap();
    file.write_all(b"BADMAGIC").unwrap();
    drop(file);

    assert!(matches!(
        SegmentReader::open(&path),
        Err(SegmentError::HeaderChecksumMismatch { offset: 0 })
    ));
    assert!(matches!(
        recover_segment(&path),
        Err(SegmentError::InvalidMagic(_))
    ));

    let _ = fs::remove_file(&path);
}

#[test]
fn test_parallel_write_and_concurrent_read() {
    const NUM_THREADS: usize = 4;
    const EVENTS_PER_SEGMENT: usize = 250;
    const MAX_BLOCK_RECORDS: usize = 16;

    let mut write_handles = Vec::with_capacity(NUM_THREADS);

    for t in 0..NUM_THREADS {
        write_handles.push(thread::spawn(move || {
            let segment_id = (t + 1) as u32;
            let path = temp_segment_path(&format!("parallel_seg_{}", segment_id));
            let mut writer =
                SegmentWriter::create(&path, segment_id, 10_000, MAX_BLOCK_RECORDS).unwrap();

            let mut expected = Vec::with_capacity(EVENTS_PER_SEGMENT);
            let mut refs = Vec::with_capacity(EVENTS_PER_SEGMENT);

            for i in 0..EVENTS_PER_SEGMENT {
                let seq = (t * 10_000 + i) as u64;
                let ev = make_event(seq, 10_000 + i as i64);
                let ev_ref = writer.append(&ev).unwrap();
                expected.push(ev);
                refs.push(ev_ref);
            }

            writer.finish().unwrap();
            (path, Arc::new(expected), Arc::new(refs))
        }));
    }

    let segments: Vec<_> = write_handles
        .into_iter()
        .map(|h| h.join().unwrap())
        .collect();

    let mut read_handles = Vec::with_capacity(NUM_THREADS * 2);

    for (path, expected, refs) in &segments {
        let p_seq = path.clone();
        let exp_seq = Arc::clone(expected);
        read_handles.push(thread::spawn(move || {
            let reader = SegmentReader::open(&p_seq).unwrap();
            let actual: Vec<Event> = reader.map(|r| r.unwrap()).collect();
            assert_eq!(&actual, exp_seq.as_ref());
        }));

        let p_rnd = path.clone();
        let exp_rnd = Arc::clone(expected);
        let refs_rnd = Arc::clone(refs);
        read_handles.push(thread::spawn(move || {
            let mut reader = SegmentReader::open(&p_rnd).unwrap();
            for i in (0..EVENTS_PER_SEGMENT).rev() {
                let ev = reader.read_at(refs_rnd[i]).unwrap();
                assert_eq!(ev, exp_rnd[i]);
            }
        }));
    }

    for h in read_handles {
        h.join().unwrap();
    }

    for (path, _, _) in segments {
        let _ = fs::remove_file(path);
    }
}
