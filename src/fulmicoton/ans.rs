use super::bijection::Bijection;

pub struct AnsBijection;

const SCALE_BITS: u32 = 12;
const SCALE: u32 = 1 << SCALE_BITS;
const STATE_LOWER_BOUND: u32 = 1 << 16; 

impl Bijection<Vec<u8>, Vec<u8>> for AnsBijection {
    fn apply(&self, source: Vec<u8>) -> Vec<u8> {
        if source.is_empty() { return vec![]; }
        
        let mut counts = [0u32; 256];
        for &b in &source {
            counts[b as usize] += 1;
        }

        let mut normalized_counts = [0u16; 256];
        let total = source.len() as u64;
        let mut sum = 0u32;
        let mut max_symbol = 0;
        let mut max_count = 0;

        for i in 0..256 {
            if counts[i] > 0 {
                let mut c = (counts[i] as u64 * SCALE as u64 / total) as u32;
                if c == 0 { c = 1; }
                normalized_counts[i] = c as u16;
                sum += c;
                if c > max_count {
                    max_count = c;
                    max_symbol = i;
                }
            }
        }

        if sum != SCALE {
            let diff = SCALE as i32 - sum as i32;
            let val = normalized_counts[max_symbol] as i32 + diff;
            normalized_counts[max_symbol] = val as u16;
        }

        let mut starts = [0u32; 256];
        let mut current_start = 0;
        for i in 0..256 {
            starts[i] = current_start;
            current_start += normalized_counts[i] as u32;
        }

        let mut stream = Vec::new();
        let mut x = STATE_LOWER_BOUND;

        for &symbol in source.iter().rev() {
            let s = symbol as usize;
            let freq = normalized_counts[s] as u32;
            let start = starts[s];

            let bound = freq << (16 + 8 - SCALE_BITS);
            while x >= bound {
                stream.push(x as u8);
                x >>= 8;
            }

            x = ((x / freq) << SCALE_BITS) + (x % freq) + start;
        }

        let x_bytes = x.to_le_bytes();
        let mut result = Vec::with_capacity(512 + 4 + 4 + stream.len());
        for &c in &normalized_counts {
            result.extend_from_slice(&c.to_le_bytes());
        }
        result.extend_from_slice(&(source.len() as u32).to_le_bytes());
        result.extend_from_slice(&x_bytes);
        result.extend(stream.iter().rev());
        result
    }

    fn revert(&self, source: Vec<u8>) -> Vec<u8> {
        if source.is_empty() { return vec![]; }
        let mut cursor = 0;
        let mut normalized_counts = [0u16; 256];
        for i in 0..256 {
            let bytes = &source[cursor..cursor+2];
            normalized_counts[i] = u16::from_le_bytes([bytes[0], bytes[1]]);
            cursor += 2;
        }

        let mut cum_freq = [0u32; 257];
        let mut sum = 0;
        for i in 0..256 {
            cum_freq[i] = sum;
            sum += normalized_counts[i] as u32;
        }
        cum_freq[256] = sum;

        let mut symbol_map = [0u8; SCALE as usize];
        for s in 0..256 {
            let start = cum_freq[s] as usize;
            let end = cum_freq[s+1] as usize;
            for i in start..end {
                symbol_map[i] = s as u8;
            }
        }
        
        let len_bytes = &source[cursor..cursor+4];
        let length = u32::from_le_bytes([len_bytes[0], len_bytes[1], len_bytes[2], len_bytes[3]]) as usize;
        cursor += 4;
        
        let state_bytes = &source[cursor..cursor+4];
        let mut x = u32::from_le_bytes([state_bytes[0], state_bytes[1], state_bytes[2], state_bytes[3]]);
        cursor += 4;
        
        let mut output = Vec::with_capacity(length);
        let stream = &source[cursor..];
        let mut stream_ptr = 0;
        
        for _ in 0..length {
            let slot = (x & (SCALE - 1)) as usize;
            let s = symbol_map[slot];
            output.push(s);
            let freq = normalized_counts[s as usize] as u32;
            let start = cum_freq[s as usize];
            x = freq * (x >> SCALE_BITS) + (x & (SCALE - 1)) - start;
            while x < STATE_LOWER_BOUND {
                if stream_ptr >= stream.len() { break; }
                let byte = stream[stream_ptr] as u32;
                stream_ptr += 1;
                x = (x << 8) | byte;
            }
        }
        output
    }
}

pub struct AnsU64Bijection;

