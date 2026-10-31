//! The command line: `std::env::args` parsed by hand, because an
//! argument-parsing crate is not among the approved dependencies and four
//! subcommands with `--flag value` options do not need one.

use std::{collections::BTreeMap, path::PathBuf, time::Duration};

use vocab_core::providers::MyMemoryProvider;

use crate::{
    apkg::{enrich_from_apkg, ApkgFields},
    backfill::{backfill_de, BackfillOptions, Stop},
    curriculum::{build_curriculum, load_sources},
    error::{Error, Result},
    output::{read_curriculum, read_overrides, write_curriculum},
    sources::{fetch_sources, Fetcher, HttpFetcher, MirrorFetcher, DEFAULT_MANIFEST},
};

pub const USAGE: &str = "\
hsk-import: regenerates crates/vocab-core/data/hsk_curriculum.json (Milestone J2b)

usage:
  hsk-import fetch --out DIR [--manifest FILE] [--from MIRROR]
      Download the pinned upstream files into DIR and verify them against the
      SHA-256 manifest (default: crates/vocab-core/data/hsk_sources.json); the
      first run writes the manifest, a later mismatch is a hard error. With
      --from, the files are read from MIRROR (same layout as DIR) instead of
      downloaded, and verified the same way.

  hsk-import build --source DIR --out FILE [--de-overrides FILE]
      Flatten the fetched sources into curriculum JSON, merge German meanings
      from the overrides file, and print the build summary. A spot-check
      mismatch fails the build before anything is written.

  hsk-import backfill-de --curriculum FILE --overrides FILE
                         [--limit N] [--email ADDR] [--delay-ms MS]
      Machine-translate the first English sense of every item without German
      (MyMemory, en -> de) into the overrides file, in curriculum order,
      saving after every item. Stops cleanly when the daily quota is used up;
      run it again to continue. --email raises MyMemory's quota.

  hsk-import apkg-enrich --apkg FILE --curriculum FILE --media-out DIR [--out FILE]
                         [--word-field NAME] [--sentence-field NAME]
                         [--sentence-pinyin-field NAME] [--translation-field NAME]
      Fill example sentences, audio and images from a legacy Anki package
      (exported with \"Support older Anki versions\"), copying the media into
      DIR. Writes the curriculum back in place unless --out is given.
";

/// Pause between MyMemory requests unless `--delay-ms` says otherwise.
const DEFAULT_DELAY_MS: u64 = 300;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Help,
    Fetch {
        out: PathBuf,
        manifest: PathBuf,
        mirror: Option<PathBuf>,
    },
    Build {
        source: PathBuf,
        out: PathBuf,
        de_overrides: Option<PathBuf>,
    },
    BackfillDe {
        curriculum: PathBuf,
        overrides: PathBuf,
        limit: Option<usize>,
        email: Option<String>,
        delay_ms: u64,
    },
    ApkgEnrich {
        apkg: PathBuf,
        curriculum: PathBuf,
        media_out: PathBuf,
        out: Option<PathBuf>,
        fields: ApkgFields,
    },
}

/// Parses the arguments after the program name.
pub fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Command> {
    let mut args = args.into_iter();
    let Some(subcommand) = args.next() else {
        return Err(Error::Usage("missing subcommand".into()));
    };
    let allowed: &[&str] = match subcommand.as_str() {
        "help" | "--help" | "-h" => return Ok(Command::Help),
        "fetch" => &["out", "manifest", "from"],
        "build" => &["source", "out", "de-overrides"],
        "backfill-de" => &["curriculum", "overrides", "limit", "email", "delay-ms"],
        "apkg-enrich" => &[
            "apkg",
            "curriculum",
            "media-out",
            "out",
            "word-field",
            "sentence-field",
            "sentence-pinyin-field",
            "translation-field",
        ],
        other => return Err(Error::Usage(format!("unknown subcommand {other:?}"))),
    };
    let mut flags = Flags::parse(&subcommand, allowed, args)?;
    let command = match subcommand.as_str() {
        "fetch" => Command::Fetch {
            out: flags.required_path("out")?,
            manifest: flags
                .optional_path("manifest")
                .unwrap_or_else(|| PathBuf::from(DEFAULT_MANIFEST)),
            mirror: flags.optional_path("from"),
        },
        "build" => Command::Build {
            source: flags.required_path("source")?,
            out: flags.required_path("out")?,
            de_overrides: flags.optional_path("de-overrides"),
        },
        "backfill-de" => Command::BackfillDe {
            curriculum: flags.required_path("curriculum")?,
            overrides: flags.required_path("overrides")?,
            limit: flags.number("limit")?,
            email: flags.take("email"),
            delay_ms: flags.number("delay-ms")?.unwrap_or(DEFAULT_DELAY_MS),
        },
        _ => {
            let defaults = ApkgFields::default();
            Command::ApkgEnrich {
                apkg: flags.required_path("apkg")?,
                curriculum: flags.required_path("curriculum")?,
                media_out: flags.required_path("media-out")?,
                out: flags.optional_path("out"),
                fields: ApkgFields {
                    word: flags.take("word-field").unwrap_or(defaults.word),
                    sentence: flags.take("sentence-field").unwrap_or(defaults.sentence),
                    sentence_pinyin: flags
                        .take("sentence-pinyin-field")
                        .unwrap_or(defaults.sentence_pinyin),
                    sentence_translation: flags
                        .take("translation-field")
                        .unwrap_or(defaults.sentence_translation),
                },
            }
        }
    };
    Ok(command)
}

