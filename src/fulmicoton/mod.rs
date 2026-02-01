use bijection::{Bijection, ZStdBijection, VIntBijection, U24Bijection, PositiveDeltaBijection, U64DeltaBijection};
use columnar::{ColumnarEvents, EventsToColumns, ParsedEvent, ParseBijection};
use ans::AnsBijection;
use timestamp::TimestampCodec;
use bytes::Bytes;
use std::borrow::Cow;
use std::error::Error;
use crate::codec::EventCodec;
use crate::{EventKey, EventValue};

mod bijection;
mod columnar;
mod ans;
mod timestamp;
mod algo;

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
        
        let zstd = ZStdBijection;
        let vint = VIntBijection;
        let u24 = U24Bijection;
        let pos_delta = PositiveDeltaBijection;
        let u64_delta = U64DeltaBijection;
        let ans = AnsBijection;
        let ts_codec = TimestampCodec;
        
        // Compress columns
        let c_event_ids = zstd.apply(vint.apply(pos_delta.apply(cols.event_ids)));
        let c_event_type_indices = ans.apply(cols.event_type_indices);
        let c_dict_event_types = zstd.apply(cols.dict_event_types.join("\n").into_bytes());
        
        // Timestamps: TimestampCodec (Permutation+Histogram+Ans) -> Zstd
        let c_created_ats = zstd.apply(ts_codec.apply(cols.created_ats));
        
        let c_repo_indices = zstd.apply(u24.apply(cols.repo_indices));
        let c_dict_repo_ids = zstd.apply(vint.apply(u64_delta.apply(cols.dict_repo_ids)));
        
        // Combine repo owners and suffixes
        let joined_owners = cols.dict_repo_owners.join("\n");
        let joined_suffixes = cols.dict_repo_suffixes.join("\n");
        let mut combined_names = Vec::new();
        combined_names.extend_from_slice(joined_owners.as_bytes());
        combined_names.push(0); // Separator
        combined_names.extend_from_slice(joined_suffixes.as_bytes());
        let c_dict_repo_names = zstd.apply(combined_names);

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
        let parts = vec![
            c_event_ids, c_event_type_indices, c_dict_event_types, c_created_ats,
            c_repo_indices, c_dict_repo_ids, c_dict_repo_names
        ];

        for part in parts {
            write_vint(part.len(), &mut final_buf);
            final_buf.extend_from_slice(&part);
        }

        Ok(Bytes::from(final_buf))
    }

    fn decode(&self, bytes: &[
u8]) -> Result<Vec<(EventKey, EventValue)>, Box<dyn Error>> {
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

        let zstd = ZStdBijection;
        let vint = VIntBijection;
        let u24 = U24Bijection;
        let pos_delta = PositiveDeltaBijection;
        let u64_delta = U64DeltaBijection;
        let ans = AnsBijection;
        let ts_codec = TimestampCodec;

        // Decode dictionary names (owners + suffixes)
        let names_bytes = zstd.revert(c_dict_repo_names);
        let names_vec = names_bytes; 
        // Find separator
        let sep_pos = names_vec.iter().position(|&b| b == 0).expect("Missing separator in repo names");
        
        let owners_bytes = &names_vec[..sep_pos];
        let suffixes_bytes = &names_vec[sep_pos+1..];

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
        let et_names_bytes = zstd.revert(c_dict_event_types);
        let et_names_str = String::from_utf8(et_names_bytes)?;
        let dict_event_types: Vec<String> = if et_names_str.is_empty() {
            Vec::new()
        } else {
            et_names_str.split('\n').map(|s| s.to_string()).collect()
        };

        let cols = ColumnarEvents {
            event_ids: pos_delta.revert(vint.revert(zstd.revert(c_event_ids))),
            
            event_type_indices: ans.revert(c_event_type_indices),
            dict_event_types,
            
            // Timestamps: Zstd -> TimestampCodec
            created_ats: ts_codec.revert(zstd.revert(c_created_ats)),
            
            repo_indices: u24.revert(zstd.revert(c_repo_indices)),
            
            dict_repo_ids: u64_delta.revert(vint.revert(zstd.revert(c_dict_repo_ids))),
            dict_repo_owners,
            dict_repo_suffixes,
        };

        let transformer = EventsToColumns;
        let parsed_events = transformer.revert(cols);
        
        let parser = ParseBijection;
        let mut events: Vec<(EventKey, EventValue)> = parsed_events.iter().map(|e| parser.revert(e)).collect();
        
        // Sort back to EventKey order (ID, Type) to satisfy main.rs check
        events.sort_by(|a, b| a.0.cmp(&b.0));
        
        Ok(events)
    }
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