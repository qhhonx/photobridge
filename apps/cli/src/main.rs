use clap::{Parser, Subcommand};
use photobridge_core::*;
use photobridge_store::Receiver;
use photobridge_transport::Client;
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::Write,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

#[derive(Parser)]
#[command(version, about = "PhotoBridge reference sender and receiver")]
struct Args {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// Create a private authentication token file; its contents are never printed.
    InitToken { path: PathBuf },
    /// Run the reference HTTP receiver on loopback. Native/TLS hosts embed the router.
    Serve {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        token_file: PathBuf,
        #[arg(long, default_value = "127.0.0.1:8484")]
        listen: SocketAddr,
        #[arg(long, default_value_t = 32)]
        capacity_gib: u64,
    },
    /// Build a manifest from original files. Supply --paired-video for a motion asset.
    Manifest {
        #[arg(long)]
        source_id: String,
        #[arg(long, default_value = "1")]
        revision: String,
        #[arg(long)]
        file: PathBuf,
        #[arg(long)]
        paired_video: Option<PathBuf>,
        #[arg(long)]
        output: PathBuf,
    },
    /// Send a manifest. Resource filenames are resolved inside --files.
    Send {
        #[arg(long)]
        server: String,
        #[arg(long)]
        token_file: PathBuf,
        #[arg(long)]
        manifest: PathBuf,
        #[arg(long)]
        files: PathBuf,
    },
    Status {
        #[arg(long)]
        server: String,
        #[arg(long)]
        token_file: PathBuf,
        id: String,
    },
}
fn resource(path: &Path, role: ResourceRole) -> Result<Resource> {
    let filename = path
        .file_name()
        .and_then(|v| v.to_str())
        .ok_or_else(|| Error::Invalid("UTF-8 filename".into()))?
        .to_owned();
    let media_type = match path
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or("")
        .to_lowercase()
        .as_str()
    {
        "jpg" | "jpeg" => "image/jpeg",
        "heic" | "heif" => "image/heic",
        "png" => "image/png",
        "mov" => "video/quicktime",
        "mp4" => "video/mp4",
        _ => return Err(Error::Unsupported("file type".into())),
    }
    .to_owned();
    Ok(Resource {
        role,
        filename,
        media_type,
        size: path.metadata()?.len(),
        sha256: digest_reader(File::open(path)?)?,
    })
}
fn token(path: PathBuf) -> Result<String> {
    Ok(fs::read_to_string(path)?.trim().to_owned())
}
#[tokio::main]
async fn main() -> Result<()> {
    match Args::parse().command {
        Command::InitToken { path } => {
            let mut bytes = [0; 32];
            getrandom::fill(&mut bytes)
                .map_err(|_| Error::Storage("random source unavailable".into()))?;
            let mut options = OpenOptions::new();
            options.create_new(true).write(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(path)?;
            for byte in bytes {
                write!(file, "{byte:02x}")?;
            }
            file.write_all(b"\n")?;
            file.sync_all()?;
            println!("Token file created. Keep it private.");
        }
        Command::Serve {
            root,
            token_file,
            listen,
            capacity_gib,
        } => {
            if !listen.ip().is_loopback() {
                return Err(Error::Invalid(
                    "reference HTTP host only binds loopback; remote hosts require TLS".into(),
                ));
            }
            let capacity = capacity_gib
                .checked_mul(1024 * 1024 * 1024)
                .ok_or(Error::Capacity)?;
            let app = photobridge_transport::router(
                Receiver::open(root, capacity)?,
                &token(token_file)?,
            )?;
            let listener = tokio::net::TcpListener::bind(listen).await?;
            println!("Receiver listening on {}", listener.local_addr()?);
            axum_serve(listener, app).await?;
        }
        Command::Manifest {
            source_id,
            revision,
            file,
            paired_video,
            output,
        } => {
            let video = matches!(
                file.extension()
                    .and_then(|v| v.to_str())
                    .unwrap_or("")
                    .to_lowercase()
                    .as_str(),
                "mov" | "mp4"
            );
            if video && paired_video.is_some() {
                return Err(Error::Invalid("motion primary must be a photo".into()));
            }
            let kind = if paired_video.is_some() {
                AssetKind::Motion
            } else if video {
                AssetKind::Video
            } else {
                AssetKind::Photo
            };
            let mut resources = vec![resource(
                &file,
                if video {
                    ResourceRole::Video
                } else {
                    ResourceRole::Photo
                },
            )?];
            if let Some(path) = paired_video {
                let r = resource(&path, ResourceRole::PairedVideo)?;
                if !r.media_type.starts_with("video/") {
                    return Err(Error::Invalid("paired video".into()));
                }
                resources.push(r);
            }
            let asset = Asset {
                version: PROTOCOL_VERSION,
                source_id,
                revision,
                kind,
                metadata: BTreeMap::new(),
                resources,
            };
            let id = asset.id()?;
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(output)?;
            file.write_all(&serde_json::to_vec_pretty(&asset)?)?;
            file.sync_all()?;
            println!("Manifest created: {id}");
        }
        Command::Send {
            server,
            token_file,
            manifest,
            files,
        } => {
            let asset: Asset = serde_json::from_slice(&fs::read(manifest)?)?;
            asset.validate()?;
            let paths = asset
                .resources
                .iter()
                .map(|r| (r.sha256.clone(), files.join(&r.filename)))
                .collect::<BTreeMap<_, _>>();
            let client = Client::new(&server, &token(token_file)?)?;
            let result = client
                .send(&asset, &paths, &AtomicBool::new(false), |s| {
                    let bytes: u64 = s.resources.iter().map(|r| r.offset).sum();
                    eprintln!("{}: {} bytes received; {:?}", s.asset_id, bytes, s.receipt);
                })
                .await?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::Status {
            server,
            token_file,
            id,
        } => println!(
            "{}",
            serde_json::to_string_pretty(
                &Client::new(&server, &token(token_file)?)?
                    .status(&id)
                    .await?
            )?
        ),
    }
    Ok(())
}
async fn axum_serve(listener: tokio::net::TcpListener, router: axum::Router) -> Result<()> {
    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
