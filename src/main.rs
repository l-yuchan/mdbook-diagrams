use clap::{Arg, ArgMatches, Command};
use mdbook::errors::Error;
use mdbook::preprocess::{CmdPreprocessor, Preprocessor};
use mdbook_diagrams::DiagramsPreprocessor;
use semver::{Version, VersionReq};
use std::io;
use std::process;

pub fn make_app() -> Command {
    Command::new("mdbook-diagrams")
        .about("A mdbook preprocessor to generate diagrams on build time")
        .subcommand(
            Command::new("supports")
                .arg(Arg::new("renderer").required(true))
                .about("Check whether a renderer is supported by this preprocessor"),
        )
}

fn main() {
    let matches = make_app().get_matches();

    if let Some(sub_args) = matches.subcommand_matches("supports") {
        let preprocessor = DiagramsPreprocessor::new(None);
        handle_supports(&preprocessor, sub_args);
    } else if let Err(e) = handle_preprocessing() {
        eprintln!("{e:?}");
        process::exit(1);
    }
}

fn handle_preprocessing() -> Result<(), Error> {
    let (ctx, book) = CmdPreprocessor::parse_input(io::stdin())?;

    let preprocessor_config = ctx.config
        .get_preprocessor("diagrams");
    let config = preprocessor_config;

    let preprocessor = DiagramsPreprocessor::new(config.cloned());

    let book_version = Version::parse(&ctx.mdbook_version)?;
    let version_req = VersionReq::parse(mdbook::MDBOOK_VERSION)?;

    if !version_req.matches(&book_version) {
        eprintln!(
            "Warning: mdbook-diagrams was built against version {} of mdbook, \
             but we\'re being called from version {}",
            mdbook::MDBOOK_VERSION,
            ctx.mdbook_version
        );
    }

    let processed_book = preprocessor.run(&ctx, book)?;
    serde_json::to_writer(io::stdout(), &processed_book)?;

    Ok(())
}

fn handle_supports(pre: &dyn Preprocessor, sub_args: &ArgMatches) -> ! {
    let renderer = sub_args
        .get_one::<String>("renderer")
        .expect("Required argument");
    let supported = pre.supports_renderer(renderer);

    if supported {
        process::exit(0);
    } else {
        process::exit(1);
    }
}