//! What a 7200 dpi frame costs to write to and read from the roll folder.
//! Run with `cargo test -p nv-store --release --test big_raw -- --ignored --nocapture`.

use nv_image::FrameBuf;
use nv_store::{RawReader, RawWriter};

#[test]
#[ignore = "writes and reads a 482 MB file"]
fn a_7200_dpi_frame_round_trips_in_reasonable_time() {
    let (w, h) = (10368u32, 7755u32);
    let mut buf = FrameBuf::new(w, Some(h), 16, 3);
    let row: Vec<u8> = (0..w * 3).flat_map(|i| (((i * 37) % 40000 + 5000) as u16).to_ne_bytes()).collect();
    for _ in 0..h {
        buf.append(&row);
    }

    let path = std::env::temp_dir().join("nv-big-raw.tif");
    let t = std::time::Instant::now();
    RawWriter::write_scan(&buf, &path).unwrap();
    let write = t.elapsed();

    let t = std::time::Instant::now();
    let back = RawReader::read(&path).unwrap();
    let read = t.elapsed();
    let _ = std::fs::remove_file(&path);
    eprintln!("write {write:?}, read {read:?}");

    assert_eq!((back.width, back.effective_height(), back.depth, back.channels), (w, h, 16, 3));
    assert_eq!(&back.as_bytes()[..row.len()], &row[..], "first row differs");
    assert!(read.as_secs() < 10, "read took {read:?}");
}
