mod cli_management;
use clap::{Parser, Subcommand};
use markframe::{
    api::{AppState, IMAGE_LIMIT, router},
    storage::Storage,
};
use std::path::PathBuf;
#[derive(Parser)]
#[command(
    name = "markframe",
    version,
    about = "Local visual review for coding agents"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// List posted images, annotation counts and sizes.
    List {
        #[command(flatten)]
        connection: cli_management::Connection,
    },
    /// Delete selected images and their annotations.
    Delete {
        #[arg(required = true, num_args = 1..)]
        ids: Vec<String>,
        #[command(flatten)]
        options: cli_management::DeleteOptions,
    },
    /// Delete all posted images and annotations.
    Clear {
        #[command(flatten)]
        options: cli_management::DeleteOptions,
    },
    /// Remove older images, preserving annotated images by default.
    Prune {
        #[arg(long, value_parser = cli_management::duration, required_unless_present = "keep_last")]
        older_than: Option<u64>,
        #[arg(long)]
        keep_last: Option<usize>,
        #[arg(long)]
        include_annotated: bool,
        #[command(flatten)]
        options: cli_management::DeleteOptions,
    },
    Serve {
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(long, default_value_t = 3741)]
        port: u16,
    },
    Post {
        image: PathBuf,
        #[arg(long, default_value = "http://127.0.0.1:3741")]
        server: String,
    },
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "markframe=info".into()),
        )
        .init();
    match Cli::parse().command {
        Command::List { connection } => cli_management::list(connection).await?,
        Command::Delete { ids, options } => cli_management::delete(options, Some(ids), serde_json::json!({}), false).await?,
        Command::Clear { options } => cli_management::delete(options, None, serde_json::json!({"all":true}), false).await?,
        Command::Prune { older_than, keep_last, include_annotated, options } => cli_management::delete(options, None, serde_json::json!({"older_than_seconds":older_than,"keep_last":keep_last,"include_annotated":include_annotated}), true).await?,
        Command::Serve { host, port } => {
            let root = std::env::var_os("MARKFRAME_DATA_DIR")
                .map(PathBuf::from)
                .or_else(|| {
                    directories::ProjectDirs::from("dev", "markframe", "Markframe")
                        .map(|d| d.data_local_dir().to_owned())
                })
                .ok_or("Cannot determine data directory; set MARKFRAME_DATA_DIR")?;
            let state = AppState::new(Storage::open(root)?);
            let listener = tokio::net::TcpListener::bind((host.as_str(), port)).await?;
            tracing::info!("Markframe listening on http://{}", listener.local_addr()?);
            axum::serve(listener, router(state))
                .with_graceful_shutdown(async {
                    let _ = tokio::signal::ctrl_c().await;
                })
                .await?;
        }
        Command::Post { image, server } => {
            if tokio::fs::metadata(&image).await?.len() > IMAGE_LIMIT as u64 {
                return Err("Image exceeds 25 MiB".into());
            }
            let filename = image
                .file_name()
                .ok_or("Missing filename")?
                .to_string_lossy()
                .into_owned();
            let part =
                reqwest::multipart::Part::bytes(tokio::fs::read(&image).await?).file_name(filename);
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(60))
                .build()?;
            let response = client
                .post(format!("{}/api/images", server.trim_end_matches('/')))
                .multipart(reqwest::multipart::Form::new().part("image", part))
                .send()
                .await?;
            let status = response.status();
            let text = response.text().await?;
            if !status.is_success() {
                return Err(format!("Upload failed ({status}): {text}").into());
            }
            println!("{text}");
        }
    }
    Ok(())
}
