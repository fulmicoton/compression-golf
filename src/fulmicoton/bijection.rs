use lzma::EXTREME_PRESET;
use std::error::Error;

use crate::zstd::ZstdCodec;

use super::adaptive_mix::AdaptiveMixCoder;
use super::algo::{identify_permutation, restore_permutation};
use super::ans::AnsU64Bijection;

pub trait Bijection<A, B> {
    fn apply(&self, source: A) -> B;
    fn revert(&self, source: B) -> A;
}

pub struct ZStdBijection;

impl Bijection<Vec<u8>, Vec<u8>> for ZStdBijection {
    fn apply(&self, source: Vec<u8>) -> Vec<u8> {
        zstd::encode_all(&source[..], 22).unwrap()
    }

    fn revert(&self, source: Vec<u8>) -> Vec<u8> {
        zstd::decode_all(&source[..]).unwrap()
    }
}

pub struct LzmaBijection;

impl Bijection<Vec<u8>, Vec<u8>> for LzmaBijection {
    fn apply(&self, source: Vec<u8>) -> Vec<u8> {
        lzma::compress(&source[..], 9 | EXTREME_PRESET).unwrap()
    }

    fn revert(&self, source: Vec<u8>) -> Vec<u8> {
        lzma::decompress(&source[..]).unwrap()
    }
}

pub struct VIntBijection;

impl Bijection<Vec<u64>, Vec<u8>> for VIntBijection {
    fn apply(&self, numbers: Vec<u64>) -> Vec<u8> {
        let mut buf = Vec::with_capacity(numbers.len() * 8); // Heuristic
        for &num in &numbers {
            let mut n = num;
            loop {
                let mut byte = (n & 0x7F) as u8;
                n >>= 7;
                if n != 0 {
                    byte |= 0x80;
                }
                buf.push(byte);
                if n == 0 {
                    break;
                }
            }
        }
        buf
    }

    fn revert(&self, bytes: Vec<u8>) -> Vec<u64> {
        let mut numbers = Vec::new();
        let mut n = 0u64;
        let mut shift = 0;
        for byte in bytes {
            n |= ((byte & 0x7F) as u64) << shift;
            if byte & 0x80 == 0 {
                numbers.push(n);
                n = 0;
                shift = 0;
            } else {
                shift += 7;
            }
        }
        numbers
    }
}

pub struct U24Bijection;

impl Bijection<Vec<u64>, Vec<u8>> for U24Bijection {
    fn apply(&self, numbers: Vec<u64>) -> Vec<u8> {
        let mut buf = Vec::with_capacity(numbers.len() * 3);
        for &num in &numbers {
            if num >= (1 << 24) {
                panic!("Index too large for u24");
            }
            buf.push(num as u8);
            buf.push((num >> 8) as u8);
            buf.push((num >> 16) as u8);
        }
        buf
    }

    fn revert(&self, bytes: Vec<u8>) -> Vec<u64> {
        if bytes.len() % 3 != 0 {
            panic!("Invalid length for u24 decoding");
        }
        let mut numbers = Vec::with_capacity(bytes.len() / 3);
        for chunk in bytes.chunks_exact(3) {
            let n = (chunk[0] as u64) | ((chunk[1] as u64) << 8) | ((chunk[2] as u64) << 16);
            numbers.push(n);
        }
        numbers
    }
}

// impl Bijection<Vec<u64>, Vec<u8>> for U24Bijection {
//     fn apply(&self, numbers: Vec<u64>) -> Vec<u8> {
//         let mut lanes = [Vec::new(), Vec::new(), Vec::new()];
//         for &num in &numbers {
//             if num >= (1 << 24) {
//                 panic!("Index too large for u24");
//             }
//             lanes[0].push(num as u8);
//             lanes[1].push((num >> 8) as u8);
//             lanes[2].push((num >> 16) as u8);
//         }
//         let buf: Vec<u8> = lanes.concat();
//         buf
//     }

//     fn revert(&self, bytes: Vec<u8>) -> Vec<u64> {
//         if bytes.len() % 3 != 0 {
//             panic!("Invalid length for u24 decoding");
//         }
//         let n =  bytes.len() / 3;
//         let mut numbers = Vec::with_capacity(n);
//         for i in 0..n {
//             let n = (bytes[i] as u64) | ((bytes[i + n] as u64) << 8) | ((bytes[i + 2*n] as u64) << 16);
//             numbers.push(n);
//         }
//         numbers
//     }
// }

