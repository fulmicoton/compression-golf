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
