use crate::codec::EventCodec;
use crate::{EventKey, EventValue};
use ans::{Ans2048Bijection, AnsBijection};
use bijection::ZStdBijection as GeneralCompression;
use bijection::{
    Bijection, EliasFanoBijection, MonotonicPermutationBijection, PositiveDeltaBijection,
    U24Bijection, U64DeltaBijection, VIntBijection,
};
use bytes::Bytes;
use columnar::{ColumnarEvents, EventsToColumns, ParseBijection, ParsedEvent};
use std::borrow::Cow;
use std::collections::HashMap;
use std::error::Error;

mod algo;
mod ans;
mod bijection;
mod columnar;
mod timestamp;

pub struct FulmicotonCodec;

impl FulmicotonCodec {
    pub fn new() -> Self {
        Self
    }
}

impl EventCodec for FulmicotonCodec {
    fn name(&self) -> &str {
        "fulmicoton"
    }

    fn encode(&self, events: &[(EventKey, EventValue)]) -> Result<Bytes, Box<dyn Error>> {
        let mut events = events.to_vec();
        events.sort_by(|a, b| a.0.cmp(&b.0));

        let parser = ParseBijection;
        let parsed_events: Vec<ParsedEvent> = events.iter().map(|e| parser.apply(e)).collect();

        let transformer = EventsToColumns;
        let cols = transformer.apply(Cow::Owned(parsed_events));

        let general_compression = GeneralCompression;
        let vint = VIntBijection;
        let pos_delta = PositiveDeltaBijection;
        let u64_delta = U64DeltaBijection;
        let ans = AnsBijection;
        let perm = MonotonicPermutationBijection;

        // Compress columns
        let c_event_ids = general_compression.apply(vint.apply(pos_delta.apply(cols.event_ids)));
        let c_event_type_indices = ans.apply(cols.event_type_indices);
        let c_dict_event_types =
            general_compression.apply(cols.dict_event_types.join("\n").into_bytes());

        // Timestamps: MonotonicPermutationBijection (internal Histogram + AnsU64)
        let c_created_ats = perm.apply(cols.created_ats);

        // Hybrid encoding for repo indices: top 2047 via ANS, rest via U24+Zstd
        let c_repo_indices = encode_repo_indices_hybrid(&cols.repo_indices);
        // Elias-Fano + Zstd encoding for sorted repo IDs
        let elias_fano = EliasFanoBijection;
        let c_dict_repo_ids = general_compression.apply(elias_fano.apply(cols.dict_repo_ids));

        // Combine repo owners and suffixes
        let joined_owners = cols.dict_repo_owners.join("\n");
        let joined_suffixes = cols.dict_repo_suffixes.join("\n");
        let mut combined_names = Vec::new();
        combined_names.extend_from_slice(joined_owners.as_bytes());
        combined_names.push(0); // Separator
        combined_names.extend_from_slice(joined_suffixes.as_bytes());
        let general_compression = GeneralCompression;
        let c_dict_repo_names = general_compression.apply(combined_names);

        println!("Compressed Component Sizes:");
        println!("  event_ids:          {} bytes", c_event_ids.len());
        println!("  event_type_indices: {} bytes", c_event_type_indices.len());
        println!("  dict_event_types:   {} bytes", c_dict_event_types.len());
        println!("  created_ats:        {} bytes", c_created_ats.len());
        println!("  repo_indices:       {} bytes", c_repo_indices.len());
        println!("  dict_repo_ids:      {} bytes", c_dict_repo_ids.len());
        println!("  dict_repo_names:    {} bytes", c_dict_repo_names.len());

        // Concatenate with VInt lengths
        let mut final_buf = Vec::new();
        let parts: Vec<Vec<u8>> = vec![
            c_event_ids,
            c_event_type_indices,
            c_dict_event_types,
            c_created_ats,
            c_repo_indices,
            c_dict_repo_ids,
            c_dict_repo_names,
        ];

        for part in parts {
            write_vint(part.len(), &mut final_buf);
            final_buf.extend_from_slice(&part);
        }

        Ok(Bytes::from(final_buf))
    }

