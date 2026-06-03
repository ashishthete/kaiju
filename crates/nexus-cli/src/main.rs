mod client;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "agentnexus",
    about = "AgentNexus - Unified control plane for AI coding agents",
    version
)]
struct Cli {
    /// Daemon API URL
    #[arg(long, env = "NEXUS_URL", default_value = "http://127.0.0.1:7800")]
    url: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Spawn a new agent
    Start {
        /// Agent type: claude, codex, gemini
        #[arg(short = 't', long)]
        agent_type: String,

        /// Working directory for the agent
        #[arg(short, long, default_value = ".")]
        workspace: String,

        /// Model to use (e.g. sonnet, o3, pro)
        #[arg(short, long)]
        model: Option<String>,

        /// Prompt/task for the agent
        #[arg(short, long)]
        prompt: Option<String>,
    },

    /// List all agents
    List {
        /// Show only active agents
        #[arg(long)]
        active: bool,
    },

    /// Get agent status
    Status {
        /// Agent ID
        id: String,
    },

    /// Get agent logs
    Logs {
        /// Agent ID
        id: String,
    },

    /// Stop a running agent
    Stop {
        /// Agent ID
        id: String,
    },

    /// Send interrupt (Ctrl-C) to an agent
    Interrupt {
        /// Agent ID
        id: String,
    },

    /// Remove an agent (stops if running)
    Remove {
        /// Agent ID
        id: String,
    },

    /// Attach to an agent's tmux session
    Attach {
        /// Agent ID
        id: String,
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let client = client::NexusClient::new(&cli.url);

    let result = match cli.command {
        Commands::Start {
            agent_type,
            workspace,
            model,
            prompt,
        } => {
            let workspace = if workspace == "." {
                std::env::current_dir()
                    .unwrap()
                    .display()
                    .to_string()
            } else {
                workspace
            };
            client.start(&agent_type, &workspace, model, prompt).await
        }
        Commands::List { active } => client.list(active).await,
        Commands::Status { id } => client.status(&id).await,
        Commands::Logs { id } => client.logs(&id).await,
        Commands::Stop { id } => client.stop(&id).await,
        Commands::Interrupt { id } => client.interrupt(&id).await,
        Commands::Remove { id } => client.remove(&id).await,
        Commands::Attach { id } => client.attach(&id).await,
    };

    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