pub struct DeltaBijection;

impl Bijection<Vec<i64>, Vec<i64>> for DeltaBijection {
    fn apply(&self, source: Vec<i64>) -> Vec<i64> {
        if source.is_empty() {
            return vec![];
        }
        let mut deltas = Vec::with_capacity(source.len());
        let mut prev = 0;
        for &val in &source {
            deltas.push(val - prev);
            prev = val;
        }
        deltas
    }

    fn revert(&self, source: Vec<i64>) -> Vec<i64> {
        let mut original = Vec::with_capacity(source.len());
        let mut prev = 0;
        for &delta in &source {
            let val = prev + delta;
            original.push(val);
            prev = val;
        }
        original
    }
}

pub struct PositiveDeltaBijection;

impl Bijection<Vec<i64>, Vec<u64>> for PositiveDeltaBijection {
    fn apply(&self, source: Vec<i64>) -> Vec<u64> {
        if source.is_empty() {
            return Vec::new();
        }
        let mut deltas = Vec::with_capacity(source.len());
        let mut prev = 0;
        for &val in &source {
            let delta = val - prev;
            if delta < 0 {
                panic!("Negative delta encountered in PositiveDeltaBijection");
            }
            deltas.push(delta as u64);
            prev = val;
        }
        deltas
    }

    fn revert(&self, source: Vec<u64>) -> Vec<i64> {
        let mut original = Vec::with_capacity(source.len());
        let mut prev = 0;
        for &delta in &source {
            let val = prev + (delta as i64);
            original.push(val);
            prev = val;
        }
        original
    }
}

pub struct U64DeltaBijection;

impl Bijection<Vec<u64>, Vec<u64>> for U64DeltaBijection {
    fn apply(&self, source: Vec<u64>) -> Vec<u64> {
        if source.is_empty() {
            return vec![];
        }
        let mut deltas = Vec::with_capacity(source.len());
        let mut prev = 0;
        for &val in &source {
            if val < prev {
                panic!("Non-positive delta encountered in U64DeltaBijection");
            }
            deltas.push(val - prev);
            prev = val;
        }
        deltas
    }

    fn revert(&self, source: Vec<u64>) -> Vec<u64> {
        let mut original = Vec::with_capacity(source.len());
        let mut prev = 0;
        for &delta in &source {
            let val = prev + delta;
            original.push(val);
            prev = val;
        }
        original
    }
}

pub struct EliasFanoBijection;

impl Bijection<Vec<u64>, Vec<u8>> for EliasFanoBijection {
    fn apply(&self, source: Vec<u64>) -> Vec<u8> {
        if source.is_empty() {
            return vec![];
        }

        let n = source.len() as u64;
        let max_val = *source.last().unwrap();

        // Calculate optimal low_bits: floor(log2(max_val / n))
        // If max_val < n, use 0 low bits
        let low_bits = if max_val >= n {
            (max_val / n).ilog2() as usize
        } else {
            0
        };
        let low_mask = (1u64 << low_bits).wrapping_sub(1);

        // Store low bits as fixed-width integers
        let mut low_parts: Vec<u64> = Vec::with_capacity(source.len());
        for &val in &source {
            low_parts.push(val & low_mask);
        }

        // Build high bits bitvector using unary coding
        // For each element, the high part is val >> low_bits
        // We encode as: (high - prev_high) zeros followed by a one
        let mut high_bits: Vec<u8> = Vec::new();
        let mut current_byte = 0u8;
        let mut bit_pos = 0usize;

        let mut prev_high = 0u64;
        for &val in &source {
            let high = val >> low_bits;
            let zeros = high - prev_high;

            // Write 'zeros' zero bits
            for _ in 0..zeros {
                // bit is 0, just advance
                bit_pos += 1;
                if bit_pos == 8 {
                    high_bits.push(current_byte);
                    current_byte = 0;
                    bit_pos = 0;
                }
            }

            // Write a 1 bit
            current_byte |= 1 << bit_pos;
            bit_pos += 1;
            if bit_pos == 8 {
                high_bits.push(current_byte);
                current_byte = 0;
                bit_pos = 0;
            }

            prev_high = high;
        }

        // Flush remaining bits
        if bit_pos > 0 {
            high_bits.push(current_byte);
        }

        // Serialize: [n: u32] [low_bits: u8] [high_bits_len: u32] [high_bits] [low_parts packed]
        let mut result = Vec::new();
        result.extend_from_slice(&(source.len() as u32).to_le_bytes());
        result.push(low_bits as u8);
        result.extend_from_slice(&(high_bits.len() as u32).to_le_bytes());
        result.extend_from_slice(&high_bits);

        // Pack low parts: each is low_bits wide
        if low_bits > 0 {
            let mut packed_low = Vec::new();
            let mut current: u64 = 0;
            let mut bits_in_current = 0usize;

            for &low in &low_parts {
                current |= low << bits_in_current;
                bits_in_current += low_bits;

                while bits_in_current >= 8 {
                    packed_low.push(current as u8);
                    current >>= 8;
                    bits_in_current -= 8;
                }
            }

            if bits_in_current > 0 {
                packed_low.push(current as u8);
            }

            result.extend_from_slice(&packed_low);
        }

        result
    }

