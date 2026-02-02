use std::collections::HashMap;

/// Order-0 frequency model with Laplace smoothing
pub struct Order0Model {
    counts: [u32; 256],
    total: u32,
    num_symbols: u32,
}

impl Order0Model {
    pub fn new(num_symbols: u32) -> Self {
        Self {
            counts: [0; 256],
            total: 0,
            num_symbols,
        }
    }

    pub fn probability(&self, symbol: u8) -> f64 {
        // Laplace smoothing: (count + 1) / (total + num_symbols)
        (self.counts[symbol as usize] as f64 + 1.0) / (self.total as f64 + self.num_symbols as f64)
    }

    pub fn update(&mut self, symbol: u8) {
        self.counts[symbol as usize] += 1;
        self.total += 1;
    }
}

/// Order-1 context model
pub struct Order1Model {
    contexts: HashMap<u8, Order0Model>,
    num_symbols: u32,
}

impl Order1Model {
    pub fn new(num_symbols: u32) -> Self {
        Self {
            contexts: HashMap::new(),
            num_symbols,
        }
    }

    pub fn probability(&self, context: u8, symbol: u8) -> f64 {
        match self.contexts.get(&context) {
            Some(model) if model.total > 0 => model.probability(symbol),
            _ => 1.0 / self.num_symbols as f64, // Uniform fallback
        }
    }

    pub fn update(&mut self, context: u8, symbol: u8) {
        self.contexts
            .entry(context)
            .or_insert_with(|| Order0Model::new(self.num_symbols))
            .update(symbol);
    }
}

/// Adaptive mixer combining order-0 and order-1 models
pub struct AdaptiveMixer {
    order0: Order0Model,
    order1: Order1Model,
    prev_symbol: u8,
    context_quantize: u8,
}

impl AdaptiveMixer {
    pub fn new(num_symbols: u32, context_quantize: u8) -> Self {
        Self {
            order0: Order0Model::new(num_symbols),
            order1: Order1Model::new(num_symbols),
            prev_symbol: 0,
            context_quantize,
        }
    }

    fn quantized_context(&self) -> u8 {
        self.prev_symbol.min(self.context_quantize - 1)
    }

    /// Get mixed probability for a symbol
    pub fn probability(&self, symbol: u8) -> f64 {
        let p0 = self.order0.probability(symbol);
        let p1 = self.order1.probability(self.quantized_context(), symbol);
        // Simple average mixing
        (p0 + p1) / 2.0
    }

    /// Update models after encoding/decoding a symbol
    pub fn update(&mut self, symbol: u8) {
        let ctx = self.quantized_context();
        self.order0.update(symbol);
        self.order1.update(ctx, symbol);
        self.prev_symbol = symbol;
    }

    /// Calculate bits needed to encode a symbol (for analysis)
    pub fn bits_for_symbol(&self, symbol: u8) -> f64 {
        -self.probability(symbol).log2()
    }
}

/// Simple range/arithmetic encoder
pub struct RangeEncoder {
    low: u64,
    high: u64,
    pending_bits: u32,
    output: Vec<u8>,
}

impl RangeEncoder {
    pub fn new() -> Self {
        Self {
            low: 0,
            high: 0xFFFFFFFF,
            pending_bits: 0,
            output: Vec::new(),
        }
    }

    fn output_bit(&mut self, bit: bool) {
        if bit {
            self.output.push(1);
        } else {
            self.output.push(0);
        }
        while self.pending_bits > 0 {
            self.output.push(if bit { 0 } else { 1 });
            self.pending_bits -= 1;
        }
    }

