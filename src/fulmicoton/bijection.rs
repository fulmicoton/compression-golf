use lzma::EXTREME_PRESET;
use std::error::Error;

use crate::zstd::ZstdCodec;

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
