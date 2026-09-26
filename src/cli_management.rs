use clap::Args;
use markframe::management::{Failure, Report};
use serde_json::{Value, json};
use std::io::{IsTerminal, Write};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Args)]
pub struct Connection {
    #[arg(long, default_value = "http://127.0.0.1:3741")]
    server: String,
    #[arg(long)]
    json: bool,
}
#[derive(Args)]
pub struct DeleteOptions {
    #[command(flatten)]
    connection: Connection,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    yes: bool,
}
pub fn duration(value: &str) -> std::result::Result<u64, String> {
    let split = value
        .len()
        .checked_sub(1)
        .ok_or("Use a positive integer followed by h, d or w")?;
    let (digits, unit) = value.split_at_checked(split).ok_or("Invalid duration")?;
    let multiplier = match unit {
        "h" => 3600,
        "d" => 86400,
        "w" => 604800,
        _ => return Err("Use h, d or w".into()),
    };
    if digits.is_empty() || !digits.bytes().all(|c| c.is_ascii_digit()) {
        return Err("Use a positive integer".into());
    }
    digits
        .parse::<u64>()
        .ok()
        .and_then(|n| n.checked_mul(multiplier))
        .filter(|n| *n > 0 && *n <= i64::MAX as u64)
        .ok_or("Duration is zero or too large".into())
}
fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()?)
}
async fn response(request: reqwest::RequestBuilder) -> Result<Value> {
    let response = request.send().await?;
    let status = response.status();
    let value: Value = response.json().await?;
    if !status.is_success() {
        return Err(format!("Request failed ({status}): {value}").into());
    }
    Ok(value)
}
pub async fn list(connection: Connection) -> Result<()> {
    let value = response(client()?.get(format!(
        "{}/api/images",
        connection.server.trim_end_matches('/')
    )))
    .await?;
    if connection.json {
        println!("{}", serde_json::to_string(&value)?);
    } else {
        println!("ID\tNAME\tCREATED\tANNOTATIONS\tBYTES");
        for image in value["images"].as_array().ok_or("Invalid image list")? {
            let name = image["name"]
                .as_str()
                .unwrap_or_else(|| image["filename"].as_str().unwrap_or(""));
            let name: String = name
                .chars()
                .map(|c| if c.is_control() { ' ' } else { c })
                .collect();
            println!(
                "{}\t{}\t{}\t{}\t{}",
                image["id"].as_str().unwrap_or(""),
                name,
                image["created_at"].as_str().unwrap_or(""),
                image["annotation_count"],
                image["size_bytes"]
            );
        }
    }
    Ok(())
}
async fn batch(client: &reqwest::Client, base: &str, prune: bool, body: &Value) -> Result<Report> {
    let request = if prune {
        client.post(format!("{base}/api/images/prune"))
    } else {
        client.delete(format!("{base}/api/images"))
    };
    Ok(serde_json::from_value(response(request.json(body)).await?)?)
}
async fn individuals(
    client: &reqwest::Client,
    base: &str,
    ids: &[String],
    dry_run: bool,
) -> Report {
    let mut report = Report {
        dry_run,
        ..Default::default()
    };
    for id in ids {
        let result = client
            .delete(format!("{base}/api/images/{id}?dry_run={dry_run}"))
            .send()
            .await;
        match result {
            Ok(res) if res.status() == reqwest::StatusCode::NOT_FOUND => {
                report.missing_ids.push(id.clone())
            }
            Ok(res) => {
                let status = res.status();
                match res.json::<Value>().await {
                    Ok(value) if status.is_success() => {
                        match serde_json::from_value::<Report>(value) {
                            Ok(part) => {
                                report.target_ids.extend(part.target_ids);
                                report.target_bytes += part.target_bytes;
                                report.deleted_ids.extend(part.deleted_ids);
                                report.freed_bytes += part.freed_bytes;
                                report.failures.extend(part.failures);
                            }
                            Err(e) => report.failures.push(Failure {
                                id: id.clone(),
                                message: e.to_string(),
                            }),
                        }
                    }
                    Ok(value) => report.failures.push(Failure {
                        id: id.clone(),
                        message: format!("{status}: {value}"),
                    }),
                    Err(e) => report.failures.push(Failure {
                        id: id.clone(),
                        message: e.to_string(),
                    }),
                }
            }
            Err(e) => report.failures.push(Failure {
                id: id.clone(),
                message: e.to_string(),
            }),
        }
    }
    report
}
fn output(report: &Report, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string(report)?);
    } else {
        println!(
            "{}: {} images, {} bytes",
            if report.dry_run {
                "Would delete"
            } else {
                "Selected"
            },
            report.target_ids.len(),
            report.target_bytes
        );
        for id in &report.target_ids {
            println!("{id}");
        }
        println!(
            "Deleted: {}; protected: {}; missing: {}; freed: {} bytes",
            report.deleted_ids.len(),
            report.protected_ids.len(),
            report.missing_ids.len(),
            report.freed_bytes
        );
        for failure in &report.failures {
            eprintln!("{}: {}", failure.id, failure.message);
        }
    }
    Ok(())
}
pub async fn delete(
    options: DeleteOptions,
    mut ids: Option<Vec<String>>,
    mut body: Value,
    prune: bool,
) -> Result<()> {
    if let Some(ids) = &mut ids {
        for id in ids.iter() {
            uuid::Uuid::parse_str(id)?;
        }
        let mut seen = std::collections::HashSet::new();
        ids.retain(|id| seen.insert(id.clone()));
    }
    let client = client()?;
    let base = options.connection.server.trim_end_matches('/');
    body["dry_run"] = json!(true);
    let preview = if let Some(ids) = &ids {
        individuals(&client, base, ids, true).await
    } else {
        batch(&client, base, prune, &body).await?
    };
    let preview_failed =
        !preview.failures.is_empty() || (ids.is_some() && !preview.missing_ids.is_empty());
    if options.dry_run || preview.target_ids.is_empty() {
        let mut report = preview;
        if !options.dry_run && ids.is_none() && options.yes {
            body["dry_run"] = json!(false);
            body["candidate_ids"] = json!([]);
            let executed = batch(&client, base, prune, &body).await?;
            report.dry_run = false;
            report.freed_bytes = executed.freed_bytes;
            report.failures.extend(executed.failures);
        }
        report.dry_run = options.dry_run;
        let failed = preview_failed || !report.failures.is_empty();
        output(&report, options.connection.json)?;
        return if failed {
            Err("Some images could not be processed".into())
        } else {
            Ok(())
        };
    }
    if !options.yes {
        if !std::io::stdin().is_terminal() {
            return Err("Non-interactive deletion requires --yes (or use --dry-run)".into());
        }
        eprintln!(
            "Delete {} images ({} bytes), including their annotations?",
            preview.target_ids.len(),
            preview.target_bytes
        );
        for id in &preview.target_ids {
            eprintln!("  {id}");
        }
        eprint!("Type yes to continue: ");
        std::io::stderr().flush()?;
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        if answer.trim() != "yes" {
            return Err("Deletion cancelled".into());
        }
    }
    let mut report = if ids.is_some() {
        individuals(&client, base, &preview.target_ids, false).await
    } else {
        body["dry_run"] = json!(false);
        body["candidate_ids"] = json!(preview.target_ids);
        batch(&client, base, prune, &body).await?
    };
    report.missing_ids.extend(preview.missing_ids);
    report.failures.extend(preview.failures);
    for id in preview.protected_ids {
        if !report.protected_ids.contains(&id) {
            report.protected_ids.push(id);
        }
    }
    let failed = !report.failures.is_empty() || (ids.is_some() && !report.missing_ids.is_empty());
    output(&report, options.connection.json)?;
    if failed {
        return Err("Some images could not be processed".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::duration;
    #[test]
    fn durations() {
        assert_eq!(duration("7d").unwrap(), 604800);
        assert_eq!(duration("2h").unwrap(), 7200);
        assert_eq!(duration("1w").unwrap(), 604800);
        for value in [
            "",
            "0d",
            "-1d",
            "+1d",
            "1.5d",
            "d",
            "1m",
            "999999999999999999999w",
            "é",
        ] {
            assert!(duration(value).is_err(), "{value}");
        }
    }
}
