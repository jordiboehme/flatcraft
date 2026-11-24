//! Command line arguments.

use clap::{Parser, Subcommand};

/// FlatCraft drives cutting plotters. Without a command it opens the window.
#[derive(Debug, Parser)]
#[command(name = "flatcraft", version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum Command {
    /// Serve the MCP tools on stdin and stdout for an AI agent.
    Mcp {
        /// Also open the FlatCraft window, so you can watch the agent work
        /// and confirm USB cuts.
        #[arg(long)]
        window: bool,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Option<Command> {
        Cli::try_parse_from(args).unwrap().command
    }

    #[test]
    fn commands() {
        assert_eq!(parse(&["flatcraft"]), None);
        assert_eq!(
            parse(&["flatcraft", "mcp"]),
            Some(Command::Mcp { window: false })
        );
        assert_eq!(
            parse(&["flatcraft", "mcp", "--window"]),
            Some(Command::Mcp { window: true })
        );
        let e = Cli::try_parse_from(["flatcraft", "--version"]).unwrap_err();
        assert_eq!(e.kind(), clap::error::ErrorKind::DisplayVersion);
    }
}
