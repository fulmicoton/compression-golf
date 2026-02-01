use std::error::Error;

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

pub struct DeltaBijection;

impl Bijection<Vec<i64>, Vec<i64>> for DeltaBijection {
    fn apply(&self, source: Vec<i64>) -> Vec<i64> {
        if source.is_empty() { return vec![]; }
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
        if source.is_empty() { return vec![]; }
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
        if source.is_empty() { return vec![]; }
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
        if source.is_empty() { return vec![]; }
        
        let min = source[0];
        let max = source[source.len() - 1];
        
        // Ensure sorted
        for i in 0..source.len()-1 {
            if source[i] > source[i+1] {
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
        if source.is_empty() { return vec![]; }
        
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

pub struct MonotonicRepairBijection;

impl Bijection<Vec<i64>, Vec<u8>> for MonotonicRepairBijection {
    fn apply(&self, source: Vec<i64>) -> Vec<u8> {
        if source.is_empty() { return vec![]; }
        
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
        
        // Encode M (monotonic)
        let m_u64 = hist.apply(m);
        let m_bytes = zstd.apply(vint.apply(m_u64));
        
        // Encode Offsets (non-decreasing)
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
        if source.is_empty() { return vec![]; }
        
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