    fn decode(&self, bytes: &[u8]) -> Result<Vec<(EventKey, EventValue)>, Box<dyn Error>> {
        let mut offset = 0;
        let mut read_part = || {
            let len = read_vint(bytes, &mut offset);
            let part = &bytes[offset..offset + len];
            offset += len;
            part.to_vec()
        };

        let c_event_ids = read_part();
        let c_event_type_indices = read_part();
        let c_dict_event_types = read_part();
        let c_created_ats = read_part();
        let c_repo_indices = read_part();
        let c_dict_repo_ids = read_part();
        let c_dict_repo_names = read_part();

        let general_compression = GeneralCompression;
        let vint = VIntBijection;
        let pos_delta = PositiveDeltaBijection;
        let u64_delta = U64DeltaBijection;
        let ans = AnsBijection;
        let perm = MonotonicPermutationBijection;

        // Decode dictionary names (owners + suffixes)
        let names_bytes = general_compression.revert(c_dict_repo_names);
        let names_vec = names_bytes;
        // Find separator
        let sep_pos = names_vec
            .iter()
            .position(|&b| b == 0)
            .expect("Missing separator in repo names");

        let owners_bytes = &names_vec[..sep_pos];
        let suffixes_bytes = &names_vec[sep_pos + 1..];

        let owners_str = String::from_utf8(owners_bytes.to_vec())?;
        let dict_repo_owners: Vec<String> = if owners_str.is_empty() {
            Vec::new()
        } else {
            owners_str.split('\n').map(|s| s.to_string()).collect()
        };

        let suffixes_str = String::from_utf8(suffixes_bytes.to_vec())?;
        let dict_repo_suffixes: Vec<String> = if suffixes_str.is_empty() {
            if dict_repo_owners.is_empty() {
                Vec::new()
            } else {
                vec!["".to_string(); dict_repo_owners.len()]
            }
        } else {
            suffixes_str.split('\n').map(|s| s.to_string()).collect()
        };

        // Decode dictionary event types
        let et_names_bytes = general_compression.revert(c_dict_event_types);
        let et_names_str = String::from_utf8(et_names_bytes)?;
        let dict_event_types: Vec<String> = if et_names_str.is_empty() {
            Vec::new()
        } else {
            et_names_str.split('\n').map(|s| s.to_string()).collect()
        };

        let cols = ColumnarEvents {
            event_ids: pos_delta.revert(vint.revert(general_compression.revert(c_event_ids))),

            event_type_indices: ans.revert(c_event_type_indices),
            dict_event_types,

            // Timestamps: MonotonicPermutation Revert
            created_ats: perm.revert(c_created_ats),

            // Hybrid decoding for repo indices
            repo_indices: decode_repo_indices_hybrid(c_repo_indices),

            // Elias-Fano + Zstd decoding for sorted repo IDs
            dict_repo_ids: EliasFanoBijection.revert(general_compression.revert(c_dict_repo_ids)),
            dict_repo_owners,
            dict_repo_suffixes,
        };

        let transformer = EventsToColumns;
        let parsed_events = transformer.revert(cols);

        let parser = ParseBijection;
        let mut events: Vec<(EventKey, EventValue)> =
            parsed_events.iter().map(|e| parser.revert(e)).collect();

        // Sort back to EventKey order (ID, Type) to satisfy main.rs check
        events.sort_by(|a, b| a.0.cmp(&b.0));

        Ok(events)
    }
}

const OTHER_SYMBOL: u16 = 2047;

fn encode_repo_indices_hybrid(repo_indices: &[u64]) -> Vec<u8> {
    // Count frequencies
    let mut freq_map: HashMap<u64, usize> = HashMap::new();
    for &idx in repo_indices {
        *freq_map.entry(idx).or_insert(0) += 1;
    }

    // Sort by frequency descending, take top 2047
    let mut freq_vec: Vec<(u64, usize)> = freq_map.into_iter().collect();
    freq_vec.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    let top_count = freq_vec.len().min(2047);
    let top_repos: Vec<u64> = freq_vec
        .iter()
        .take(top_count)
        .map(|(idx, _)| *idx)
        .collect();

    // Create mapping: original_index -> ANS symbol
    let mut index_to_symbol: HashMap<u64, u16> = HashMap::new();
    for (symbol, &original_idx) in top_repos.iter().enumerate() {
        index_to_symbol.insert(original_idx, symbol as u16);
    }

    // Encode symbols and collect "other" indices
    let mut symbols: Vec<u16> = Vec::with_capacity(repo_indices.len());
    let mut other_indices: Vec<u64> = Vec::new();

    for &idx in repo_indices {
        if let Some(&symbol) = index_to_symbol.get(&idx) {
            symbols.push(symbol);
        } else {
            symbols.push(OTHER_SYMBOL);
            other_indices.push(idx);
        }
    }

    // Encode using ANS
    let ans = Ans2048Bijection;
    let c_symbols = ans.apply(symbols);

    // Encode "other" indices using columnar U24 (ANS on high byte, zstd on low/mid)
    let zstd = GeneralCompression;
    let other_count = other_indices.len();
    let c_other = encode_u24_columnar(&other_indices);

    // Encode the mapping table (top 2047 repo indices) using VInt + Zstd
    let vint = VIntBijection;
    let c_mapping = zstd.apply(vint.apply(top_repos));

    println!("    repo_indices hybrid breakdown:");
    println!(
        "      mapping table: {} bytes ({} top repos)",
        c_mapping.len(),
        top_count
    );
    println!("      ANS symbols:   {} bytes", c_symbols.len());
    println!(
        "      other indices: {} bytes ({} items)",
        c_other.len(),
        other_count
    );

    // Combine: [len_mapping][mapping][len_symbols][symbols][other]
    let mut result = Vec::new();
    write_vint(c_mapping.len(), &mut result);
    result.extend(c_mapping);
    write_vint(c_symbols.len(), &mut result);
    result.extend(c_symbols);
    write_vint(c_other.len(), &mut result);
    result.extend(c_other);

    result
}

