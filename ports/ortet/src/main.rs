// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! `ortet` — open one document in one window. See the crate docs in `lib.rs`.

#[cfg(not(target_arch = "wasm32"))]
use ortet::args::{self, Invocation};
#[cfg(not(target_arch = "wasm32"))]
use ortet::fetch::OrtetFetcher;
#[cfg(not(target_arch = "wasm32"))]
use ortet::shell;

#[cfg(not(target_arch = "wasm32"))]
fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("ortet: {error}");
            std::process::ExitCode::FAILURE
        },
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn run() -> Result<(), String> {
    let config = match args::parse(std::env::args().skip(1))? {
        Invocation::Help => {
            print!("{}", args::USAGE);
            return Ok(());
        },
        Invocation::Run(config) => *config,
    };

    // Both lanes, always: a local page may name a remote stylesheet or image,
    // and the scheme split — not the address ortet was started with — is what
    // decides where each request goes.
    let fetcher = OrtetFetcher::with_network()?;

    println!("ortet: address {}", config.address);
    let outcome = shell::run(config, fetcher)?;
    println!(
        "ortet: presented {} frame(s) at {}x{}",
        outcome.frames, outcome.size.0, outcome.size.1
    );
    // The address the run ended on, which differs from the one it started on
    // exactly when a link was followed. That is the whole of what a navigation
    // receipt has to show.
    println!("ortet: settled at {}", outcome.address);
    if let (Some(artifact), Some(digest)) = (&outcome.artifact, outcome.digest) {
        println!(
            "ortet: receipt {} digest 0x{digest:016x}",
            artifact.display()
        );
    }
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn main() {}
