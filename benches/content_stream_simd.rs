//! A/B benchmark: SWAR `utf16_len_of_utf8` vs a byte-at-a-time scalar
//! baseline, across realistic content-stream workloads.
//!
//! Inlines both implementations so the benchmark measures the algorithm,
//! not the crate boundary.  Matches the pattern used in `simd_comparison.rs`.
//!
//! Run with:
//!   cargo bench --bench content_stream_simd

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

// ---- OLD implementation (scalar lead-byte walk) ----

mod scalar {
    #[inline]
    pub fn utf16_len_of_utf8(bytes: &[u8]) -> u32 {
        let mut units: u32 = 0;
        let mut i = 0;
        while i < bytes.len() {
            let b = bytes[i];
            let (adv, u) = if b < 0x80 {
                (1, 1)
            } else if b < 0xC0 {
                (1, 0)
            } else if b < 0xE0 {
                (2, 1)
            } else if b < 0xF0 {
                (3, 1)
            } else {
                (4, 2)
            };
            i += adv;
            units = units.saturating_add(u);
        }
        units
    }
}

// ---- NEW implementation (8-byte SWAR + Mycroft zero-byte) ----

mod swar {
    #[inline]
    pub fn utf16_len_of_utf8(bytes: &[u8]) -> u32 {
        let total = bytes.len();

        let mut cont: u64 = 0;
        let mut four: u64 = 0;

        let mut chunks = bytes.chunks_exact(8);
        for chunk in &mut chunks {
            let arr: [u8; 8] = [
                chunk[0], chunk[1], chunk[2], chunk[3], chunk[4], chunk[5], chunk[6], chunk[7],
            ];
            let w = u64::from_ne_bytes(arr);

            let cont_mask = (w & 0xC0C0_C0C0_C0C0_C0C0) ^ 0x8080_8080_8080_8080;
            cont = cont.wrapping_add(count_zero_bytes_u64(cont_mask) as u64);

            let four_mask = (w & 0xF0F0_F0F0_F0F0_F0F0) ^ 0xF0F0_F0F0_F0F0_F0F0;
            four = four.wrapping_add(count_zero_bytes_u64(four_mask) as u64);
        }

        for &b in chunks.remainder() {
            if (b & 0xC0) == 0x80 {
                cont += 1;
            }
            if b >= 0xF0 {
                four += 1;
            }
        }

        let units = (total as u64).saturating_sub(cont).saturating_add(four);
        units.min(u32::MAX as u64) as u32
    }

    #[inline(always)]
    fn count_zero_bytes_u64(v: u64) -> u32 {
        let mask = v.wrapping_sub(0x0101_0101_0101_0101) & !v & 0x8080_8080_8080_8080;
        mask.count_ones()
    }
}

// ---- Workloads ----

fn ascii_payload(n_bytes: usize) -> Vec<u8> {
    // Realistic-ish HTML, ASCII-heavy.
    let tpl = b"<p>Lorem ipsum dolor sit amet consectetur adipiscing elit.</p>";
    tpl.iter().cycle().take(n_bytes).copied().collect()
}

fn mixed_payload(n_bytes: usize) -> Vec<u8> {
    // Mix of ASCII, 2-byte (é), 3-byte (€), 4-byte (😀).
    let tpl = "Hello héllo 世界 🌍 lorem ipsum. ";
    let bytes = tpl.as_bytes();
    let mut out = Vec::with_capacity(n_bytes + bytes.len());
    while out.len() < n_bytes {
        out.extend_from_slice(bytes);
    }
    out.truncate(n_bytes);
    // Ensure we don't truncate mid-sequence — back up to a safe boundary.
    while !out.is_empty() && (out[out.len() - 1] & 0xC0) == 0x80 {
        out.pop();
    }
    out
}

fn three_byte_heavy(n_bytes: usize) -> Vec<u8> {
    // "日" (U+65E5) = 3-byte UTF-8, 1 UTF-16 unit.
    let tpl = "日本語テキスト。";
    let bytes = tpl.as_bytes();
    let mut out = Vec::with_capacity(n_bytes + bytes.len());
    while out.len() < n_bytes {
        out.extend_from_slice(bytes);
    }
    out.truncate(n_bytes);
    while !out.is_empty() && (out[out.len() - 1] & 0xC0) == 0x80 {
        out.pop();
    }
    out
}

fn four_byte_heavy(n_bytes: usize) -> Vec<u8> {
    // Emoji (4-byte UTF-8, 2 UTF-16 units each).
    let tpl = "😀🌍🚀🔥💯";
    let bytes = tpl.as_bytes();
    let mut out = Vec::with_capacity(n_bytes + bytes.len());
    while out.len() < n_bytes {
        out.extend_from_slice(bytes);
    }
    out.truncate(n_bytes);
    while !out.is_empty() && (out[out.len() - 1] & 0xC0) == 0x80 {
        out.pop();
    }
    out
}

// ---- Bench groups ----

fn bench_workload(c: &mut Criterion, label: &str, payload: Vec<u8>) {
    // Sanity: both implementations must agree on every benchmarked input.
    let scalar_n = scalar::utf16_len_of_utf8(&payload);
    let swar_n = swar::utf16_len_of_utf8(&payload);
    assert_eq!(
        scalar_n, swar_n,
        "{label}: scalar and SWAR disagreed (scalar={scalar_n}, swar={swar_n})"
    );

    let mut group = c.benchmark_group(format!("utf16_len_of_utf8/{label}"));
    group.throughput(Throughput::Bytes(payload.len() as u64));

    group.bench_with_input(
        BenchmarkId::new("scalar", payload.len()),
        &payload,
        |b, p| b.iter(|| scalar::utf16_len_of_utf8(black_box(p))),
    );
    group.bench_with_input(BenchmarkId::new("swar", payload.len()), &payload, |b, p| {
        b.iter(|| swar::utf16_len_of_utf8(black_box(p)))
    });

    group.finish();
}

fn bench_all(c: &mut Criterion) {
    // 64 KiB is one default chunk; 1 MiB is a medium doc slice.
    for &size in &[64 * 1024, 1024 * 1024] {
        bench_workload(c, &format!("ascii_{size}"), ascii_payload(size));
        bench_workload(c, &format!("mixed_{size}"), mixed_payload(size));
        bench_workload(c, &format!("three_byte_{size}"), three_byte_heavy(size));
        bench_workload(c, &format!("four_byte_{size}"), four_byte_heavy(size));
    }
}

criterion_group!(benches, bench_all);
criterion_main!(benches);
