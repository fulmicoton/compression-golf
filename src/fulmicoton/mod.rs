use crate::codec::EventCodec;
use crate::{EventKey, EventValue};
use ans::{AnsGenericBijection, AnsBijection};
mod adaptive_mix;
use bijection::ZStdBijection as GeneralCompression;
use bijection::{
    AdaptiveMixBijection, AdaptiveMixU64Bijection, BicBijection, Bijection,
    MonotonicPermutationBijection, PositiveDeltaBijection, U64DeltaBijection, VIntBijection,
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
        let ans = AnsBijection;
        let perm = MonotonicPermutationBijection;

        // Output event IDs to JSON
        let event_ids_json = serde_json::to_string(&cols.event_ids).unwrap();
        std::fs::write("event_ids.json", event_ids_json).unwrap();

        // Compress columns
        // Event IDs: first delta as VInt, rest as AdaptiveMix
        let adaptive_mix = AdaptiveMixU64Bijection;
        let deltas = pos_delta.apply(cols.event_ids);
        let mut c_event_ids = Vec::new();
        if !deltas.is_empty() {
            write_vint(deltas[0] as usize, &mut c_event_ids);
            c_event_ids.extend(adaptive_mix.apply(deltas[1..].to_vec()));
        }
        let c_event_type_indices = ans.apply(cols.event_type_indices);
        let c_dict_event_types =
            general_compression.apply(cols.dict_event_types.join("\n").into_bytes());

        // Timestamps: MonotonicPermutationBijection (internal Histogram + AnsU64)
        let c_created_ats = perm.apply(cols.created_ats);

        // Hybrid encoding for repo indices: top 2047 via ANS, rest via U24+Zstd
        let c_repo_indices = encode_repo_indices_hybrid(&cols.repo_indices);
        // BIC encoding for sorted repo IDs
        // Separate unique IDs from duplicate indices (duplicates occur from repo renames)
        let bic = BicBijection;
        let mut unique_ids: Vec<u64> = Vec::new();
        let mut dup_indices: Vec<u32> = Vec::new();
        for (i, &id) in cols.dict_repo_ids.iter().enumerate() {
            if i > 0 && id == cols.dict_repo_ids[i - 1] {
                dup_indices.push(i as u32);
            } else {
                unique_ids.push(id);
            }
        }
        let bic_encoded = bic.apply(unique_ids);
        // Encode duplicate indices as delta + vint
        let dup_deltas: Vec<u64> = if dup_indices.is_empty() {
            Vec::new()
        } else {
            let mut deltas = vec![dup_indices[0] as u64];
            for i in 1..dup_indices.len() {
                deltas.push((dup_indices[i] - dup_indices[i-1]) as u64);
            }
            deltas
        };
        let dup_encoded = vint.apply(dup_deltas);
        // Combine: [num_dups: u32][dup_encoded_len][dup_encoded][bic_encoded]
        let mut c_dict_repo_ids = Vec::new();
        c_dict_repo_ids.extend_from_slice(&(dup_indices.len() as u32).to_le_bytes());
        write_vint(dup_encoded.len(), &mut c_dict_repo_ids);
        c_dict_repo_ids.extend(dup_encoded);
        c_dict_repo_ids.extend(bic_encoded);

        // Hybrid encoding for repo suffixes: top 1023 via ANS, rest stored with owners
        let c_dict_repo_names = encode_repo_names_hybrid(&cols.dict_repo_owners, &cols.dict_repo_suffixes);

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

        // Decode dictionary names (owners + suffixes with hybrid encoding)
        let (dict_repo_owners, dict_repo_suffixes) = decode_repo_names_hybrid(&c_dict_repo_names);

        // Decode dictionary event types
        let et_names_bytes = general_compression.revert(c_dict_event_types);
        let et_names_str = String::from_utf8(et_names_bytes)?;
        let dict_event_types: Vec<String> = if et_names_str.is_empty() {
            Vec::new()
        } else {
            et_names_str.split('\n').map(|s| s.to_string()).collect()
        };

        // Decode event IDs: first delta as VInt, rest as AdaptiveMix
        let adaptive_mix = AdaptiveMixU64Bijection;
        let mut delta_offset = 0;
        let first_delta = read_vint(&c_event_ids, &mut delta_offset) as u64;
        let rest_deltas = adaptive_mix.revert(c_event_ids[delta_offset..].to_vec());
        let mut deltas = vec![first_delta];
        deltas.extend(rest_deltas);
        let event_ids = pos_delta.revert(deltas);

        let cols = ColumnarEvents {
            event_ids,

            event_type_indices: ans.revert(c_event_type_indices),
            dict_event_types,

            // Timestamps: MonotonicPermutation Revert
            created_ats: perm.revert(c_created_ats),

            // Hybrid decoding for repo indices
            repo_indices: decode_repo_indices_hybrid(c_repo_indices),

            // BIC decoding for sorted repo IDs
            dict_repo_ids: {
                let mut offset = 0;
                let num_dups = u32::from_le_bytes([
                    c_dict_repo_ids[0], c_dict_repo_ids[1],
                    c_dict_repo_ids[2], c_dict_repo_ids[3]
                ]) as usize;
                offset += 4;
                let dup_len = read_vint(&c_dict_repo_ids, &mut offset);
                let dup_encoded = &c_dict_repo_ids[offset..offset + dup_len];
                offset += dup_len;
                let bic_encoded = c_dict_repo_ids[offset..].to_vec();

                // Decode duplicate indices from deltas
                let dup_deltas = vint.revert(dup_encoded.to_vec());
                let mut dup_indices: Vec<usize> = Vec::with_capacity(num_dups);
                let mut pos = 0u64;
                for delta in dup_deltas {
                    pos += delta;
                    dup_indices.push(pos as usize);
                }

                // Decode unique IDs
                let unique_ids = BicBijection.revert(bic_encoded);

                // Reconstruct full list by inserting duplicates
                let total_len = unique_ids.len() + dup_indices.len();
                let mut result = Vec::with_capacity(total_len);
                let mut unique_iter = unique_ids.into_iter();
                let mut dup_set: std::collections::HashSet<usize> = dup_indices.into_iter().collect();

                for i in 0..total_len {
                    if dup_set.contains(&i) {
                        // Duplicate: copy previous value
                        result.push(*result.last().unwrap());
                    } else {
                        result.push(unique_iter.next().unwrap());
                    }
                }
                result
            },
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
    let ans = AnsGenericBijection::<2048>;
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
    let ans = AnsGenericBijection::<2048>;
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

// Configuration for hybrid name encoding
const TOP_OWNERS_COUNT: usize = 1023;
const OTHER_OWNER_SYMBOL: u16 = 1023;
const TOP_SUFFIXES_COUNT: usize = 1023;
const OTHER_SUFFIX_SYMBOL: u16 = 1023;

fn encode_repo_names_hybrid(owners: &[String], suffixes: &[String]) -> Vec<u8> {
    if suffixes.is_empty() {
        return vec![];
    }

    // === Encode owners with hybrid approach ===
    let mut owner_freq_map: HashMap<&str, usize> = HashMap::new();
    for s in owners {
        *owner_freq_map.entry(s.as_str()).or_insert(0) += 1;
    }

    let mut owner_freq_vec: Vec<(&str, usize)> = owner_freq_map.into_iter().collect();
    owner_freq_vec.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    let top_owner_count = owner_freq_vec.len().min(TOP_OWNERS_COUNT);
    let top_owners: Vec<&str> = owner_freq_vec.iter().take(top_owner_count).map(|(s, _)| *s).collect();

    let mut owner_to_symbol: HashMap<&str, u16> = HashMap::new();
    for (symbol, &owner) in top_owners.iter().enumerate() {
        owner_to_symbol.insert(owner, symbol as u16);
    }

    let mut owner_symbols: Vec<u16> = Vec::with_capacity(owners.len());
    let mut other_owners: Vec<&str> = Vec::new();

    for s in owners {
        if let Some(&symbol) = owner_to_symbol.get(s.as_str()) {
            owner_symbols.push(symbol);
        } else {
            owner_symbols.push(OTHER_OWNER_SYMBOL);
            other_owners.push(s.as_str());
        }
    }

    let c_owner_symbols = AnsGenericBijection::<1024>.apply(owner_symbols);

    // === Encode suffixes with hybrid approach ===
    let mut suffix_freq_map: HashMap<&str, usize> = HashMap::new();
    for s in suffixes {
        *suffix_freq_map.entry(s.as_str()).or_insert(0) += 1;
    }

    let mut suffix_freq_vec: Vec<(&str, usize)> = suffix_freq_map.into_iter().collect();
    suffix_freq_vec.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    let top_suffix_count = suffix_freq_vec.len().min(TOP_SUFFIXES_COUNT);
    let top_suffixes: Vec<&str> = suffix_freq_vec.iter().take(top_suffix_count).map(|(s, _)| *s).collect();

    let mut suffix_to_symbol: HashMap<&str, u16> = HashMap::new();
    for (symbol, &suffix) in top_suffixes.iter().enumerate() {
        suffix_to_symbol.insert(suffix, symbol as u16);
    }

    let mut suffix_symbols: Vec<u16> = Vec::with_capacity(suffixes.len());
    let mut other_suffixes: Vec<&str> = Vec::new();

    for s in suffixes {
        if let Some(&symbol) = suffix_to_symbol.get(s.as_str()) {
            suffix_symbols.push(symbol);
        } else {
            suffix_symbols.push(OTHER_SUFFIX_SYMBOL);
            other_suffixes.push(s.as_str());
        }
    }

    let c_suffix_symbols = AnsGenericBijection::<1024>.apply(suffix_symbols);

    // Combine all strings into one blob for zstd
    // Format: top_owners\0other_owners\0top_suffixes\0other_suffixes
    let joined_top_owners = top_owners.join("\n");
    let joined_other_owners = other_owners.join("\n");
    let joined_top_suffixes = top_suffixes.join("\n");
    let joined_other_suffixes = other_suffixes.join("\n");

    let mut combined = Vec::new();
    combined.extend_from_slice(joined_top_owners.as_bytes());
    combined.push(0);
    combined.extend_from_slice(joined_other_owners.as_bytes());
    combined.push(0);
    combined.extend_from_slice(joined_top_suffixes.as_bytes());
    combined.push(0);
    combined.extend_from_slice(joined_other_suffixes.as_bytes());

    let zstd = GeneralCompression;
    let c_strings = zstd.apply(combined);

    println!("    names hybrid breakdown:");
    println!(
        "      owner ANS:     {} bytes ({} top, {} other)",
        c_owner_symbols.len(), top_owner_count, other_owners.len()
    );
    println!(
        "      suffix ANS:    {} bytes ({} top, {} other)",
        c_suffix_symbols.len(), top_suffix_count, other_suffixes.len()
    );
    println!("      strings blob:  {} bytes", c_strings.len());

    // Combine: [len_owner_symbols][owner_symbols][len_suffix_symbols][suffix_symbols][strings_blob]
    let mut result = Vec::new();
    write_vint(c_owner_symbols.len(), &mut result);
    result.extend(c_owner_symbols);
    write_vint(c_suffix_symbols.len(), &mut result);
    result.extend(c_suffix_symbols);
    result.extend(c_strings);

    result
}

fn decode_repo_names_hybrid(data: &[u8]) -> (Vec<String>, Vec<String>) {
    if data.is_empty() {
        return (vec![], vec![]);
    }

    let mut offset = 0;

    let owner_symbols_len = read_vint(data, &mut offset);
    let c_owner_symbols = data[offset..offset + owner_symbols_len].to_vec();
    offset += owner_symbols_len;

    let suffix_symbols_len = read_vint(data, &mut offset);
    let c_suffix_symbols = data[offset..offset + suffix_symbols_len].to_vec();
    offset += suffix_symbols_len;

    let c_strings = data[offset..].to_vec();

    // Decode ANS symbols with appropriate sizes
    let owner_symbols = AnsGenericBijection::<1024>.revert(c_owner_symbols);
    let suffix_symbols = AnsGenericBijection::<1024>.revert(c_suffix_symbols);

    // Decode combined strings blob
    let zstd = GeneralCompression;
    let strings_bytes = zstd.revert(c_strings);

    // Find separators (4 sections = 3 separators)
    let sep_positions: Vec<usize> = strings_bytes
        .iter()
        .enumerate()
        .filter(|(_, &b)| b == 0)
        .map(|(i, _)| i)
        .collect();

    let top_owners_bytes = &strings_bytes[..sep_positions[0]];
    let other_owners_bytes = &strings_bytes[sep_positions[0] + 1..sep_positions[1]];
    let top_suffixes_bytes = &strings_bytes[sep_positions[1] + 1..sep_positions[2]];
    let other_suffixes_bytes = &strings_bytes[sep_positions[2] + 1..];

    let parse_strings = |bytes: &[u8]| -> Vec<String> {
        let s = String::from_utf8(bytes.to_vec()).unwrap();
        if s.is_empty() {
            vec![]
        } else {
            s.split('\n').map(|x| x.to_string()).collect()
        }
    };

    let top_owners = parse_strings(top_owners_bytes);
    let other_owners = parse_strings(other_owners_bytes);
    let top_suffixes = parse_strings(top_suffixes_bytes);
    let other_suffixes = parse_strings(other_suffixes_bytes);

    // Reconstruct owners
    let mut dict_repo_owners = Vec::with_capacity(owner_symbols.len());
    let mut other_owner_iter = other_owners.into_iter();

    for symbol in owner_symbols {
        if symbol == OTHER_OWNER_SYMBOL {
            dict_repo_owners.push(other_owner_iter.next().expect("Missing 'other' owner"));
        } else {
            dict_repo_owners.push(top_owners[symbol as usize].clone());
        }
    }

    // Reconstruct suffixes
    let mut dict_repo_suffixes = Vec::with_capacity(suffix_symbols.len());
    let mut other_suffix_iter = other_suffixes.into_iter();

    for symbol in suffix_symbols {
        if symbol == OTHER_SUFFIX_SYMBOL {
            dict_repo_suffixes.push(other_suffix_iter.next().expect("Missing 'other' suffix"));
        } else {
            dict_repo_suffixes.push(top_suffixes[symbol as usize].clone());
        }
    }

    (dict_repo_owners, dict_repo_suffixes)
}