fn decode_repo_indices_hybrid(data: Vec<u8>) -> Vec<u64> {
    let mut offset = 0;

    let mapping_len = read_vint(&data, &mut offset);
    let c_mapping = data[offset..offset + mapping_len].to_vec();
    offset += mapping_len;

    let symbols_len = read_vint(&data, &mut offset);
    let c_symbols = data[offset..offset + symbols_len].to_vec();
    offset += symbols_len;

    let other_len = read_vint(&data, &mut offset);
    let c_other = data[offset..offset + other_len].to_vec();

    // Decode mapping table
    let vint = VIntBijection;
    let zstd = GeneralCompression;
    let top_repos = vint.revert(zstd.revert(c_mapping));

    // Decode ANS symbols
    let ans = Ans2048Bijection;
    let symbols = ans.revert(c_symbols);

    // Decode "other" indices using columnar U24
    let other_indices = decode_u24_columnar(&c_other);

    // Reconstruct repo indices
    let mut result = Vec::with_capacity(symbols.len());
    let mut other_iter = other_indices.into_iter();

    for symbol in symbols {
        if symbol == OTHER_SYMBOL {
            result.push(other_iter.next().expect("Missing 'other' index"));
        } else {
            result.push(top_repos[symbol as usize]);
        }
    }

    result
}

fn encode_u24_columnar(indices: &[u64]) -> Vec<u8> {
    if indices.is_empty() {
        return vec![];
    }

    // Split: high byte separate, low two bytes interleaved row-oriented
    let mut low_bytes = Vec::with_capacity(indices.len() * 2);
    let mut high_bytes = Vec::with_capacity(indices.len());

    for &idx in indices {
        // Row-oriented: low byte then mid byte for each index
        low_bytes.push(idx as u8);
        low_bytes.push((idx >> 8) as u8);
        // High byte separate
        high_bytes.push((idx >> 16) as u8);
    }

    // Compress low bytes with zstd, high bytes with ANS
    let zstd = GeneralCompression;
    let c_low = zstd.apply(low_bytes);
    let ans = AnsBijection;
    let c_high = ans.apply(high_bytes);

    // Combine with lengths
    let mut result = Vec::new();
    write_vint(c_low.len(), &mut result);
    result.extend(c_low);
    write_vint(c_high.len(), &mut result);
    result.extend(c_high);

    result
}

fn decode_u24_columnar(data: &[u8]) -> Vec<u64> {
    if data.is_empty() {
        return vec![];
    }

    let mut offset = 0;

    let len_low = read_vint(data, &mut offset);
    let c_low = &data[offset..offset + len_low];
    offset += len_low;

    let len_high = read_vint(data, &mut offset);
    let c_high = &data[offset..offset + len_high];

    // Decompress low bytes with zstd, high bytes with ANS
    let zstd = GeneralCompression;
    let low_bytes = zstd.revert(c_low.to_vec());
    let ans = AnsBijection;
    let high_bytes = ans.revert(c_high.to_vec());

    // Reconstruct indices from row-oriented low bytes and separate high bytes
    let count = high_bytes.len();
    let mut indices = Vec::with_capacity(count);
    for i in 0..count {
        let low = low_bytes[i * 2] as u64;
        let mid = low_bytes[i * 2 + 1] as u64;
        let high = high_bytes[i] as u64;
        let idx = low | (mid << 8) | (high << 16);
        indices.push(idx);
    }

    indices
}

fn write_vint(mut n: usize, buf: &mut Vec<u8>) {
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

fn read_vint(bytes: &[u8], offset: &mut usize) -> usize {
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