    fn revert(&self, source: Vec<u8>) -> Vec<u64> {
        if source.is_empty() {
            return vec![];
        }

        let mut offset = 0;

        let n = u32::from_le_bytes([source[0], source[1], source[2], source[3]]) as usize;
        offset += 4;

        let low_bits = source[offset] as usize;
        offset += 1;

        let high_bits_len = u32::from_le_bytes([
            source[offset],
            source[offset + 1],
            source[offset + 2],
            source[offset + 3],
        ]) as usize;
        offset += 4;

        let high_bits = &source[offset..offset + high_bits_len];
        offset += high_bits_len;

        let packed_low = &source[offset..];
        let low_mask = (1u64 << low_bits).wrapping_sub(1);

        // Decode high bits from unary
        let mut values = Vec::with_capacity(n);
        let mut current_high = 0u64;
        let mut bit_idx = 0usize;

        while values.len() < n {
            let byte_idx = bit_idx / 8;
            let bit_in_byte = bit_idx % 8;

            if byte_idx >= high_bits.len() {
                break;
            }

            let bit = (high_bits[byte_idx] >> bit_in_byte) & 1;
            if bit == 1 {
                // Found an element
                values.push(current_high);
            } else {
                // Increment high value
                current_high += 1;
            }
            bit_idx += 1;
        }

        // Decode low bits and combine
        if low_bits > 0 {
            let mut low_bit_offset = 0usize;

            for i in 0..n {
                let byte_start = low_bit_offset / 8;
                let bit_start = low_bit_offset % 8;

                // Read up to 8 bytes to get enough bits
                let mut raw = 0u64;
                for j in 0..8 {
                    if byte_start + j < packed_low.len() {
                        raw |= (packed_low[byte_start + j] as u64) << (j * 8);
                    }
                }

                let low = (raw >> bit_start) & low_mask;
                values[i] = (values[i] << low_bits) | low;
                low_bit_offset += low_bits;
            }
        }

        values
    }
}

pub struct ZigZagBijection;

impl Bijection<Vec<i64>, Vec<u64>> for ZigZagBijection {
    fn apply(&self, source: Vec<i64>) -> Vec<u64> {
        let mut zigzags = Vec::with_capacity(source.len());
        for &n in &source {
            let encoded = ((n << 1) ^ (n >> 63)) as u64;
            zigzags.push(encoded);
        }
        zigzags
    }

    fn revert(&self, source: Vec<u64>) -> Vec<i64> {
        let mut original = Vec::with_capacity(source.len());
        for &n in &source {
            let decoded = ((n >> 1) as i64) ^ -((n & 1) as i64);
            original.push(decoded);
        }
        original
    }
}

pub struct HistogramBijection;

