use super::bijection::{Bijection, HistogramBijection, VIntBijection};
use super::ans::AnsBijection;
use super::algo::{identify_permutation, restore_permutation};

pub struct TimestampCodec;

impl Bijection<Vec<i64>, Vec<u8>> for TimestampCodec {
    fn apply(&self, source: Vec<i64>) -> Vec<u8> {
        if source.is_empty() { return vec![]; }

        // 1. Normalize to u64 offsets (handling potential negative min?)
        // The problem is Histogram expects i64 sorted.
        // IdentifyPermutation works on u64.
        
        let min_val = *source.iter().min().unwrap();
        let offsets: Vec<u64> = source.iter().map(|&x| (x - min_val) as u64).collect();
        
        // 2. Identify Permutation (Sort)
        let (sorted_offsets, moves) = identify_permutation(offsets);
        
        // 3. Encode Moves (VInt)
        let mut moves_buf = Vec::new();
        // Store number of moves first? Or just stream?
        // Sequence of (adv, bubble).
        // Since we don't know the count, we can prefix length of buffer?
        // Or write count of moves.
        write_vint(moves.len() as u64, &mut moves_buf);
        for (adv, bubble) in moves {
            write_vint(adv as u64, &mut moves_buf);
            write_vint(bubble as u64, &mut moves_buf);
        }
        
        // 4. Encode Sorted (Histogram -> VInt -> ANS)
        let hist = HistogramBijection;
        // Histogram expects Vec<i64>. sorted_offsets is Vec<u64>.
        // They are offsets, so fit in i64.
        let sorted_i64: Vec<i64> = sorted_offsets.iter().map(|&x| x as i64).collect();
        let counts_u64 = hist.apply(sorted_i64); // [min_offset, count0, count1...]
        
        // Split min (which is 0 because we subtracted min_val? No, sorted_offsets[0] is 0.)
        // Histogram returns [min_val, counts...]. min_val of sorted_offsets is 0.
        // So counts_u64[0] should be 0.
        
        let counts = &counts_u64[1..]; // Skip the min (we know it's 0 relative to offset)
        
        // VInt encode counts to bytes
        let vint = VIntBijection;
        let counts_bytes = vint.apply(counts.to_vec());
        
        // ANS encode counts bytes
        let ans = AnsBijection;
        let counts_compressed = ans.apply(counts_bytes);
        
        // 5. Combine
        // [min_val (i64, VInt/ZigZag?)] [len_moves (vint)] [moves_buf] [counts_compressed]
        // min_val is i64. VInt handles u64. Cast to u64 (bit pattern).
        let mut result = Vec::new();
        write_vint(min_val as u64, &mut result); // Store original min
        
        write_vint(moves_buf.len() as u64, &mut result);
        result.extend(moves_buf);
        
        result.extend(counts_compressed); // Rest is counts
        
        result
    }

    fn revert(&self, source: Vec<u8>) -> Vec<i64> {
        if source.is_empty() { return vec![]; }
        
        let mut offset = 0;
        let min_val = read_vint(&source, &mut offset) as i64;
        
        let moves_len = read_vint(&source, &mut offset) as usize;
        let moves_end = offset + moves_len;
        let moves_slice = &source[offset..moves_end];
        offset = moves_end;
        
        // Decode Moves
        let mut m_offset = 0;
        let moves_count = read_vint_slice(moves_slice, &mut m_offset) as usize;
        let mut moves = Vec::with_capacity(moves_count);
        for _ in 0..moves_count {
            let adv = read_vint_slice(moves_slice, &mut m_offset) as usize;
            let bubble = read_vint_slice(moves_slice, &mut m_offset) as usize;
            moves.push((adv, bubble));
        }
        
        // Decode Counts
        let counts_compressed = source[offset..].to_vec();
        let ans = AnsBijection;
        let counts_bytes = ans.revert(counts_compressed);
        
        let vint = VIntBijection;
        let counts = vint.revert(counts_bytes);
        
        // Reconstruct Sorted
        let hist = HistogramBijection;
        // Reconstruct [min, counts...]
        let mut hist_input = Vec::with_capacity(counts.len() + 1);
        hist_input.push(0); // relative min is 0
        hist_input.extend(counts);
        
        let sorted_i64 = hist.revert(hist_input);
        let sorted_offsets: Vec<u64> = sorted_i64.iter().map(|&x| x as u64).collect();
        
        // Restore Permutation
        let restored_offsets = restore_permutation(sorted_offsets, moves);
        
        // Add min_val
        restored_offsets.iter().map(|&x| (x as i64) + min_val).collect()
    }
}

fn write_vint(mut n: u64, buf: &mut Vec<u8>) {
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

fn read_vint(bytes: &[u8], offset: &mut usize) -> u64 {
    let mut n = 0;
    let mut shift = 0;
    loop {
        if *offset >= bytes.len() {
            panic!("TimestampCodec: Unexpected EOF");
        }
        let byte = bytes[*offset];
        *offset += 1;
        n |= ((byte & 0x7F) as u64) << shift;
        if byte & 0x80 == 0 {
            break;
        }
        shift += 7;
    }
    n
}

fn read_vint_slice(bytes: &[u8], offset: &mut usize) -> u64 {
    read_vint(bytes, offset)
}