/// `--name value` / `--name=value` options of one subcommand.
struct Flags {
    subcommand: String,
    values: BTreeMap<String, String>,
}

impl Flags {
    fn parse(
        subcommand: &str,
        allowed: &[&str],
        args: impl Iterator<Item = String>,
    ) -> Result<Self> {
        let usage = |message: String| Error::Usage(format!("{subcommand}: {message}"));
        let mut values = BTreeMap::new();
        let mut args = args.peekable();
        while let Some(arg) = args.next() {
            let Some(flag) = arg.strip_prefix("--") else {
                return Err(usage(format!("unexpected argument {arg:?}")));
            };
            let (name, value) = match flag.split_once('=') {
                Some((name, value)) => (name.to_string(), value.to_string()),
                None => {
                    let value = args
                        .next_if(|next| !next.starts_with("--"))
                        .ok_or_else(|| usage(format!("--{flag} needs a value")))?;
                    (flag.to_string(), value)
                }
            };
            if !allowed.contains(&name.as_str()) {
                return Err(usage(format!("unknown option --{name}")));
            }
            if values.insert(name.clone(), value).is_some() {
                return Err(usage(format!("--{name} given twice")));
            }
        }
        Ok(Self {
            subcommand: subcommand.to_string(),
            values,
        })
    }

    fn take(&mut self, name: &str) -> Option<String> {
        self.values.remove(name)
    }

    fn optional_path(&mut self, name: &str) -> Option<PathBuf> {
        self.take(name).map(PathBuf::from)
    }

    fn required_path(&mut self, name: &str) -> Result<PathBuf> {
        self.optional_path(name)
            .ok_or_else(|| Error::Usage(format!("{}: --{name} is required", self.subcommand)))
    }

    fn number<T: std::str::FromStr>(&mut self, name: &str) -> Result<Option<T>> {
        self.take(name)
            .map(|value| {
                value.parse().map_err(|_| {
                    Error::Usage(format!(
                        "{}: --{name} must be a non-negative integer, not {value:?}",
                        self.subcommand
                    ))
                })
            })
            .transpose()
    }
}