impl Bijection<Vec<i64>, Vec<u64>> for HistogramBijection {
    fn apply(&self, source: Vec<i64>) -> Vec<u64> {
        if source.is_empty() {
            return vec![];
        }

        let min = source[0];
        let max = source[source.len() - 1];

        // Ensure sorted
        for i in 0..source.len() - 1 {
            if source[i] > source[i + 1] {
                panic!("HistogramBijection: not sorted at index {}", i);
            }
        }

        let range = (max - min + 1) as usize;
        if range > 50_000_000 {
            panic!("HistogramBijection: range too large {}", range);
        }

        let mut counts = vec![0u64; range];
        for &val in &source {
            let idx = (val - min) as usize;
            counts[idx] += 1;
        }

        let mut output = Vec::with_capacity(1 + counts.len());
        output.push(min as u64); // Assume positive timestamp
        output.extend(counts);
        output
    }

    fn revert(&self, source: Vec<u64>) -> Vec<i64> {
        if source.is_empty() {
            return vec![];
        }

        let min = source[0] as i64;
        let counts = &source[1..];

        let total: u64 = counts.iter().sum();
        let mut output = Vec::with_capacity(total as usize);

        let mut current_val = min;
        for &count in counts {
            for _ in 0..count {
                output.push(current_val);
            }
            current_val += 1;
        }

        output
    }
}

pub struct MonotonicPermutationBijection;

impl Bijection<Vec<i64>, Vec<u8>> for MonotonicPermutationBijection {
    fn apply(&self, source: Vec<i64>) -> Vec<u8> {
        if source.is_empty() {
            return vec![];
        }

        let min_val = *source.iter().min().unwrap();
        let values: Vec<u64> = source.iter().map(|&x| (x - min_val) as u64).collect();

        let (sorted_u64, moves) = identify_permutation(values);

        // Encode sorted values using Histogram -> ANS(u64)
        let hist = HistogramBijection;
        let ans_u64 = AnsU64Bijection;
        let zstd = ZStdBijection;

        let sorted_i64: Vec<i64> = sorted_u64.iter().map(|&x| x as i64).collect();
        let hist_u64 = hist.apply(sorted_i64);

        // Use AnsU64Bijection directly on hist_u64
        let sorted_bytes = ans_u64.apply(hist_u64);

        // Encode moves: [Count][Adv...][Bub...] -> Zstd
        let mut moves_buf = Vec::new();
        let count = moves.len();
        write_vint_local(count, &mut moves_buf);

        for &(adv, _) in &moves {
            write_vint_local(adv, &mut moves_buf);
        }
        for &(_, bub) in &moves {
            write_vint_local(bub, &mut moves_buf);
        }

        let moves_compressed = zstd.apply(moves_buf);

        // Output: [min_val(8B)] [LenSorted][SortedBytes] [LenMoves][MovesBytes]
        let mut result = Vec::new();
        result.extend_from_slice(&min_val.to_le_bytes());

        write_vint_local(sorted_bytes.len(), &mut result);
        result.extend(sorted_bytes);

        write_vint_local(moves_compressed.len(), &mut result);
        result.extend(moves_compressed);

        result
    }

    fn revert(&self, source: Vec<u8>) -> Vec<i64> {
        if source.is_empty() {
            return vec![];
        }

        let mut offset = 0;
        let min_val_bytes = &source[offset..offset + 8];
        let min_val = i64::from_le_bytes([
            min_val_bytes[0],
            min_val_bytes[1],
            min_val_bytes[2],
            min_val_bytes[3],
            min_val_bytes[4],
            min_val_bytes[5],
            min_val_bytes[6],
            min_val_bytes[7],
        ]);
        offset += 8;

        let mut read_part = || {
            let len = read_vint_local(&source, &mut offset);
            let part = &source[offset..offset + len];
            offset += len;
            part.to_vec()
        };

        let sorted_bytes = read_part();
        let moves_compressed = read_part();

        let hist = HistogramBijection;
        let ans_u64 = AnsU64Bijection;
        let zstd = ZStdBijection;

        // Decode sorted values
        let hist_u64 = ans_u64.revert(sorted_bytes);
        let sorted_i64 = hist.revert(hist_u64);
        let values: Vec<u64> = sorted_i64.iter().map(|&x| x as u64).collect();

        // Decode moves
        let moves_raw = zstd.revert(moves_compressed);
        let mut moves = Vec::new();
        let mut m_offset = 0;
        if !moves_raw.is_empty() {
            let count = read_vint_local(&moves_raw, &mut m_offset);
            let mut advances = Vec::with_capacity(count);
            for _ in 0..count {
                advances.push(read_vint_local(&moves_raw, &mut m_offset));
            }
            let mut bubbles = Vec::with_capacity(count);
            for _ in 0..count {
                bubbles.push(read_vint_local(&moves_raw, &mut m_offset));
            }

            for (adv, bub) in advances.into_iter().zip(bubbles.into_iter()) {
                moves.push((adv, bub));
            }
        }

        let restored_values = restore_permutation(values, moves);

        restored_values
            .iter()
            .map(|&x| (x as i64) + min_val)
            .collect()
    }
}

