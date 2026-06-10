mod client;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "kaiju",
    about = "Kaiju - Unified control plane for AI coding agents",
    version
)]
struct Cli {
    /// Daemon API URL
    #[arg(long, env = "KAIJU_URL", default_value = "http://127.0.0.1:7800")]
    url: String,

    /// Bearer token for an authenticated daemon
    #[arg(long, env = "KAIJU_TOKEN")]
    token: Option<String>,

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

        /// Run the agent in its own git worktree (requires a git workspace)
        #[arg(long)]
        isolate: bool,

        /// Run non-interactively via the CLI's structured mode (precise metrics)
        #[arg(long)]
        batch: bool,
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

    /// Show the changes an agent has made
    Diff {
        /// Agent ID
        id: String,
    },

    /// Stop a running agent
    Stop {
        /// Agent ID
        id: String,
    },

    /// Resume a stopped or finished agent (continues its conversation)
    Resume {
        /// Agent ID
        id: String,
    },

    /// Send a follow-up message or approval to a running agent
    Send {
        /// Agent ID
        id: String,
        /// Message text to send
        message: String,
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

    /// Submit a task to the queue (the pool runs it when a slot frees)
    Submit {
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

        /// Run the task in its own git worktree
        #[arg(long)]
        isolate: bool,
    },

    /// List queued, running, and finished tasks
    Queue,

    /// Cancel a queued or running task
    Cancel {
        /// Task ID
        id: String,
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let client = client::NexusClient::new(&cli.url, cli.token);

    let result = match cli.command {
        Commands::Start {
            agent_type,
            workspace,
            model,
            prompt,
            isolate,
            batch,
        } => {
            let workspace = if workspace == "." {
                std::env::current_dir().unwrap().display().to_string()
            } else {
                workspace
            };
            client
                .start(&agent_type, &workspace, model, prompt, isolate, batch)
                .await
        }
        Commands::List { active } => client.list(active).await,
        Commands::Status { id } => client.status(&id).await,
        Commands::Logs { id } => client.logs(&id).await,
        Commands::Diff { id } => client.diff(&id).await,
        Commands::Stop { id } => client.stop(&id).await,
        Commands::Resume { id } => client.resume(&id).await,
        Commands::Send { id, message } => client.send(&id, &message).await,
        Commands::Interrupt { id } => client.interrupt(&id).await,
        Commands::Remove { id } => client.remove(&id).await,
        Commands::Attach { id } => client.attach(&id).await,
        Commands::Submit {
            agent_type,
            workspace,
            model,
            prompt,
            isolate,
        } => {
            let workspace = if workspace == "." {
                std::env::current_dir().unwrap().display().to_string()
            } else {
                workspace
            };
            client
                .submit(&agent_type, &workspace, model, prompt, isolate)
                .await
        }
        Commands::Queue => client.queue().await,
        Commands::Cancel { id } => client.cancel_task(&id).await,
    };

    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
