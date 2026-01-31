use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use crate::{EventKey, EventValue, Repo};
use serde::{Deserialize, Serialize};
use super::bijection::Bijection;
use chrono::{DateTime, TimeZone, Utc};

#[derive(Serialize, Deserialize, Default)]
pub struct ColumnarEvents {
    // Event specific
    pub event_ids: Vec<i64>,
    pub event_type_indices: Vec<u8>,
    pub dict_event_types: Vec<String>,
    pub created_ats: Vec<i64>,
    
    // Repo specific (Dictionary Encoded)
    pub repo_indices: Vec<u64>,
    pub dict_repo_ids: Vec<u64>,
    pub dict_repo_owners: Vec<String>,
    pub dict_repo_suffixes: Vec<String>,
}

pub struct EventsToColumns;

impl<'a> Bijection<Cow<'a, [(EventKey, EventValue)]>, ColumnarEvents> for EventsToColumns {
    fn apply(&self, events: Cow<'a, [(EventKey, EventValue)]>) -> ColumnarEvents {
        let events = events.as_ref();
        let mut cols = ColumnarEvents::default();
        
        // Pre-allocate
        cols.event_ids.reserve(events.len());
        cols.event_type_indices.reserve(events.len());
        cols.created_ats.reserve(events.len());
        cols.repo_indices.reserve(events.len());

        // 1. Build Repo Dictionary
        let mut unique_repos: HashSet<(u64, String)> = HashSet::new();
        for (_, val) in events {
            unique_repos.insert((val.repo.id, val.repo.name.clone()));
        }

        let mut sorted_repos: Vec<(u64, String)> = unique_repos.into_iter().collect();
        sorted_repos.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));

        let mut repo_to_index = HashMap::new();
        for (i, (id, name)) in sorted_repos.into_iter().enumerate() {
            repo_to_index.insert((id, name.clone()), i as u64);
            cols.dict_repo_ids.push(id);
            
            let parts: Vec<&str> = name.splitn(2, '/').collect();
            if parts.len() == 2 {
                cols.dict_repo_owners.push(parts[0].to_string());
                cols.dict_repo_suffixes.push(parts[1].to_string());
            } else {
                cols.dict_repo_owners.push(name.clone());
                cols.dict_repo_suffixes.push("".to_string());
            }
        }

        // 2. Build Event Type Dictionary
        let mut unique_event_types: HashSet<String> = HashSet::new();
        for (key, _) in events {
            unique_event_types.insert(key.event_type.clone());
        }
        let mut sorted_event_types: Vec<String> = unique_event_types.into_iter().collect();
        sorted_event_types.sort();
        
        let mut event_type_to_index = HashMap::new();
        for (i, et) in sorted_event_types.into_iter().enumerate() {
            if i > 255 { panic!("Too many event types for u8"); }
            event_type_to_index.insert(et.clone(), i as u8);
            cols.dict_event_types.push(et);
        }

        // 3. Encode Columns
        for (key, val) in events {
            cols.event_ids.push(key.id.parse::<i64>().expect("Failed to parse event id"));
            
            let et_idx = event_type_to_index.get(&key.event_type).expect("Event type not found");
            cols.event_type_indices.push(*et_idx);
            
            let dt = DateTime::parse_from_rfc3339(&val.created_at).expect("Failed to parse created_at");
            cols.created_ats.push(dt.timestamp());
            
            let idx = repo_to_index.get(&(val.repo.id, val.repo.name.clone()))
                .expect("Repo not found in dictionary");
            cols.repo_indices.push(*idx);
        }
        
        cols
    }

    fn revert(&self, cols: ColumnarEvents) -> Cow<'a, [(EventKey, EventValue)]> {
        let len = cols.event_ids.len();
        let mut events = Vec::with_capacity(len);

        for i in 0..len {
            let et_idx = cols.event_type_indices[i] as usize;
            let event_type = cols.dict_event_types[et_idx].clone();

            let key = EventKey {
                id: cols.event_ids[i].to_string(),
                event_type,
            };
            
            let dt = Utc.timestamp_opt(cols.created_ats[i], 0).unwrap();
            let created_at = dt.format("%Y-%m-%dT%H:%M:%SZ").to_string();

            let repo_idx = cols.repo_indices[i] as usize;
            let repo_id = cols.dict_repo_ids[repo_idx];
            let owner = &cols.dict_repo_owners[repo_idx];
            let suffix = &cols.dict_repo_suffixes[repo_idx];
            let repo_name = if suffix.is_empty() {
                owner.clone()
            } else {
                format!("{}/{}", owner, suffix)
            };

            let val = EventValue {
                repo: Repo {
                    id: repo_id,
                    name: repo_name.clone(),
                    url: format!("https://api.github.com/repos/{}", repo_name),
                },
                created_at,
            };
            events.push((key, val));
        }
        
        Cow::Owned(events)
    }
}