    /// Encode a symbol with given cumulative frequency and frequency
    pub fn encode(&mut self, cum_freq: u32, freq: u32, total: u32) {
        let range = self.high - self.low + 1;
        self.high = self.low + (range * (cum_freq + freq) as u64) / total as u64 - 1;
        self.low = self.low + (range * cum_freq as u64) / total as u64;

        loop {
            if self.high < 0x80000000 {
                self.output_bit(false);
                self.low <<= 1;
                self.high = (self.high << 1) | 1;
            } else if self.low >= 0x80000000 {
                self.output_bit(true);
                self.low = (self.low - 0x80000000) << 1;
                self.high = ((self.high - 0x80000000) << 1) | 1;
            } else if self.low >= 0x40000000 && self.high < 0xC0000000 {
                self.pending_bits += 1;
                self.low = (self.low - 0x40000000) << 1;
                self.high = ((self.high - 0x40000000) << 1) | 1;
            } else {
                break;
            }
        }
    }

    pub fn finish(mut self) -> Vec<u8> {
        self.pending_bits += 1;
        self.output_bit(self.low >= 0x40000000);

        // Convert bit stream to bytes
        let mut bytes = Vec::new();
        for chunk in self.output.chunks(8) {
            let mut byte = 0u8;
            for (i, &bit) in chunk.iter().enumerate() {
                byte |= bit << (7 - i);
            }
            bytes.push(byte);
        }
        // Store bit count in first 4 bytes
        let bit_count = self.output.len() as u32;
        let mut result = bit_count.to_le_bytes().to_vec();
        result.extend(bytes);
        result
    }
}

/// Simple range/arithmetic decoder
pub struct RangeDecoder {
    low: u64,
    high: u64,
    value: u64,
    bits: Vec<u8>,
    bit_pos: usize,
}

impl RangeDecoder {
    pub fn new(input: &[u8]) -> Self {
        if input.len() < 4 {
            return Self {
                low: 0,
                high: 0xFFFFFFFF,
                value: 0,
                bits: Vec::new(),
                bit_pos: 0,
            };
        }

        let bit_count = u32::from_le_bytes([input[0], input[1], input[2], input[3]]) as usize;

        // Convert bytes to bits
        let mut bits = Vec::with_capacity(bit_count);
        for &byte in &input[4..] {
            for i in (0..8).rev() {
                bits.push((byte >> i) & 1);
                if bits.len() >= bit_count {
                    break;
                }
            }
            if bits.len() >= bit_count {
                break;
            }
        }

        let mut decoder = Self {
            low: 0,
            high: 0xFFFFFFFF,
            value: 0,
            bits,
            bit_pos: 0,
        };

        // Initialize value from first 32 bits
        for _ in 0..32 {
            decoder.value = (decoder.value << 1) | decoder.read_bit() as u64;
        }

        decoder
    }

    fn read_bit(&mut self) -> u8 {
        if self.bit_pos < self.bits.len() {
            let bit = self.bits[self.bit_pos];
            self.bit_pos += 1;
            bit
        } else {
            0
        }
    }

    /// Get the current frequency count for decoding
    pub fn get_freq(&self, total: u32) -> u32 {
        let range = self.high - self.low + 1;
        (((self.value - self.low + 1) * total as u64 - 1) / range) as u32
    }

    /// Update decoder state after identifying symbol
    pub fn decode(&mut self, cum_freq: u32, freq: u32, total: u32) {
        let range = self.high - self.low + 1;
        self.high = self.low + (range * (cum_freq + freq) as u64) / total as u64 - 1;
        self.low = self.low + (range * cum_freq as u64) / total as u64;

        loop {
            if self.high < 0x80000000 {
                self.low <<= 1;
                self.high = (self.high << 1) | 1;
                self.value = (self.value << 1) | self.read_bit() as u64;
            } else if self.low >= 0x80000000 {
                self.low = (self.low - 0x80000000) << 1;
                self.high = ((self.high - 0x80000000) << 1) | 1;
                self.value = ((self.value - 0x80000000) << 1) | self.read_bit() as u64;
            } else if self.low >= 0x40000000 && self.high < 0xC0000000 {
                self.low = (self.low - 0x40000000) << 1;
                self.high = ((self.high - 0x40000000) << 1) | 1;
                self.value = ((self.value - 0x40000000) << 1) | self.read_bit() as u64;
            } else {
                break;
            }
        }
    }
}