impl Bijection<Vec<u64>, Vec<u8>> for AnsU64Bijection {
    fn apply(&self, source: Vec<u64>) -> Vec<u8> {
        if source.is_empty() { return vec![]; }
        
        let mut unique_values: Vec<u64> = source.clone();
        unique_values.sort();
        unique_values.dedup();
        
        if unique_values.len() > 65536 {
            panic!("AnsU64Bijection: Too many unique values for u16 rank");
        }
        
        let ranks: Vec<u16> = source.iter().map(|&val| {
            unique_values.binary_search(&val).unwrap() as u16
        }).collect();
        
        let alphabet_size = unique_values.len();
        let mut counts = vec![0u32; alphabet_size];
        for &r in &ranks {
            counts[r as usize] += 1;
        }
        
        let mut normalized_counts = vec![0u16; alphabet_size];
        let total = source.len() as u64;
        let mut sum = 0u32;
        let mut max_symbol = 0;
        let mut max_count = 0;
        
        for i in 0..alphabet_size {
            if counts[i] > 0 {
                let mut c = (counts[i] as u64 * SCALE as u64 / total) as u32;
                if c == 0 { c = 1; }
                normalized_counts[i] = c as u16;
                sum += c;
                if c > max_count {
                    max_count = c;
                    max_symbol = i;
                }
            }
        }
        
        if sum != SCALE {
            let diff = SCALE as i32 - sum as i32;
            let val = normalized_counts[max_symbol] as i32 + diff;
            normalized_counts[max_symbol] = val as u16;
        }
        
        let mut starts = vec![0u32; alphabet_size];
        let mut current_start = 0;
        for i in 0..alphabet_size {
            starts[i] = current_start;
            current_start += normalized_counts[i] as u32;
        }
        
        let mut stream = Vec::new();
        let mut x = STATE_LOWER_BOUND;
        
        for &r in ranks.iter().rev() {
            let s = r as usize;
            let freq = normalized_counts[s] as u32;
            let start = starts[s];
            
            let bound = freq << (16 + 8 - SCALE_BITS);
            while x >= bound {
                stream.push(x as u8);
                x >>= 8;
            }
            x = ((x / freq) << SCALE_BITS) + (x % freq) + start;
        }
        
        let mut result = Vec::new();
        
        use super::bijection::{VIntBijection, ZStdBijection};
        let vint = VIntBijection;
        let zstd = ZStdBijection;
        let dict_bytes = zstd.apply(vint.apply(unique_values));
        
        write_vint_local(dict_bytes.len(), &mut result);
        result.extend(dict_bytes);
        
        for &c in &normalized_counts {
            result.extend_from_slice(&c.to_le_bytes());
        }
        
        result.extend_from_slice(&(source.len() as u32).to_le_bytes());
        result.extend_from_slice(&x.to_le_bytes());
        result.extend(stream.iter().rev());
        
        result
    }

    fn revert(&self, source: Vec<u8>) -> Vec<u64> {
        if source.is_empty() { return vec![]; }
        
        let mut offset = 0;
        let dict_len = read_vint_local(&source, &mut offset);
        let dict_bytes = &source[offset..offset+dict_len];
        offset += dict_len;
        
        use super::bijection::{VIntBijection, ZStdBijection};
        let vint = VIntBijection;
        let zstd = ZStdBijection;
        let unique_values = vint.revert(zstd.revert(dict_bytes.to_vec()));
        let alphabet_size = unique_values.len();
        
        let mut normalized_counts = Vec::with_capacity(alphabet_size);
        for _ in 0..alphabet_size {
            let bytes = &source[offset..offset+2];
            normalized_counts.push(u16::from_le_bytes([bytes[0], bytes[1]]));
            offset += 2;
        }
        
        let mut cum_freq = Vec::with_capacity(alphabet_size + 1);
        let mut sum = 0;
        for i in 0..alphabet_size {
            cum_freq.push(sum);
            sum += normalized_counts[i] as u32;
        }
        cum_freq.push(sum);
        
        let mut symbol_map = vec![0u16; SCALE as usize];
        for s in 0..alphabet_size {
            let start = cum_freq[s] as usize;
            let end = cum_freq[s+1] as usize;
            for i in start..end {
                symbol_map[i] = s as u16;
            }
        }
        
        let len_bytes = &source[offset..offset+4];
        let length = u32::from_le_bytes([len_bytes[0], len_bytes[1], len_bytes[2], len_bytes[3]]) as usize;
        offset += 4;
        
        let state_bytes = &source[offset..offset+4];
        let mut x = u32::from_le_bytes([state_bytes[0], state_bytes[1], state_bytes[2], state_bytes[3]]);
        offset += 4;
        
        let mut output = Vec::with_capacity(length);
        let stream = &source[offset..];
        let mut stream_ptr = 0;
        
        for _ in 0..length {
            let slot = (x & (SCALE - 1)) as usize;
            let rank = symbol_map[slot];
            output.push(unique_values[rank as usize]);
            
            let freq = normalized_counts[rank as usize] as u32;
            let start = cum_freq[rank as usize];
            
            x = freq * (x >> SCALE_BITS) + (x & (SCALE - 1)) - start;
            
            while x < STATE_LOWER_BOUND {
                if stream_ptr >= stream.len() { break; }
                let byte = stream[stream_ptr] as u32;
                stream_ptr += 1;
                x = (x << 8) | byte;
            }
        }
        
        output
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ans_roundtrip() {
        let ans = AnsBijection;
        
        let data = b"abracadabra".to_vec();
        let compressed = ans.apply(data.clone());
        let decompressed = ans.revert(compressed);
        
        assert_eq!(data, decompressed);
    }

    #[test]
    fn test_ans_u64_roundtrip() {
        let ans = AnsU64Bijection;
        let data = vec![1000, 2000, 1000, 3000, 2000, 1000, 5000];
        let compressed = ans.apply(data.clone());
        let decompressed = ans.revert(compressed);
        assert_eq!(data, decompressed);
    }
}