pub struct MonotonicRepairBijection;

impl Bijection<Vec<i64>, Vec<u8>> for MonotonicRepairBijection {
    fn apply(&self, source: Vec<i64>) -> Vec<u8> {
        if source.is_empty() {
            return vec![];
        }

        let mut m = Vec::with_capacity(source.len());
        let mut offsets = Vec::with_capacity(source.len());
        let mut current_offset = 0i64;
        let mut prev_m = i64::MIN;

        for &t in &source {
            let mut val = t + current_offset;
            if val < prev_m {
                let diff = prev_m - val;
                current_offset += diff;
                val = prev_m;
            }
            m.push(val);
            offsets.push(current_offset);
            prev_m = val;
        }

        let hist = HistogramBijection;
        let vint = VIntBijection;
        let zstd = ZStdBijection;
        let pos_delta = PositiveDeltaBijection;

        let m_u64 = hist.apply(m);
        let m_bytes = zstd.apply(vint.apply(m_u64));

        let offsets_u64 = pos_delta.apply(offsets);
        let offsets_bytes = zstd.apply(vint.apply(offsets_u64));

        let mut result = Vec::new();
        write_vint_local(m_bytes.len(), &mut result);
        result.extend(m_bytes);
        write_vint_local(offsets_bytes.len(), &mut result);
        result.extend(offsets_bytes);

        result
    }

    fn revert(&self, source: Vec<u8>) -> Vec<i64> {
        if source.is_empty() {
            return vec![];
        }

        let mut offset = 0;
        let mut read_part = || {
            let len = read_vint_local(&source, &mut offset);
            let part = &source[offset..offset + len];
            offset += len;
            part.to_vec()
        };

        let m_bytes = read_part();
        let offsets_bytes = read_part();

        let hist = HistogramBijection;
        let vint = VIntBijection;
        let zstd = ZStdBijection;
        let pos_delta = PositiveDeltaBijection;

        let m = hist.revert(vint.revert(zstd.revert(m_bytes)));
        let offsets = pos_delta.revert(vint.revert(zstd.revert(offsets_bytes)));

        if m.len() != offsets.len() {
            panic!("MonotonicRepair: mismatch lengths");
        }

        let mut result = Vec::with_capacity(m.len());
        for (val, off) in m.iter().zip(offsets.iter()) {
            result.push(val - off);
        }

        result
    }
}

fn write_vint_local(mut n: usize, buf: &mut Vec<u8>) {
    loop {
        let mut byte = (n & 0x7F) as u8;
        n >>= 7;
        if n != 0 {
            byte |= 0x80;
        }
        buf.push(byte);
        if n == 0 {
            break;
        }
    }
}

fn read_vint_local(bytes: &[u8], offset: &mut usize) -> usize {
    let mut n = 0;
    let mut shift = 0;
    loop {
        let byte = bytes[*offset];
        *offset += 1;
        n |= ((byte & 0x7F) as usize) << shift;
        if byte & 0x80 == 0 {
            break;
        }
        shift += 7;
    }
    n
}

/// Binary Interpolative Coding (BIC) for sorted sequences
/// Recursively encodes middle elements using minimal bits based on range constraints
pub struct BicBijection;

impl Bijection<Vec<u64>, Vec<u8>> for BicBijection {
    fn apply(&self, source: Vec<u64>) -> Vec<u8> {
        if source.is_empty() {
            return vec![];
        }

        let n = source.len() as u64;
        let max_val = *source.last().unwrap();

        let mut writer = BitWriter::new();

        // Write header: n (u32) and max_val (u64)
        writer.write_bits(source.len() as u64, 32);
        writer.write_bits(max_val, 64);

        // Recursively encode
        bic_encode(&source, 0, max_val, &mut writer);

        writer.finish()
    }

