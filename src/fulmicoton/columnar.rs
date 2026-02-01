use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use crate::{EventKey, EventValue, Repo};
use serde::{Deserialize, Serialize};
use super::bijection::Bijection;
use chrono::{DateTime, TimeZone, Utc};

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedEvent {
    pub id: i64,
    pub event_type: String,
    pub repo_id: u64,
    pub repo_owner: String,
    pub repo_suffix: String,
    pub created_at: i64,
}

pub struct ParseBijection;

impl ParseBijection {
    pub fn apply(&self, source: &(EventKey, EventValue)) -> ParsedEvent {
        let (key, val) = source;
        
        let id = key.id.parse::<i64>().expect("Failed to parse event id");
        let created_at = DateTime::parse_from_rfc3339(&val.created_at)
            .expect("Failed to parse created_at")
            .timestamp();
            
        let parts: Vec<&str> = val.repo.name.splitn(2, '/').collect();
        let (repo_owner, repo_suffix) = if parts.len() == 2 {
            (parts[0].to_string(), parts[1].to_string())
        } else {
            (val.repo.name.clone(), "".to_string())
        };

        ParsedEvent {
            id,
            event_type: key.event_type.clone(),
            repo_id: val.repo.id,
            repo_owner,
            repo_suffix,
            created_at,
        }
    }

    pub fn revert(&self, source: &ParsedEvent) -> (EventKey, EventValue) {
        let key = EventKey {
            id: source.id.to_string(),
            event_type: source.event_type.clone(),
        };
        
        let dt = Utc.timestamp_opt(source.created_at, 0).unwrap();
        let created_at = dt.format("%Y-%m-%dT%H:%M:%SZ").to_string();

        let repo_name = if source.repo_suffix.is_empty() {
            source.repo_owner.clone()
        } else {
            format!("{}/{}", source.repo_owner, source.repo_suffix)
        };

        let val = EventValue {
            repo: Repo {
                id: source.repo_id,
                name: repo_name.clone(),
                url: format!("https://api.github.com/repos/{}", repo_name),
            },
            created_at,
        };
        
        (key, val)
    }
}

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

impl<'a> Bijection<Cow<'a, [ParsedEvent]>, ColumnarEvents> for EventsToColumns {
    fn apply(&self, events: Cow<'a, [ParsedEvent]>) -> ColumnarEvents {
        let events = events.as_ref();
        let mut cols = ColumnarEvents::default();
        
        // Pre-allocate
        cols.event_ids.reserve(events.len());
        cols.event_type_indices.reserve(events.len());
        cols.created_ats.reserve(events.len());
        cols.repo_indices.reserve(events.len());

        // 1. Build Repo Dictionary
        // We use (repo_id, repo_owner, repo_suffix) as key to handle renames perfectly?
        // Previously we used (id, name). Now name is split.
        // So key is (id, owner, suffix).
        let mut unique_repos: HashSet<(u64, String, String)> = HashSet::new();
        for event in events {
            unique_repos.insert((event.repo_id, event.repo_owner.clone(), event.repo_suffix.clone()));
        }

        let mut sorted_repos: Vec<(u64, String, String)> = unique_repos.into_iter().collect();
        // Sort by ID, then Owner, then Suffix
        sorted_repos.sort_by(|a, b| a.0.cmp(&b.0)
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.2.cmp(&b.2)));

        let mut repo_to_index = HashMap::new();
        for (i, (id, owner, suffix)) in sorted_repos.into_iter().enumerate() {
            repo_to_index.insert((id, owner.clone(), suffix.clone()), i as u64);
            cols.dict_repo_ids.push(id);
            cols.dict_repo_owners.push(owner);
            cols.dict_repo_suffixes.push(suffix);
        }

        // 2. Build Event Type Dictionary
        let mut unique_event_types: HashSet<String> = HashSet::new();
        for event in events {
            unique_event_types.insert(event.event_type.clone());
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
        for event in events {
            cols.event_ids.push(event.id);
            
            let et_idx = event_type_to_index.get(&event.event_type).expect("Event type not found");
            cols.event_type_indices.push(*et_idx);
            
            cols.created_ats.push(event.created_at);
            
            let idx = repo_to_index.get(&(event.repo_id, event.repo_owner.clone(), event.repo_suffix.clone()))
                .expect("Repo not found in dictionary");
            cols.repo_indices.push(*idx);
        }
        
        cols
    }

    fn revert(&self, cols: ColumnarEvents) -> Cow<'a, [ParsedEvent]> {
        let len = cols.event_ids.len();
        let mut events = Vec::with_capacity(len);

        for i in 0..len {
            let et_idx = cols.event_type_indices[i] as usize;
            let event_type = cols.dict_event_types[et_idx].clone();

            let repo_idx = cols.repo_indices[i] as usize;
            let repo_id = cols.dict_repo_ids[repo_idx];
            let repo_owner = cols.dict_repo_owners[repo_idx].clone();
            let repo_suffix = cols.dict_repo_suffixes[repo_idx].clone();

            events.push(ParsedEvent {
                id: cols.event_ids[i],
                event_type,
                repo_id,
                repo_owner,
                repo_suffix,
                created_at: cols.created_ats[i],
            });
        }
        
        Cow::Owned(events)
    }
}