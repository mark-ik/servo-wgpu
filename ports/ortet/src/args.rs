// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Ortet's command line.
//!
//! Six options and no configuration file: an address, an engine, a window size,
//! a frame budget, a receipt artifact, and a small action list that lets a bounded run
//! drive the document the way a person's hands would. Anything larger than this
//! is a product decision, and product decisions are Mere's.

use std::path::{Path, PathBuf};

/// The window ortet opens when `--size` is absent.
pub const DEFAULT_SIZE: (u32, u32) = (960, 640);

pub const USAGE: &str = "\
ortet — the raw Genet host: one window, one document, no chrome.

usage: ortet --url <address> [options]

  --url <address>      A file path, a file:// URL, or an http(s) URL.
  --engine <name>      Session engine: livery (default), boa, or nova.
  --size <WxH>         Window size in physical pixels (default 960x640).
  --frames <N>         Present exactly N frames, then exit.
  --artifact <path>    Write the captured frame as a PNG and print its digest.
  --actions <list>     Drive the document once, after its first laid-out frame.
                       Steps are separated by ';' or ',' and are one of:
                         scroll:<dx>,<dy>   scroll at the viewport centre
                         click:<x>,<y>      press and release at a point
                       e.g. --actions 'scroll:0,200' or 'click:40,120'
  --help               Print this and exit.
";

/// One driving step for a bounded run. Ortet has no scripting language and
/// wants none: these are the two gestures a receipt needs to show that scroll
/// and link activation reach the session.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    Scroll { dx: f32, dy: f32 },
    Click { x: f32, y: f32 },
}

/// The concrete session engine selected for a native run.
///
/// This is host policy. The selected engine still enters the shell through the
/// shared `SessionEngine<Scene>` contract, and the shell reports the engine's
/// own id in its outcome.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EngineChoice {
    #[default]
    Livery,
    Boa,
    Nova,
}

impl EngineChoice {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Livery => "livery",
            Self::Boa => "boa",
            Self::Nova => "nova",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    /// The address as the session engine will see it: an absolute `file://`
    /// URL for anything that named the filesystem, otherwise as given.
    pub address: String,
    pub engine: EngineChoice,
    pub size: (u32, u32),
    pub frames: Option<u32>,
    pub artifact: Option<PathBuf>,
    pub actions: Vec<Action>,
}

/// What `parse` produced: a run, or a request for the usage text.
#[derive(Clone, Debug, PartialEq)]
pub enum Invocation {
    Run(Box<Config>),
    Help,
}

/// Parse ortet's arguments, excluding `argv[0]`.
pub fn parse<I>(arguments: I) -> Result<Invocation, String>
where
    I: IntoIterator<Item = String>,
{
    let mut url = None;
    let mut engine = EngineChoice::default();
    let mut size = DEFAULT_SIZE;
    let mut frames = None;
    let mut artifact = None;
    let mut actions = Vec::new();

    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        let mut value = |flag: &str| {
            arguments
                .next()
                .ok_or_else(|| format!("{flag} needs a value"))
        };
        match argument.as_str() {
            "--help" | "-h" => return Ok(Invocation::Help),
            "--url" => url = Some(value("--url")?),
            "--engine" => engine = parse_engine(&value("--engine")?)?,
            "--size" => size = parse_size(&value("--size")?)?,
            "--frames" => {
                let raw = value("--frames")?;
                let count: u32 = raw
                    .parse()
                    .map_err(|_| format!("--frames wants a whole number, got {raw}"))?;
                if count == 0 {
                    return Err("--frames must be at least 1".to_owned());
                }
                frames = Some(count);
            },
            "--artifact" => artifact = Some(PathBuf::from(value("--artifact")?)),
            "--actions" => actions = parse_actions(&value("--actions")?)?,
            other => return Err(format!("unknown argument {other}")),
        }
    }

    let url = url.ok_or_else(|| "--url is required".to_owned())?;
    Ok(Invocation::Run(Box::new(Config {
        address: address_from_argument(&url)?,
        engine,
        size,
        frames,
        artifact,
        actions,
    })))
}