/// Frequency table for encoding/decoding
pub struct FrequencyTable {
    freqs: [u32; 256],
    cum_freqs: [u32; 257],
    total: u32,
}

impl FrequencyTable {
    const SCALE: u32 = 1 << 14;

    pub fn new() -> Self {
        let mut table = Self {
            freqs: [1; 256], // Start with count of 1 for smoothing
            cum_freqs: [0; 257],
            total: 256,
        };
        // Initialize cumulative frequencies
        for i in 0..256 {
            table.cum_freqs[i + 1] = table.cum_freqs[i] + table.freqs[i];
        }
        table
    }

    pub fn update(&mut self, symbol: u8) {
        self.freqs[symbol as usize] += 1;
        self.total += 1;

        // Rebuild cumulative frequencies
        self.cum_freqs[0] = 0;
        for i in 0..256 {
            self.cum_freqs[i + 1] = self.cum_freqs[i] + self.freqs[i];
        }

        // Rescale if total gets too large
        if self.total > Self::SCALE {
            self.rescale();
        }
    }

    fn rescale(&mut self) {
        self.total = 0;
        for f in &mut self.freqs {
            *f = (*f + 1) / 2; // Halve but keep at least 1
            self.total += *f;
        }
        self.cum_freqs[0] = 0;
        for i in 0..256 {
            self.cum_freqs[i + 1] = self.cum_freqs[i] + self.freqs[i];
        }
    }

    pub fn get_freq(&self, symbol: u8) -> u32 {
        self.freqs[symbol as usize]
    }

    pub fn get_cum_freq(&self, symbol: u8) -> u32 {
        self.cum_freqs[symbol as usize]
    }

    pub fn get_total(&self) -> u32 {
        self.total
    }

    /// Find symbol from cumulative frequency (for decoding)
    pub fn find_symbol(&self, cum: u32) -> u8 {
        // Binary search
        let mut lo = 0usize;
        let mut hi = 256usize;
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.cum_freqs[mid + 1] <= cum {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        lo as u8
    }
}

/// Adaptive arithmetic coder with order-0 model
pub struct AdaptiveMixCoder {
    freqs: FrequencyTable,
}

impl AdaptiveMixCoder {
    pub fn new() -> Self {
        Self {
            freqs: FrequencyTable::new(),
        }
    }

    /// Encode a sequence of bytes
    pub fn encode(&mut self, data: &[u8]) -> Vec<u8> {
        let mut encoder = RangeEncoder::new();

        for &symbol in data {
            let cum_freq = self.freqs.get_cum_freq(symbol);
            let freq = self.freqs.get_freq(symbol);
            let total = self.freqs.get_total();

            encoder.encode(cum_freq, freq, total);
            self.freqs.update(symbol);
        }

        encoder.finish()
    }

    /// Decode a sequence of bytes
    pub fn decode(&mut self, encoded: &[u8], len: usize) -> Vec<u8> {
        let mut decoder = RangeDecoder::new(encoded);
        let mut result = Vec::with_capacity(len);

        for _ in 0..len {
            let total = self.freqs.get_total();

            // Get frequency value from decoder
            let freq_val = decoder.get_freq(total).min(total - 1);

            // Find symbol
            let symbol = self.freqs.find_symbol(freq_val);

            // Update decoder state
            let cum_freq = self.freqs.get_cum_freq(symbol);
            let freq = self.freqs.get_freq(symbol);
            decoder.decode(cum_freq, freq, total);

            // Update model
            self.freqs.update(symbol);

            result.push(symbol);
        }

        result
    }