    fn revert(&self, source: Vec<u8>) -> Vec<u64> {
        if source.is_empty() {
            return vec![];
        }

        let mut reader = BitReader::new(&source);

        let n = reader.read_bits(32) as usize;
        let max_val = reader.read_bits(64);

        let mut result = vec![0u64; n];
        bic_decode(&mut result, 0, max_val, &mut reader);

        result
    }
}

fn bic_encode(values: &[u64], lo: u64, hi: u64, writer: &mut BitWriter) {
    let n = values.len();
    if n == 0 {
        return;
    }

    let mid = n / 2;
    let m = values[mid];

    // Valid range for m: [lo + mid, hi - (n - 1 - mid)]
    // We need mid elements in [lo, m-1] and (n-1-mid) elements in [m+1, hi]
    let m_lo = lo + mid as u64;
    let m_hi = hi - (n - 1 - mid) as u64;

    // Number of possible values for m
    let range = m_hi - m_lo + 1;

    if range > 1 {
        // Calculate bits needed
        let bits = 64 - (range - 1).leading_zeros();
        writer.write_bits(m - m_lo, bits as usize);
    }
    // If range == 1, m is fully determined, no bits needed

    // Recurse on left and right halves
    if mid > 0 {
        bic_encode(&values[..mid], lo, m - 1, writer);
    }
    if mid + 1 < n {
        bic_encode(&values[mid + 1..], m + 1, hi, writer);
    }
}

fn bic_decode(values: &mut [u64], lo: u64, hi: u64, reader: &mut BitReader) {
    let n = values.len();
    if n == 0 {
        return;
    }

    let mid = n / 2;

    // Valid range for m
    let m_lo = lo + mid as u64;
    let m_hi = hi - (n - 1 - mid) as u64;

    let range = m_hi - m_lo + 1;

    let m = if range > 1 {
        let bits = 64 - (range - 1).leading_zeros();
        m_lo + reader.read_bits(bits as usize)
    } else {
        m_lo
    };

    values[mid] = m;

    // Recurse on left and right halves
    if mid > 0 {
        bic_decode(&mut values[..mid], lo, m - 1, reader);
    }
    if mid + 1 < n {
        bic_decode(&mut values[mid + 1..], m + 1, hi, reader);
    }
}

struct BitWriter {
    bytes: Vec<u8>,
    current: u64,
    bits_in_current: usize,
}

impl BitWriter {
    fn new() -> Self {
        BitWriter {
            bytes: Vec::new(),
            current: 0,
            bits_in_current: 0,
        }
    }

    fn write_bits(&mut self, value: u64, num_bits: usize) {
        if num_bits == 0 {
            return;
        }

        let mut value = value;
        let mut remaining = num_bits;

        while remaining > 0 {
            let space = 64 - self.bits_in_current;
            let to_write = remaining.min(space);

            let mask = if to_write >= 64 {
                u64::MAX
            } else {
                (1u64 << to_write) - 1
            };
            self.current |= (value & mask) << self.bits_in_current;
            self.bits_in_current += to_write;

            if to_write < 64 {
                value >>= to_write;
            } else {
                value = 0;
            }
            remaining -= to_write;

            // Flush full bytes
            while self.bits_in_current >= 8 {
                self.bytes.push(self.current as u8);
                self.current >>= 8;
                self.bits_in_current -= 8;
            }
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if self.bits_in_current > 0 {
            self.bytes.push(self.current as u8);
        }
        self.bytes
    }
}

struct BitReader<'a> {
    bytes: &'a [u8],
    byte_pos: usize,
    bit_pos: usize,
}

