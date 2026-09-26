use crate::storage::Storage;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub older_than_seconds: Option<u64>,
    pub keep_last: Option<usize>,
    #[serde(default)]
    pub include_annotated: bool,
    #[serde(default)]
    pub dry_run: bool,
    pub candidate_ids: Option<Vec<String>>,
    #[serde(default)]
    pub all: bool,
}
#[derive(Default, Serialize, Deserialize)]
pub struct Report {
    pub dry_run: bool,
    pub target_ids: Vec<String>,
    pub target_bytes: u64,
    pub deleted_ids: Vec<String>,
    pub protected_ids: Vec<String>,
    pub missing_ids: Vec<String>,
    pub freed_bytes: u64,
    pub failures: Vec<Failure>,
}
#[derive(Serialize, Deserialize)]
pub struct Failure {
    pub id: String,
    pub message: String,
}
impl Storage {
    pub fn image_size(&self, id: &str) -> std::io::Result<u64> {
        match std::fs::metadata(self.root.join("images").join(id)) {
            Ok(m) => Ok(m.len()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(0),
            Err(e) => Err(e),
        }
    }
    pub fn cleanup(&mut self) -> Report {
        let mut result = Report::default();
        let mut next = self.data.clone();
        next.pending_deletions.retain(|id| {
            // Never remove a live image or accept a path from damaged state.
            if uuid::Uuid::parse_str(id).is_err() || self.data.images.iter().any(|m| &m.id == id) {
                result.failures.push(Failure {
                    id: id.clone(),
                    message: "Invalid deletion entry".into(),
                });
                return true;
            }
            let size = self.image_size(id).unwrap_or(0);
            match std::fs::remove_file(self.root.join("images").join(id)) {
                Ok(()) => {
                    result.freed_bytes += size;
                    false
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
                Err(e) => {
                    result.failures.push(Failure {
                        id: id.clone(),
                        message: e.to_string(),
                    });
                    true
                }
            }
        });
        if next.pending_deletions != self.data.pending_deletions
            && let Err(e) = self.commit(next)
        {
            result.failures.push(Failure {
                id: String::new(),
                message: e.to_string(),
            });
        }
        result
    }
    pub fn select_and_delete(
        &mut self,
        options: &Selection,
        now: DateTime<Utc>,
    ) -> std::io::Result<Report> {
        let mut report = Report {
            dry_run: options.dry_run,
            ..Report::default()
        };
        let candidates = options
            .candidate_ids
            .as_ref()
            .map(|ids| ids.iter().collect::<HashSet<_>>());
        if let Some(ids) = &options.candidate_ids {
            for id in ids {
                if !self.data.images.iter().any(|m| &m.id == id) && !report.missing_ids.contains(id)
                {
                    report.missing_ids.push(id.clone());
                }
            }
        }
        for (index, image) in self.data.images.iter().enumerate() {
            if candidates
                .as_ref()
                .is_some_and(|ids| !ids.contains(&image.id))
            {
                continue;
            }
            if !options.all {
                if options.keep_last.is_some_and(|n| index < n) {
                    continue;
                }
                if let Some(seconds) = options.older_than_seconds {
                    let created = DateTime::parse_from_rfc3339(&image.created_at)
                        .map_err(std::io::Error::other)?;
                    let age = now.signed_duration_since(created);
                    if age.num_seconds() < seconds as i64
                        || (age.num_seconds() == seconds as i64 && age.subsec_nanos() <= 0)
                    {
                        continue;
                    }
                }
                if !options.include_annotated
                    && self
                        .data
                        .annotations
                        .get(&image.id)
                        .is_some_and(|a| !a.is_empty())
                {
                    report.protected_ids.push(image.id.clone());
                    continue;
                }
            }
            report.target_bytes += self.image_size(&image.id)?;
            report.target_ids.push(image.id.clone());
        }
        if options.dry_run {
            return Ok(report);
        }
        if !report.target_ids.is_empty() {
            let mut next = self.data.clone();
            next.images.retain(|m| !report.target_ids.contains(&m.id));
            for id in &report.target_ids {
                next.annotations.remove(id);
                if !next.pending_deletions.contains(id) {
                    next.pending_deletions.push(id.clone());
                }
            }
            self.commit(next)?;
            report.deleted_ids = report.target_ids.clone();
        }
        let cleanup = self.cleanup();
        report.freed_bytes = cleanup.freed_bytes;
        report.failures = cleanup.failures;
        Ok(report)
    }
}