    /// Calculate theoretical bits (for analysis without actual encoding)
    pub fn theoretical_bits(&mut self, data: &[u8]) -> f64 {
        let mut total_bits = 0.0;

        for &symbol in data {
            let f = self.freqs.get_freq(symbol);
            let t = self.freqs.get_total();
            let p = f as f64 / t as f64;

            total_bits += -p.log2();
            self.freqs.update(symbol);
        }

        total_bits
    }
}

impl Default for AdaptiveMixCoder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_order0_model() {
        let mut model = Order0Model::new(256);

        // Initially uniform with smoothing
        let p = model.probability(0);
        assert!((p - 1.0 / 256.0).abs() < 0.01);

        // After updating, probability should increase
        model.update(42);
        let p_new = model.probability(42);
        assert!(p_new > 1.0 / 256.0);
    }

    #[test]
    fn test_adaptive_mixer() {
        let mut mixer = AdaptiveMixer::new(256, 16);

        // Encode some data
        let data = vec![1, 2, 1, 2, 1, 2, 1, 2];
        let mut total_bits = 0.0;

        for &symbol in &data {
            total_bits += mixer.bits_for_symbol(symbol);
            mixer.update(symbol);
        }

        // With alternating pattern, should learn and compress well
        assert!(total_bits < data.len() as f64 * 8.0);
    }

    #[test]
    fn test_round_trip_small() {
        // Test with small data
        let data = vec![1, 2, 3, 1, 2, 3, 1, 2, 3, 4, 5, 6];

        let mut encoder = AdaptiveMixCoder::new();
        let encoded = encoder.encode(&data);

        let mut decoder = AdaptiveMixCoder::new();
        let decoded = decoder.decode(&encoded, data.len());

        assert_eq!(data, decoded);
    }

    #[test]
    fn test_round_trip_repeated() {
        // Test with repeated pattern
        let data: Vec<u8> = (0..1000).map(|i| (i % 10) as u8).collect();

        let mut encoder = AdaptiveMixCoder::new();
        let encoded = encoder.encode(&data);

        let mut decoder = AdaptiveMixCoder::new();
        let decoded = decoder.decode(&encoded, data.len());

        assert_eq!(data, decoded);

        // Should compress well
        assert!(encoded.len() < data.len());
    }

    #[test]
    fn test_round_trip_random_like() {
        // Test with pseudo-random data (deterministic for reproducibility)
        let mut data = Vec::with_capacity(1000);
        let mut x: u32 = 12345;
        for _ in 0..1000 {
            x = x.wrapping_mul(1103515245).wrapping_add(12345);
            data.push(((x >> 16) & 0xFF) as u8);
        }

        let mut encoder = AdaptiveMixCoder::new();
        let encoded = encoder.encode(&data);

        let mut decoder = AdaptiveMixCoder::new();
        let decoded = decoder.decode(&encoded, data.len());

        assert_eq!(data, decoded);
    }

    #[test]
    fn test_compression_ratio() {
        // Test that compressible data actually compresses
        let data: Vec<u8> = vec![1; 1000]; // All ones

        let mut encoder = AdaptiveMixCoder::new();
        let encoded = encoder.encode(&data);

        // Highly compressible data should compress significantly
        assert!(
            encoded.len() < data.len() / 2,
            "Expected significant compression, got {} -> {} bytes",
            data.len(),
            encoded.len()
        );

        // Verify round-trip
        let mut decoder = AdaptiveMixCoder::new();
        let decoded = decoder.decode(&encoded, data.len());
        assert_eq!(data, decoded);
    }

    #[test]
    fn test_frequency_table() {
        let mut table = FrequencyTable::new();

        // Update with some symbols
        for _ in 0..100 {
            table.update(5);
        }

        // Symbol 5 should have higher frequency
        assert!(table.get_freq(5) > table.get_freq(0));

        // Find symbol should work
        let cum = table.get_cum_freq(5);
        let found = table.find_symbol(cum);
        assert_eq!(found, 5);
    }
}
