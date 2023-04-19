// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Copyright 2023 Oxide Computer Company

use anyhow::Result;
use clap::Parser;
use oxide_api::types::NameOrId;
use oxide_api::ClientInstancesExt;
use std::path::PathBuf;

#[derive(Parser, Debug, Clone)]
#[clap(verbatim_doc_comment)]
#[clap(name = "instance")]
pub enum CmdInstance {
    Serial(CmdInstanceSerial),
}

impl CmdInstance {
    pub async fn run(&self, ctx: &mut crate::context::Context) -> Result<()> {
        match &self {
            Self::Serial(cmd) => cmd.run(ctx).await,
        }
    }
}

/// Connect to or retrieve data from the instance's serial console.
#[derive(Parser, Debug, Clone)]
#[clap(verbatim_doc_comment)]
#[clap(name = "serial")]
pub struct CmdInstanceSerial {
    #[clap(subcommand)]
    subcmd: SubCommand,
}

impl CmdInstanceSerial {
    pub async fn run(&self, ctx: &mut crate::context::Context) -> Result<()> {
        match &self.subcmd {
            SubCommand::Console(cmd) => cmd.run(ctx).await,
            SubCommand::History(cmd) => cmd.run(ctx).await,
        }
    }
}

#[derive(Parser, Debug, Clone)]
enum SubCommand {
    Console(CmdInstanceSerialConsole),
    History(CmdInstanceSerialHistory),
}

/// Connect to an instance's serial console interactively.
#[derive(Parser, Debug, Clone)]
#[clap(verbatim_doc_comment)]
#[clap(name = "console")]
pub struct CmdInstanceSerialConsole {
    /// Name or ID of the instance
    #[clap(long, short)]
    instance: NameOrId,

    /// Name or ID of the project
    #[clap(long, short)]
    project: Option<NameOrId>,

    /// The offset since boot (or if negative, the current end of the
    /// buffered data) from which to retrieve output history.
    #[clap(long, short)]
    byte_offset: Option<i64>,

    /// If this sequence of bytes is typed, the client will exit.
    /// Defaults to "^]^C" (Ctrl+], Ctrl+C). Note that the string passed
    /// for this argument must be valid UTF-8, and is used verbatim without
    /// any parsing; in most shells, if you wish to include a special
    /// character (such as Enter or a Ctrl+letter combo), you can insert
    /// the character by preceding it with Ctrl+V at the command line.
    /// To disable the escape string altogether, provide an empty string to
    /// this flag (and to exit in such a case, use pkill or similar).
    #[clap(long, short, default_value = "\x1d\x03")]
    escape_string: String,

    /// The number of bytes from the beginning of the escape string to pass
    /// to the VM before beginning to buffer inputs until a mismatch.
    /// Defaults to 0, such that input matching the escape string does not
    /// get sent to the VM at all until a non-matching character is typed.
    /// For example, to mimic the escape sequence for exiting SSH ("\n~."),
    /// you may pass `-e '^M~.' --escape-prefix-length=1` such that newline
    /// gets sent to the VM immediately while still continuing to match the
    /// rest of the sequence.
    #[clap(long, default_value = "0")]
    escape_prefix_length: usize,

    /// Use a specified tty device (e.g. /dev/ttyUSB0) rather than the current
    /// terminal's stdin/stdout.
    #[clap(long, short)]
    tty: Option<PathBuf>,
}

impl CmdInstanceSerialConsole {
    // cli process becomes an interactive remote shell.
    pub async fn run(&self, ctx: &mut crate::context::Context) -> Result<()> {
        let mut req = ctx
            .client()
            .instance_serial_console_stream()
            .instance(self.instance.clone());

        if let Some(value) = &self.project {
            req = req.project(value.clone());
        }

        match self.byte_offset {
            Some(x) if x >= 0 => req = req.from_start(x as u64),
            Some(x) => req = req.most_recent(-x as u64),
            None => {}
        }

        let upgraded = req.send().await.map_err(|e| e.into_untyped())?.into_inner();

        let esc_bytes = self.escape_string.clone().into_bytes();
        let escape = if esc_bytes.is_empty() {
            None
        } else {
            Some(thouart::EscapeSequence::new(
                esc_bytes,
                self.escape_prefix_length,
            )?)
        };
        if let Some(path) = &self.tty {
            let f_in = tokio::fs::File::open(path).await?;
            let f_out = tokio::fs::OpenOptions::new().write(true).open(path).await?;
            let mut tty = thouart::Console::new(f_in, f_out, escape).await?;
            tty.attach_to_websocket(upgraded).await?;
        } else {
            let mut tty = thouart::Console::new_stdio(escape).await?;
            tty.attach_to_websocket(upgraded).await?;
        }
        Ok(())
    }
}

/// Fetch an instance's serial console output.
#[derive(Parser, Debug, Clone)]
#[clap(verbatim_doc_comment)]
#[clap(name = "history")]
pub struct CmdInstanceSerialHistory {
    /// Name or ID of the instance
    #[clap(long, short)]
    instance: NameOrId,

    /// Name or ID of the project
    #[clap(long, short)]
    project: Option<NameOrId>,

    /// The offset since boot (or if negative, the current end of the
    /// buffered data) from which to retrieve output history.
    /// Defaults to the instance's first output from boot.
    #[clap(long, short, default_value = "0")]
    byte_offset: Option<i64>,

    /// Maximum number of bytes of buffered serial console contents to return.
    /// If the requested range (starting at --byte-offset) runs to the end of
    /// the available buffer, the data returned will be shorter (and if --json
    /// is provided, the actual final offset will be provided).
    #[clap(long, short)]
    max_bytes: Option<u64>,

    /// Output a JSON payload of the requested bytes, and the absolute
    /// byte-offset-since-boot of the last byte retrieved, rather than
    /// formatting the output to the terminal directly.
    #[clap(long, short)]
    json: bool,
}

impl CmdInstanceSerialHistory {
    // cli process becomes an interactive remote shell.
    pub async fn run(&self, ctx: &mut crate::context::Context) -> Result<()> {
        let mut req = ctx
            .client()
            .instance_serial_console()
            .instance(self.instance.clone());

        if let Some(value) = self.max_bytes {
            req = req.max_bytes(value);
        }

        if let Some(value) = &self.project {
            req = req.project(value.clone());
        }

        match self.byte_offset {
            Some(x) if x >= 0 => req = req.from_start(x as u64),
            Some(x) => req = req.most_recent(-x as u64),
            None => {}
        }

        let data = req.send().await.map_err(|e| e.into_untyped())?.into_inner();

        if self.json {
            println!("{}", serde_json::to_string(&data)?);
        } else {
            let stdin = tokio::io::stdin();
            let stdout = tokio::io::stdout();
            let mut tty = thouart::Console::new(stdin, stdout, None).await?;
            tty.write_stdout(&data.data).await?;
        }
        Ok(())
    }
}