fn parse_engine(raw: &str) -> Result<EngineChoice, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "livery" => Ok(EngineChoice::Livery),
        "boa" => Ok(EngineChoice::Boa),
        "nova" => Ok(EngineChoice::Nova),
        _ => Err(format!("--engine wants livery, boa, or nova, got {raw}")),
    }
}

fn parse_size(raw: &str) -> Result<(u32, u32), String> {
    let (width, height) = raw
        .split_once(['x', 'X'])
        .ok_or_else(|| format!("--size wants WxH, got {raw}"))?;
    let width = width
        .trim()
        .parse::<u32>()
        .map_err(|_| format!("--size width is not a number: {width}"))?;
    let height = height
        .trim()
        .parse::<u32>()
        .map_err(|_| format!("--size height is not a number: {height}"))?;
    if width == 0 || height == 0 {
        return Err(format!("--size must be positive, got {raw}"));
    }
    Ok((width, height))
}

fn parse_actions(raw: &str) -> Result<Vec<Action>, String> {
    raw.split(';')
        .map(str::trim)
        .filter(|step| !step.is_empty())
        .map(parse_action)
        .collect()
}

fn parse_action(step: &str) -> Result<Action, String> {
    let (name, operands) = step
        .split_once(':')
        .ok_or_else(|| format!("action {step} wants <name>:<x>,<y>"))?;
    let (first, second) = operands
        .split_once(',')
        .ok_or_else(|| format!("action {step} wants two comma-separated numbers"))?;
    let number = |text: &str| {
        text.trim()
            .parse::<f32>()
            .map_err(|_| format!("action {step} has a non-numeric operand {text}"))
    };
    let (first, second) = (number(first)?, number(second)?);
    match name.trim() {
        "scroll" => Ok(Action::Scroll {
            dx: first,
            dy: second,
        }),
        "click" => Ok(Action::Click {
            x: first,
            y: second,
        }),
        other => Err(format!("unknown action {other}")),
    }
}

/// Turn what the user typed into an address the fetch lanes and `resolve_href`
/// both understand.
///
/// A filesystem path becomes an absolute `file://` URL rather than staying a
/// bare path, because only the URL form makes `resolve_href` join an in-page
/// `#fragment` onto the document instead of onto its directory.
pub fn address_from_argument(raw: &str) -> Result<String, String> {
    if url_scheme(raw).is_some() {
        return Ok(raw.to_owned());
    }
    let path = Path::new(raw);
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| format!("could not read the working directory: {error}"))?
            .join(path)
    };
    Ok(file_url_from_path(&absolute))
}

/// The URL scheme of `raw`, if it has one.
///
/// A scheme is at least **two** characters, which is what keeps a Windows drive
/// path (`C:\pages\a.html`) a path rather than a `c:` URL.
pub fn url_scheme(raw: &str) -> Option<&str> {
    let index = raw.find(':')?;
    let scheme = &raw[..index];
    let alphabetic_start = scheme
        .chars()
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic());
    (scheme.len() >= 2
        && alphabetic_start
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')))
    .then_some(scheme)
}

