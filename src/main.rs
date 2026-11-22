// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

use clap::Parser;
use std::process::ExitCode;
use zorvia::cli::Cli;

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match zorvia::run(cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            // Wrap anyhow's own error-chain formatting (which already
            // includes every `.context(...)` frame) as a miette diagnostic
            // message, purely for miette's colored/boxed rendering --
            // `run()` and every handler underneath it keep returning plain
            // `anyhow::Result`, only this final print site changes.
            eprintln!("{:?}", miette::Report::msg(format!("{err:?}")));
            ExitCode::FAILURE
        }
    }
}
