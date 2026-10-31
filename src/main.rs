mod identity;
mod ssh_config;
mod transport;

use anyhow::Result;
use clap::{Parser, Subcommand};
use peeroxide::KeyPair;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "hyperssh", about = "SSH over HyperDHT, no IP address or port forwarding needed")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run on the machine you want to SSH into: accept tunnels and forward them to the local sshd
    #[command(alias = "roost")]
    Serve {
        /// Throwaway identity (a new id on every start)
        #[arg(long)]
        ephemeral: bool,
        /// Port sshd listens on
        #[arg(long, default_value_t = 22)]
        ssh_port: u16,
        /// Server identity file (default: <config dir>/hyperssh/roost.key)
        #[arg(long)]
        identity: Option<PathBuf>,
    },

    /// Connect to a server by id. Used as ssh's ProxyCommand via `connect --stdio`
    #[command(alias = "fly")]
    Connect {
        /// Server id (64 hex characters)
        id: String,
        /// Pipe the tunnel over stdin/stdout (ProxyCommand mode)
        #[arg(long)]
        stdio: bool,
        /// Optional persistent client identity (default: ephemeral)
        #[arg(long)]
        identity: Option<PathBuf>,
    },

    /// Add a `Host` entry to ~/.ssh/config
    Add {
        #[arg(long)]
        id: String,
        #[arg(long)]
        name: String,
    },

    /// List configured routes
    List,

    /// Remove a route from ~/.ssh/config
    Remove { name: String },

    /// Print this machine's server id (creating the key if needed)
    Id {
        #[arg(long)]
        identity: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    // stdout belongs to the tunnel in ProxyCommand mode; all logs go to stderr.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn".into()),
        )
        .init();

    match Cli::parse().command {
        Command::Serve { ephemeral, ssh_port, identity: path } => {
            let key_pair = if ephemeral {
                KeyPair::generate()
            } else {
                let path = match path {
                    Some(p) => p,
                    None => identity::default_roost_key_path()?,
                };
                let (kp, created) = identity::load_or_create(&path).await?;
                if created {
                    eprintln!("Created new identity at {}", path.display());
                }
                kp
            };
            transport::serve(key_pair, format!("127.0.0.1:{ssh_port}")).await
        }

        Command::Connect { id, stdio, identity: path } => {
            if !stdio {
                anyhow::bail!("Only `--stdio` mode is implemented so far");
            }
            let remote = identity::decode_public_key(&id)?;
            let key_pair = match path {
                Some(p) => identity::load(&p).await?,
                None => KeyPair::generate(),
            };
            transport::connect_stdio(key_pair, remote).await
        }

        Command::Add { id, name } => ssh_config::add(&name, &id).await,
        Command::List => ssh_config::list().await,
        Command::Remove { name } => ssh_config::remove(&name).await,

        Command::Id { identity: path } => {
            let path = match path {
                Some(p) => p,
                None => identity::default_roost_key_path()?,
            };
            let (kp, _) = identity::load_or_create(&path).await?;
            println!("{}", identity::public_key_hex(&kp));
            Ok(())
        }
    }
}