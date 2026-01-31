use super::bijection::Bijection;

pub struct AnsBijection;

const SCALE_BITS: u32 = 12;
const SCALE: u32 = 1 << SCALE_BITS;
const STATE_LOWER_BOUND: u32 = 1 << 16; // Renormalization limit

impl Bijection<Vec<u8>, Vec<u8>> for AnsBijection {
    fn apply(&self, source: Vec<u8>) -> Vec<u8> {
        if source.is_empty() {
            return vec![];
        }

        // 1. Frequency Analysis
        let mut counts = [0u32; 256];
        for &b in &source {
            counts[b as usize] += 1;
        }

        // 2. Normalization (to sum == SCALE)
        let mut normalized_counts = [0u16; 256];
        let total = source.len() as u64;
        let mut sum = 0u32;
        let mut max_symbol = 0;
        let mut max_count = 0;

        for i in 0..256 {
            if counts[i] > 0 {
                // Ensure at least 1
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

        // Adjust sum to match SCALE exactly
        if sum != SCALE {
            let diff = SCALE as i32 - sum as i32;
            let val = normalized_counts[max_symbol] as i32 + diff;
            if val <= 0 {
                // This case is rare/edge: simplistic handling, force 1 and fix others?
                // For simplicity, just pick first non-zero to absorb if max fails (unlikely with reasonable data)
                // Panic safe approach:
                panic!("ANS Normalization failed unexpectedly");
            }
            normalized_counts[max_symbol] = val as u16;
        }

        // 3. Precompute Cumulative Frequencies
        let mut starts = [0u32; 256];
        let mut current_start = 0;
        for i in 0..256 {
            starts[i] = current_start;
            current_start += normalized_counts[i] as u32;
        }

        // 4. Encode (Backwards)
        let mut stream = Vec::new();
        let mut x = STATE_LOWER_BOUND; // Initial state

        for &symbol in source.iter().rev() {
            let s = symbol as usize;
            let freq = normalized_counts[s] as u32;
            let start = starts[s];

            // Renormalize (emit bytes to keep x within bounds after update)
            // x_new = ((x / freq) << SCALE_BITS) + (x % freq) + start
            // We need x_new < 2^32 approx (or whatever our upper bound implies).
            // Actually standard rANS re-normalization:
            // max_x = (STATE_LOWER_BOUND * SCALE) - 1; (roughly)
            // while x >= (freq * STATE_LOWER_BOUND) ...? No.
            // Condition: x' = C(x, s). We want x before C to be such that x' is within bounds.
            // Standard:
            // while x >= ((STATE_LOWER_BOUND / freq) << SCALE_BITS) --> this is not quite right for byte output.
            // Valid range for x is [L, b*L - 1].
            // Here we use byte-aligned rANS.
            // while x >= freq * (1 << (32 - SCALE_BITS)) ?
            // Simplified: while x >= freq * 256 (heuristic for 16-bit L)
            // Let's stick to the reference logic:
            // x_max = (STATE_LOWER_BOUND >> SCALE_BITS) << 8 * freq; -- complex.
            //
            // Correct check for byte output:
            // max_val = ((1 << 16) * freq) >> 12; ?
            // If we assume STATE_LOWER_BOUND = 65536.
            // We want resulting x to fit in u32?
            // Actually, we output bytes *before* updating state.
            // while (x >= (freq << (32 - SCALE_BITS))) // if 32-bit state
            // But we typically use a threshold per symbol.
            // let limit = (STATE_LOWER_BOUND >> SCALE_BITS) << 8; // No.
            //
            // Let's use the property:
            // x must be small enough so that `(x / freq) << SCALE` doesn't overflow or exceed range?
            // Renormalize down:
            // while x >= (freq << (31 - SCALE_BITS)) { // safe upper bound?
            //   stream.push(x as u8);
            //   x >>= 8;
            // }
            // Correct check:
            // MAX_X = (1 << 31) - 1;
            // We want x_new <= MAX_X.
            // x_new ~= (x / freq) * SCALE.
            // So x / freq * SCALE <= MAX_X => x <= MAX_X / SCALE * freq.
            // Let's simplify: L = 2^16.
            // We verify x >= L * scale / freq ???
            //
            // Standard rANS C implementation:
            // R = 1 << SCALE_BITS;
            // mask = R - 1;
            // while (x >= ((frequency[s] << 16))) { // Assuming L=2^16? No.
            //   *ptr++ = x & 0xFF;
            //   x >>= 8;
            // }
            // This is for L=2^24?
            // Let's pick L = 1 << 16.
            // The upper bound of the interval for symbol s is `freq * L`. (?)
            // If x >= freq * (1 << (32 - SCALE_BITS)) -> output.
            // Let's assume 32-bit state.
            // L = 1<<16.
            // while x >= (freq << (32 - 12 - 8))?? No.
            //
            // Let's use the explicit bound:
            // Max allowed `x` before encoding `s` is determined so that `x_new` fits in 32 bits?
            // Actually, standard is:
            // while (x >= (freq << (16))) { // If using 16-bit renormalization
            //    stream.push(x as u8);
            //    x >>= 8;
            // }
            // Let's try this. If L=2^16.

            let max_val = (STATE_LOWER_BOUND >> SCALE_BITS) << 8; // This seems small if L=2^16, Scale=12. (64/4096)*256 = 4. Bad.

            // Let's use a standard setup:
            // L = 2^16.
            // while x >= (freq << 10) { ... } ?
            //
            // Let's follow Fabian Giesen's rANS.
            // L = 1 << 16.
            // x is [L, ...].
            // Decoding consumes bytes to keep x >= L.
            // Encoding emits bytes to keep x < ???
            //
            // x_new = ((x / freq) << SCALE) + (x % freq) + start.
            // We need x to be such that we can decode it.
            // For decoding, we need x >= L.
            // So for encoding, we need to output bytes if x becomes too large.
            // Bound: x >= ((L / freq) << 8) * freq? No.
            //
            // Correct inverse renormalization for encoding:
            // while (x >= bound[s]) { output byte; x >>= 8; }
            // where bound[s] = (L >> SCALE_BITS) << 8 * freq; // Still feels wrong.
            //
            // Let's try:
            // L = 1 << 23; (8MB range).
            // Scale = 12.
            // x >= (freq << (23 + 8 - 12)) ??
            //
            // Let's assume x is u32. L = 1 << 16.
            // We want to keep x roughly in [L, L*256).
            // But strict condition depends on freq.
            // Max x allowed = (freq * L) >> SCALE_BITS ?? No.
            //
            // Let's look at `constriction` or similar logic memory.
            // x_new = floor(x / freq) * 2^12 + (x % freq) + start.
            // We require x_new < 2^32 (approx).
            // So floor(x / freq) * 2^12 < 2^32.
            // x / freq < 2^20.
            // x < freq * 2^20.
            // If L=2^16. This works.
            // But we need x_new to be decodable. The decoder ensures x \in [L, ...).
            // The encoder must ensure x stays within a range that maps to [L, ...).
            //
            // Actually, the condition is:
            // while (x >= (freq << (32 - SCALE_BITS))) // for 32-bit output?
            // If using byte-wise:
            // bound = (L / freq) << 8; -- if scale matches?
            //
            // Let's use:
            // L = 1 << 16.
            // while (x >= ((1 << 16) * freq)) { // Wait, 1<<16 * freq might be large.
            //    stream.push(x as u8);
            //    x >>= 8;
            // }
            // NO. The standard check is `x >= ((frequency * L) >> SCALE_BITS) << 8`?
            //
            // Let's go with a safe, conservative bound.
            // We want `x_new` to roughly fit in u32.
            // `x_new ~= x * (SCALE / freq)`.
            // We want `x` to be reduced if `x_new` would overflow or exceed L*256?
            //
            // Let's use Ryg's rANS implementation values:
            // L = 1 << 16.
            // while (x >= ((freq << 16) >> SCALE_BITS) << 8) { // If SCALE=12, (freq << 4) << 8 = freq << 12.
            //     stream.push(x as u8);
            //     x >>= 8;
            // }
            // Let's trace:
            // If freq=1. Bound = 1 << 12 = 4096.
            // If x=4096. output byte. x=16.
            // x_new = (16/1)*4096 + ... = 65536 + ... > L. Good.
            // If freq=4096 (max). Bound = (4096 << 4) << 8 = 16M.
            // If x >= 16M...

            let bound = (freq << (16 + 8 - SCALE_BITS));
            while x >= bound {
                stream.push(x as u8);
                x >>= 8;
            }

            x = ((x / freq) << SCALE_BITS) + (x % freq) + start;
        }

        // 5. Finalize
        // Stream is backwards. Output x (4 bytes, Little Endian).
        let x_bytes = x.to_le_bytes();

        // Structure: [Freqs 512B] [OriginalLen 4B] [FinalState 4B] [Stream (Reversed)]
        let mut result = Vec::with_capacity(512 + 4 + 4 + stream.len());

        // Freqs
        for &c in &normalized_counts {
            result.extend_from_slice(&c.to_le_bytes());
        }

        // Length
        result.extend_from_slice(&(source.len() as u32).to_le_bytes());

        // State
        result.extend_from_slice(&x_bytes);

        // Stream (it was pushed backwards, so reading it backwards = forward stream order for decoder?)
        // Wait, encoder emits bytes. Decoder reads bytes.
        // Encoder: `stream.push` appends. `x` reduces.
        // Decoder: `x` increases. `x = (x << 8) | byte`.
        // So decoder consumes bytes in the REVERSE order of generation?
        // Standard rANS: Encoder stack (LIFO).
        // So `stream` contains [byte_N, ..., byte_1].
        // Decoder needs [byte_N, ..., byte_1] if it pops?
        // Or if it reads from stream pointer?
        // `x = (x << 8) | *ptr++`.
        // This corresponds to consuming the bytes generated "last" (high x) first?
        // No. Re-normalization: `x >>= 8`. Byte is `x & 0xFF`.
        // `x` was large. We chopped off the bottom byte.
        // Decoder: `x` is small. We shift up and add the byte.
        // So decoder must process the bytes in the REVERSE order of emission.
        // Emission was `push`. So `stream` is `[b1, b2, b3]`. `b3` was emitted last.
        // Decoder needs to read `b3` first.
        // So we should reverse `stream` in the file?
        // If we write `stream` as `[b1, b2, b3]`.
        // Decoder reads `b3`.
        // So Decoder reads from End of stream?
        // Usually we prefer forward reading.
        // So we should write `[b3, b2, b1]`.
        // So `result.extend(stream.iter().rev())`.

        result.extend(stream.iter().rev());

        result
    }

    fn revert(&self, source: Vec<u8>) -> Vec<u8> {
        if source.is_empty() { return vec![]; }

        let mut cursor = 0;

        // 1. Parse Freqs
        let mut normalized_counts = [0u16; 256];
        for i in 0..256 {
            let bytes = &source[cursor..cursor+2];
            normalized_counts[i] = u16::from_le_bytes([bytes[0], bytes[1]]);
            cursor += 2;
        }

        // Build tables
        let mut cum_freq = [0u32; 257]; // 257 for convenience? or 256
        let mut sum = 0;
        for i in 0..256 {
            cum_freq[i] = sum;
            sum += normalized_counts[i] as u32;
        }
        cum_freq[256] = sum; // Should be SCALE

        // Inverse mapping: cum_freq -> symbol
        // For O(1) lookup: array of size SCALE mapping value to symbol.
        let mut symbol_map = [0u8; SCALE as usize];
        for s in 0..256 {
            let start = cum_freq[s] as usize;
            let end = cum_freq[s+1] as usize;
            for i in start..end {
                symbol_map[i] = s as u8;
            }
        }

        // 2. Parse Length
        let len_bytes = &source[cursor..cursor+4];
        let length = u32::from_le_bytes([len_bytes[0], len_bytes[1], len_bytes[2], len_bytes[3]]) as usize;
        cursor += 4;

        // 3. Parse Initial State
        let state_bytes = &source[cursor..cursor+4];
        let mut x = u32::from_le_bytes([state_bytes[0], state_bytes[1], state_bytes[2], state_bytes[3]]);
        cursor += 4;

        // 4. Decode
        let mut output = Vec::with_capacity(length);
        let stream = &source[cursor..];
        let mut stream_ptr = 0;

        for _ in 0..length {
            // Decode symbol
            let slot = (x & (SCALE - 1)) as usize;
            let s = symbol_map[slot];
            output.push(s);

            let freq = normalized_counts[s as usize] as u32;
            let start = cum_freq[s as usize];

            // Advance state
            // x = freq * (x >> SCALE_BITS) + (x & mask) - start
            // Logic:
            // x_new = ((x / freq) << SCALE) + remainder + start
            // We inverted this?
            // Decoder:
            // slot = x & mask;
            // s = symbol(slot);
            // x = freq * (x >> SCALE) + slot - start;
            // Correct.

            x = freq * (x >> SCALE_BITS) + (x & (SCALE - 1)) - start;

            // Renormalize (fill x with bytes)
            while x < STATE_LOWER_BOUND {
                if stream_ptr >= stream.len() {
                    // Should not happen if stream is correct
                    break;
                }
                let byte = stream[stream_ptr] as u32;
                stream_ptr += 1;
                x = (x << 8) | byte;
            }
        }

        output
    }
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
    fn test_ans_empty() {
        let ans = AnsBijection;
        let data = vec![];
        let compressed = ans.apply(data.clone());
        let decompressed = ans.revert(compressed);
        assert_eq!(data, decompressed);
    }

    #[test]
    fn test_ans_repetitive() {
        let ans = AnsBijection;
        let data = vec![b'a'; 100];
        let compressed = ans.apply(data.clone());

        // Freq table (512) + Len (4) + State (4) + Stream (small)
        // With all 'a', entropy is 0. Stream should be empty or very small.
        // Freq 'a' = 4096. Bound = (4096 << 4) << 8 = 16M.
        // State grows slowly? x_new = x. (if freq=scale).
        // So no renormalization needed. Stream len 0.
        // Total size ~ 520 bytes.

        assert!(compressed.len() < 530);
        let decompressed = ans.revert(compressed);
        assert_eq!(data, decompressed);
    }

    #[test]
    fn test_ans_random_ish() {
        let ans = AnsBijection;
        let mut data = Vec::new();
        for i in 0..1000 {
            data.push((i % 256) as u8);
        }
        let compressed = ans.apply(data.clone());
        let decompressed = ans.revert(compressed);
        assert_eq!(data, decompressed);
    }
}