/// An absolute path as a `file://` URL. Backslashes become forward slashes so
/// a Windows path is a URL path; the drive letter keeps its colon, which is
/// what `LocalFetcher` reverses when it reads the file.
pub fn file_url_from_path(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    if text.starts_with('/') {
        format!("file://{text}")
    } else {
        format!("file:///{text}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(raw: &[&str]) -> Vec<String> {
        raw.iter().map(|text| (*text).to_owned()).collect()
    }

    fn run(raw: &[&str]) -> Config {
        match parse(args(raw)).expect("parses") {
            Invocation::Run(config) => *config,
            Invocation::Help => panic!("expected a run, got help"),
        }
    }

    #[test]
    fn a_bare_url_takes_the_default_window() {
        let config = run(&["--url", "https://example.invalid/a"]);
        assert_eq!(config.address, "https://example.invalid/a");
        assert_eq!(config.engine, EngineChoice::Livery);
        assert_eq!(config.size, DEFAULT_SIZE);
        assert_eq!(config.frames, None);
        assert_eq!(config.artifact, None);
        assert!(config.actions.is_empty());
    }

    #[test]
    fn a_receipt_run_carries_its_frames_size_and_artifact() {
        let config = run(&[
            "--url",
            "file:///x/a.html",
            "--size",
            "400x300",
            "--frames",
            "3",
            "--artifact",
            "out.png",
        ]);
        assert_eq!(config.size, (400, 300));
        assert_eq!(config.frames, Some(3));
        assert_eq!(config.artifact, Some(PathBuf::from("out.png")));
    }

    #[test]
    fn engine_selection_is_explicit_and_closed() {
        assert_eq!(
            run(&["--url", "a.html", "--engine", "boa"]).engine,
            EngineChoice::Boa
        );
        assert_eq!(
            run(&["--url", "a.html", "--engine", "NOVA"]).engine,
            EngineChoice::Nova
        );
        assert!(parse(args(&["--url", "a.html", "--engine", "other"])).is_err());
    }

    #[test]
    fn actions_parse_in_order_and_reject_nonsense() {
        let config = run(&[
            "--url",
            "file:///x/a.html",
            "--actions",
            "scroll:0,200; click:40.5,120",
        ]);
        assert_eq!(
            config.actions,
            vec![
                Action::Scroll { dx: 0.0, dy: 200.0 },
                Action::Click { x: 40.5, y: 120.0 },
            ]
        );
        assert!(parse_action("scroll:0").is_err(), "one operand is not two");
        assert!(parse_action("jump:1,2").is_err(), "unknown action name");
        assert!(parse_action("scroll:a,2").is_err(), "operands are numbers");
    }

    #[test]
    fn the_argument_surface_stays_closed() {
        assert_eq!(parse(args(&["--help"])).expect("help"), Invocation::Help);
        assert!(parse(args(&[])).is_err(), "--url is required");
        assert!(
            parse(args(&["--url", "a.html", "--profile", "reader"])).is_err(),
            "ortet has no profiles; an unknown flag must not be ignored"
        );
        assert!(parse(args(&["--url"])).is_err(), "a flag needs its value");
        assert!(
            parse(args(&["--url", "a.html", "--frames", "0"])).is_err(),
            "a zero-frame run would present nothing"
        );
        assert!(parse(args(&["--url", "a.html", "--size", "0x10"])).is_err());
        assert!(parse(args(&["--url", "a.html", "--size", "wide"])).is_err());
    }

    /// A Windows drive letter is one character, so it must not read as a URL
    /// scheme; that is the whole reason the scheme test has a length floor.
    #[test]
    fn a_drive_letter_is_not_a_url_scheme() {
        assert_eq!(url_scheme("https://example.invalid/"), Some("https"));
        assert_eq!(url_scheme("file:///x/a.html"), Some("file"));
        assert_eq!(url_scheme("data:text/html,<p>x</p>"), Some("data"));
        assert_eq!(url_scheme("C:\\pages\\a.html"), None);
        assert_eq!(url_scheme("a.html"), None);
        assert_eq!(url_scheme("#deep"), None);
        assert_eq!(
            url_scheme("2fast://x"),
            None,
            "a scheme starts with a letter"
        );
    }

    #[test]
    fn filesystem_addresses_become_absolute_file_urls() {
        assert_eq!(
            file_url_from_path(Path::new("/x/a.html")),
            "file:///x/a.html"
        );
        assert_eq!(
            file_url_from_path(Path::new("C:\\pages\\a.html")),
            "file:///C:/pages/a.html"
        );
        // A URL argument is left exactly as typed.
        assert_eq!(
            address_from_argument("file:///x/a.html").expect("passes through"),
            "file:///x/a.html"
        );
        let relative = address_from_argument("a.html").expect("resolves");
        assert!(
            relative.starts_with("file://") && relative.ends_with("/a.html"),
            "a relative path becomes an absolute file URL, got {relative}"
        );
    }
}