impl<'a> BitReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        BitReader {
            bytes,
            byte_pos: 0,
            bit_pos: 0,
        }
    }

    fn read_bits(&mut self, num_bits: usize) -> u64 {
        if num_bits == 0 {
            return 0;
        }

        let mut result = 0u64;
        let mut bits_read = 0;

        while bits_read < num_bits {
            if self.byte_pos >= self.bytes.len() {
                break;
            }

            let bits_available_in_byte = 8 - self.bit_pos;
            let bits_needed = num_bits - bits_read;
            let bits_to_read = bits_available_in_byte.min(bits_needed);

            let mask = ((1u16 << bits_to_read) - 1) as u8;
            let bits = (self.bytes[self.byte_pos] >> self.bit_pos) & mask;

            result |= (bits as u64) << bits_read;

            bits_read += bits_to_read;
            self.bit_pos += bits_to_read;

            if self.bit_pos >= 8 {
                self.bit_pos = 0;
                self.byte_pos += 1;
            }
        }

        result
    }
}

/// Adaptive arithmetic coding bijection for Vec<u8>
pub struct AdaptiveMixBijection;

impl Bijection<Vec<u8>, Vec<u8>> for AdaptiveMixBijection {
    fn apply(&self, source: Vec<u8>) -> Vec<u8> {
        if source.is_empty() {
            return vec![];
        }

        let mut coder = AdaptiveMixCoder::new();
        let encoded = coder.encode(&source);

        // Prepend the length as u32
        let mut result = Vec::with_capacity(4 + encoded.len());
        result.extend_from_slice(&(source.len() as u32).to_le_bytes());
        result.extend(encoded);
        result
    }

    fn revert(&self, source: Vec<u8>) -> Vec<u8> {
        if source.is_empty() {
            return vec![];
        }

        let len = u32::from_le_bytes([source[0], source[1], source[2], source[3]]) as usize;
        let encoded = &source[4..];

        let mut coder = AdaptiveMixCoder::new();
        coder.decode(encoded, len)
    }
}

/// Adaptive arithmetic coding bijection for Vec<u64> (assumes values fit in u8)
pub struct AdaptiveMixU64Bijection;

impl Bijection<Vec<u64>, Vec<u8>> for AdaptiveMixU64Bijection {
    fn apply(&self, source: Vec<u64>) -> Vec<u8> {
        if source.is_empty() {
            return vec![];
        }

        // Convert u64 to u8 (assumes all values fit)
        let data: Vec<u8> = source.iter().map(|&v| v as u8).collect();

        let mut coder = AdaptiveMixCoder::new();
        let encoded = coder.encode(&data);

        // Prepend the length as u32
        let mut result = Vec::with_capacity(4 + encoded.len());
        result.extend_from_slice(&(source.len() as u32).to_le_bytes());
        result.extend(encoded);
        result
    }

    fn revert(&self, source: Vec<u8>) -> Vec<u64> {
        if source.is_empty() {
            return vec![];
        }

        let len = u32::from_le_bytes([source[0], source[1], source[2], source[3]]) as usize;
        let encoded = &source[4..];

        let mut coder = AdaptiveMixCoder::new();
        let decoded = coder.decode(encoded, len);

        // Convert u8 back to u64
        decoded.iter().map(|&v| v as u64).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_adaptive_mix_u64_bijection() {
        let bij = AdaptiveMixU64Bijection;

        // Test with typical delta values (small numbers)
        let deltas: Vec<u64> = vec![1, 2, 1, 3, 1, 1, 2, 5, 1, 2, 1, 1, 3, 2, 1];
        let encoded = bij.apply(deltas.clone());
        let decoded = bij.revert(encoded);
        assert_eq!(deltas, decoded);
    }

    #[test]
    fn test_adaptive_mix_u64_bijection_large() {
        let bij = AdaptiveMixU64Bijection;

        // Test with larger data
        let deltas: Vec<u64> = (0..10000).map(|i| ((i % 10) + 1) as u64).collect();
        let encoded = bij.apply(deltas.clone());
        let decoded = bij.revert(encoded.clone());
        assert_eq!(deltas, decoded);

        // Should compress well
        assert!(encoded.len() < deltas.len(), "Expected compression");
    }

    #[test]
    fn test_adaptive_mix_u64_bijection_empty() {
        let bij = AdaptiveMixU64Bijection;
        let empty: Vec<u64> = vec![];
        let encoded = bij.apply(empty.clone());
        let decoded = bij.revert(encoded);
        assert_eq!(empty, decoded);
    }
}