/// Runs a parsed command, printing its summary to stdout.
pub fn run(command: Command) -> Result<()> {
    match command {
        Command::Help => print!("{USAGE}"),
        Command::Fetch {
            out,
            manifest,
            mirror,
        } => {
            let fetcher: Box<dyn Fetcher> = match mirror {
                Some(root) => Box::new(MirrorFetcher::new(root)),
                None => Box::new(HttpFetcher::new()?),
            };
            let report = fetch_sources(fetcher.as_ref(), &out, &manifest)?;
            for file in &report.files {
                println!("{}  {:>9} bytes  {}", file.sha256, file.bytes, file.path);
            }
            if report.manifest_written {
                println!("recorded the manifest in {}", manifest.display());
            } else {
                println!("every file matches {}", manifest.display());
            }
        }
        Command::Build {
            source,
            out,
            de_overrides,
        } => {
            let sources = load_sources(&source)?;
            let overrides = match &de_overrides {
                Some(path) => Some(read_overrides(path)?.ok_or_else(|| {
                    Error::Data(format!("{}: no such overrides file", path.display()))
                })?),
                None => None,
            };
            let (items, report) = build_curriculum(&sources, overrides.as_ref());
            print!("{report}");
            report.check_spot_checks()?;
            write_curriculum(&out, &items)?;
            println!("wrote {} items to {}", items.len(), out.display());
        }
        Command::BackfillDe {
            curriculum,
            overrides,
            limit,
            email,
            delay_ms,
        } => {
            let items = read_curriculum(&curriculum)?;
            let provider = MyMemoryProvider::new(email);
            let options = BackfillOptions {
                limit,
                delay: Duration::from_millis(delay_ms),
            };
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|err| Error::io("async runtime", err))?;
            let report = runtime.block_on(backfill_de(&provider, &items, &overrides, &options))?;
            print!("{report}");
            println!("overrides: {}", overrides.display());
            if let Stop::Failed(error) = report.stop {
                return Err(Error::Translation(error));
            }
        }
        Command::ApkgEnrich {
            apkg,
            curriculum,
            media_out,
            out,
            fields,
        } => {
            let mut items = read_curriculum(&curriculum)?;
            let report = enrich_from_apkg(&apkg, &mut items, &media_out, &fields)?;
            print!("{report}");
            let out = out.unwrap_or(curriculum);
            write_curriculum(&out, &items)?;
            println!(
                "wrote {} items to {}; media in {} (do not commit them)",
                items.len(),
                out.display(),
                media_out.display()
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Command> {
        parse_args(args.iter().map(|arg| arg.to_string()))
    }

    fn usage_message(args: &[&str]) -> String {
        match parse(args) {
            Err(Error::Usage(message)) => message,
            other => panic!("expected a usage error for {args:?}, got {other:?}"),
        }
    }

    #[test]
    fn parses_every_subcommand() {
        assert_eq!(
            parse(&["fetch", "--out", "src"]).unwrap(),
            Command::Fetch {
                out: "src".into(),
                manifest: DEFAULT_MANIFEST.into(),
                mirror: None,
            }
        );
        assert_eq!(
            parse(&[
                "fetch",
                "--out=src",
                "--manifest",
                "m.json",
                "--from",
                "mirror"
            ])
            .unwrap(),
            Command::Fetch {
                out: "src".into(),
                manifest: "m.json".into(),
                mirror: Some("mirror".into()),
            }
        );
        assert_eq!(
            parse(&["build", "--source", "s", "--out", "o.json"]).unwrap(),
            Command::Build {
                source: "s".into(),
                out: "o.json".into(),
                de_overrides: None
            }
        );
        assert_eq!(
            parse(&[
                "backfill-de",
                "--curriculum",
                "c.json",
                "--overrides",
                "de.json",
                "--limit",
                "120",
                "--email=me@example.com",
            ])
            .unwrap(),
            Command::BackfillDe {
                curriculum: "c.json".into(),
                overrides: "de.json".into(),
                limit: Some(120),
                email: Some("me@example.com".into()),
                delay_ms: DEFAULT_DELAY_MS,
            }
        );
        assert_eq!(
            parse(&[
                "apkg-enrich",
                "--apkg",
                "deck.apkg",
                "--curriculum",
                "c.json",
                "--media-out",
                "media",
                "--word-field",
                "Hanzi",
            ])
            .unwrap(),
            Command::ApkgEnrich {
                apkg: "deck.apkg".into(),
                curriculum: "c.json".into(),
                media_out: "media".into(),
                out: None,
                fields: ApkgFields {
                    word: "Hanzi".into(),
                    ..ApkgFields::default()
                },
            }
        );
        for help in ["help", "--help", "-h"] {
            assert_eq!(parse(&[help]).unwrap(), Command::Help);
        }
    }

    #[test]
    fn rejects_malformed_command_lines() {
        let cases: [(&[&str], &str); 9] = [
            (&[], "missing subcommand"),
            (&["frobnicate"], "unknown subcommand"),
            (&["fetch"], "--out is required"),
            (&["fetch", "--out"], "--out needs a value"),
            (
                &["fetch", "--out", "--manifest", "m"],
                "--out needs a value",
            ),
            (&["fetch", "--out", "a", "--out", "b"], "given twice"),
            (
                &["fetch", "--out", "a", "--limit", "3"],
                "unknown option --limit",
            ),
            (&["build", "--source", "s", "o.json"], "unexpected argument"),
            (
                &[
                    "backfill-de",
                    "--curriculum",
                    "c",
                    "--overrides",
                    "o",
                    "--limit",
                    "-1",
                ],
                "non-negative integer",
            ),
        ];
        for (args, needle) in cases {
            let message = usage_message(args);
            assert!(message.contains(needle), "{args:?}: {message}");
        }
    }
}
